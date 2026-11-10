//! Five-face analytic STEP boundary of a bounded helical tube cut.
use super::*;
use crate::helical_tube_cut::{BoundaryEdge, Cut, CylinderTrim};
use helical_trim::{CavityCapPcurve, CavitySurface, Rail, Spline, Surface, TopCavity, UvSpline};
use std::f64::consts::{PI, TAU};

mod curves;

pub fn write_helical_tube_cut(
    cut: Cut,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_helical_tube_cut(cut, origin, u, v, tolerance_mm))
}

/// Cheap writer preflight for the bounded stock chart and world precision.
/// Constructing the actual spline nets applies the remaining work budget.
pub fn supports_helical_tube_cut(cut: Cut, origin: Point, tolerance_mm: f64) -> bool {
    Chart::new(cut, origin, tolerance_mm).is_ok()
}

struct Chart {
    lift: f64,
    trims: [CylinderTrim; 3],
    rails: [Rail; 3],
    cavity: CavitySurface,
    top: TopCavity,
    cap: CavityCapPcurve,
}

impl Chart {
    fn new(cut: Cut, origin: Point, tolerance: f64) -> Result<Self, StepError> {
        let spec = cut.spec();
        let radius = spec.stock_radius_mm;
        let height = spec.stock_height_mm;
        let rise = spec.pitch_mm / TAU;
        let raw_seam = spec.start_angle_rad + PI - spec.start_height_mm / rise;
        let seam0 = raw_seam.rem_euclid(TAU);
        let lift = seam0 - raw_seam;
        let seam1 = seam0 + height / rise;
        let wall_clearance = tolerance / radius;
        let [negative, positive] = cut.cylinder_trims().map(|trim| CylinderTrim {
            azimuth_rad: trim.azimuth_rad.map(|u| u + lift),
            height_mm: trim.height_mm,
        });
        let [top_negative, top_positive] = [negative, positive].map(|trim| trim.azimuth_rad[1]);
        if seam1 - seam0 > 256. * TAU
            || !(seam1 - TAU + wall_clearance < top_negative
                && top_negative < top_positive
                && top_positive < seam1 - wall_clearance)
            || origin.iter().any(|&x| {
                !x.is_finite()
                    || x.abs() + radius + height > 1e7
                    || 128. * f64::EPSILON * (1. + x.abs() + radius + height) > tolerance * 0.125
            })
        {
            return Err(StepError::Limit);
        }
        let trims = [
            CylinderTrim {
                azimuth_rad: [seam0, seam1],
                height_mm: [0., height],
            },
            positive,
            negative,
        ];
        let [seam, positive, negative] = trims.map(|trim| Rail::new(radius, trim, tolerance));
        Ok(Self {
            lift,
            trims,
            rails: [seam?, positive?, negative?],
            cavity: CavitySurface::new(cut, tolerance)?,
            top: TopCavity::new(cut, tolerance)?,
            cap: CavityCapPcurve::new(cut, tolerance)?,
        })
    }
}

pub(super) fn encode(
    file: &mut StepFile,
    cut: Cut,
    placement: &Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    let chart = Chart::new(cut, placement.origin, tolerance)?;
    let spec = cut.spec();
    let radius = spec.stock_radius_mm;
    let height = spec.stock_height_mm;
    let rise = spec.pitch_mm / TAU;
    let [seam_trim, positive, negative] = chart.trims;
    let [seam0, seam1] = seam_trim.azimuth_rad;
    let [top_positive, top_negative] = [positive, negative].map(|trim| trim.azimuth_rad[1]);
    // Prepare and certify each finite spline before serializing any poles.
    let tensor = chart.cavity.surface()?;
    let surface_bound = chart.cavity.certify(&tensor)?;
    let top_curve = chart.top.spline()?;
    let top_bound = chart.top.certify(&top_curve)?;
    let top_uv = chart.cap.spline()?;
    let cap_bound = chart.cap.certify(&top_uv)?;
    check_error_budget(surface_bound + top_bound + cap_bound, tolerance)?;
    let domains = chart.cavity.domains();
    let [phi_positive, phi_negative] = domains[0];
    let [seam, rail_pos, rail_neg] = chart.rails.map(prepared_rail);
    let [
        (seam, seam_bound),
        (rail_pos, pos_bound),
        (rail_neg, neg_bound),
    ] = [seam?, rail_pos?, rail_neg?];
    let wall_bound = seam_bound.max(surface_bound + pos_bound.max(neg_bound));
    check_error_budget(wall_bound, tolerance)?;
    let floor_uv = curves::floor_cylinder_uv(cut, chart.lift, tolerance)?;

    let world = |[x, y, z]: Point| placement.point_f64([x, y], z);
    let vertex = |file: &mut StepFile, point: Point| file.vertex(world(point));
    let [bottom_vertex, seam_top] =
        [seam.poles_mm[0], *seam.poles_mm.last().unwrap()].map(|point| vertex(file, point));
    let [positive_floor, negative_floor] = [phi_positive, phi_negative]
        .map(|angle| vertex(file, cut.edge_point(BoundaryEdge::FloorRim, angle)));
    let [positive_top, negative_top] = [phi_positive, phi_negative]
        .map(|angle| vertex(file, cut.edge_point(BoundaryEdge::TopCavity, angle)));

    let stock_bottom = file.point(placement.origin);
    let stock_top = file.point(world([0., 0., height]));
    let cylinder_axis = file.axis(stock_bottom, placement.normal, placement.u);
    let cylinder = file.entity(format_args!(
        "CYLINDRICAL_SURFACE('',{cylinder_axis},{radius:.16E})"
    ));
    let top_axis = file.axis(stock_top, placement.normal, placement.u);
    let top_plane = file.entity(format_args!("PLANE('',{top_axis})"));
    let [floor_origin, radial, transverse] = {
        let (origin, radial, transverse) = cut.floor_frame();
        [
            world(origin),
            placement.vector(radial),
            placement.vector(transverse),
        ]
    };
    let floor_normal = cross(radial, transverse);
    let floor_center = file.point(floor_origin);
    let floor_axis = file.axis(floor_center, floor_normal, radial);
    let floor_plane = file.entity(format_args!("PLANE('',{floor_axis})"));
    let cavity_support = curves::surface(file, &tensor, world);
    let uv = curves::UvContext::new(file);

    let stock = curves::StockChart {
        uv: &uv,
        cylinder,
        placement,
        radius,
    };
    let bottom = stock.arc(
        file,
        [bottom_vertex; 2],
        stock_bottom,
        [seam0 - TAU, seam0],
        0.,
    );
    let top_lo = stock.arc(
        file,
        [positive_top, seam_top],
        stock_top,
        [top_positive, seam1],
        height,
    );
    let top_hi = stock.arc(
        file,
        [seam_top, negative_top],
        stock_top,
        [seam1 - TAU, top_negative],
        height,
    );

    let encode_spline = |file: &mut StepFile, spline: &Spline, vertices| {
        curves::spline(file, spline, vertices, placement, tolerance * 0.125)
    };
    let seam_curve = encode_spline(file, &seam, [bottom_vertex, seam_top])?;
    let seam_uv = [0., -TAU].map(|offset| {
        uv.line(
            file,
            cylinder,
            seam_trim.azimuth_rad,
            [[seam0 + offset, 0.], [seam1 + offset, height]],
        )
    });
    let seam = curves::edge(
        file,
        [bottom_vertex, seam_top],
        seam_curve,
        &seam_uv,
        curves::Master::Seam,
    );
    let [rail_pos, rail_neg] = [
        (
            positive,
            phi_positive,
            rail_pos,
            [positive_floor, positive_top],
        ),
        (
            negative,
            phi_negative,
            rail_neg,
            [negative_floor, negative_top],
        ),
    ]
    .map(|(trim, phi, spline, vertices)| {
        let chart = uv.line(
            file,
            cylinder,
            trim.azimuth_rad,
            [
                [trim.azimuth_rad[0], trim.height_mm[0]],
                [trim.azimuth_rad[1], trim.height_mm[1]],
            ],
        );
        let cavity = uv.line(
            file,
            cavity_support,
            trim.azimuth_rad,
            [[phi, 0.], [phi, trim.azimuth_rad[1] - trim.azimuth_rad[0]]],
        );
        let curve = encode_spline(file, &spline, vertices)?;
        Ok::<_, StepError>(curves::edge(
            file,
            vertices,
            curve,
            &[chart, cavity],
            curves::Master::Pcurve,
        ))
    });
    let [rail_pos, rail_neg] = [rail_pos?, rail_neg?];
    let floor_rim_curve = curves::circle(file, floor_axis, spec.tube_radius_mm);
    let floor_rim_uv = uv.line(
        file,
        cavity_support,
        domains[0],
        [[phi_positive, 0.], [phi_negative, 0.]],
    );
    let floor_rim = curves::edge(
        file,
        [positive_floor, negative_floor],
        floor_rim_curve,
        &[floor_rim_uv],
        curves::Master::Curve,
    );
    let floor_ellipse_center = file.point(world([0., 0., spec.start_height_mm]));
    let ellipse_axis = file.axis(floor_ellipse_center, floor_normal, transverse);
    let semi_major = radius * radius.hypot(rise) / rise;
    let floor_curve = file.entity(format_args!(
        "ELLIPSE('',{ellipse_axis},{semi_major:.16E},{radius:.16E})"
    ));
    let floor_cylinder_uv = uv.spline(file, cylinder, &floor_uv);
    let floor_cylinder = curves::edge(
        file,
        [negative_floor, positive_floor],
        floor_curve,
        &[floor_cylinder_uv],
        curves::Master::Curve,
    );
    let top_pcurves = [
        uv.spline(file, cavity_support, &top_uv),
        uv.spline(
            file,
            top_plane,
            &UvSpline {
                degree: top_curve.degree,
                knots: top_curve.knots.clone(),
                multiplicities: top_curve.multiplicities.clone(),
                poles_uv: top_curve.poles_mm.iter().map(|p| [p[0], p[1]]).collect(),
            },
        ),
    ];
    let top_cavity_curve = encode_spline(file, &top_curve, [positive_top, negative_top])?;
    let top_cavity = curves::edge(
        file,
        [positive_top, negative_top],
        top_cavity_curve,
        &top_pcurves,
        curves::Master::Curve,
    );

    let bottom_bound = bound(file, [(bottom, false)]);
    let bottom_face = file.face(
        &[bottom_bound],
        stock_bottom,
        placement.normal.map(|x| -x),
        placement.u,
    );
    let top_bound = bound(file, [(top_lo, true), (top_hi, true), (top_cavity, false)]);
    let top = file.face_on(&[top_bound], top_plane, true);
    let cylinder_bound = bound(
        file,
        [
            (bottom, true),
            (seam, true),
            (top_lo, false),
            (rail_pos, false),
            (floor_cylinder, false),
            (rail_neg, true),
            (top_hi, false),
            (seam, false),
        ],
    );
    let wall = file.face_on(&[cylinder_bound], cylinder, true);
    let cavity_bound = bound(
        file,
        [
            (rail_pos, true),
            (top_cavity, true),
            (rail_neg, false),
            (floor_rim, false),
        ],
    );
    let cavity = file.face_on(&[cavity_bound], cavity_support, false);
    let floor_bound = bound(file, [(floor_rim, true), (floor_cylinder, true)]);
    let floor = file.face_on(&[floor_bound], floor_plane, true);
    Ok(file.solid(&[bottom_face, top, wall, cavity, floor]))
}

fn bound<const N: usize>(file: &mut StepFile, edges: [(Entity, bool); N]) -> Entity {
    let edges = edges.map(|(edge, forward)| file.oriented_edge(edge, forward));
    file.bound(&edges, true)
}

fn prepared_rail(rail: Rail) -> Result<(Spline, f64), StepError> {
    let spline = rail.spline()?;
    let bound = rail.certify(rail.pcurve(), &spline)?;
    Ok((spline, bound))
}

fn check_error_budget(error: f64, tolerance: f64) -> Result<(), StepError> {
    // Reserve one quarter of the budget for world placement and replacement
    // of spline endpoints by their shared topological vertices.
    if error > tolerance * 0.75 {
        return Err(StepError::Limit);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
