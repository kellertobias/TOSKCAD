//! Exact trimmed surfaces for a stock body with drilled cuts.
use super::*;
use multicad_core::references::{AnalyticGeometry, TopologyKind};
use multicad_rust_kernel::{
    planar_projection::{AnalyticFace, FaceTrim, SpatialRimCandidate},
    trim_curve::{Ellipse, EllipseArc},
    warped_union::Elevation,
};
use std::{collections::BTreeMap, f64::consts::FRAC_PI_2};

type Trims = BTreeMap<usize, Vec<Vec<FaceTrim>>>;

pub(in crate::drawing::curved) fn analytic_stock(
    solid: &Solid,
    source: usize,
    options: DrawingProjection,
) -> Result<Option<Elevation>, GeometryError> {
    let Some((stock, cuts)) = solid.drawing_circular_cuts() else {
        return oblique_hex(solid, source, options);
    };
    let screen = Screen::new(options.frame);
    if options.hidden_lines
        && !cuts
            .iter()
            .any(|cut| is_oblique(screen.vector(cut.outward)))
    {
        return Ok(None);
    }
    let tolerance = solid.geometry_policy().linear_tolerance_mm;
    let mut result = Elevation::default();
    let mut holes = BTreeMap::new();
    for cut in cuts {
        append_cut(
            Cut {
                faces: cut.faces,
                centers: cut.centers.map(|point| screen.point(point)),
                outward: screen.vector(cut.outward),
                radius: cut.radius,
            },
            source,
            tolerance,
            &mut holes,
            &mut result,
        )?;
    }
    append_stock(
        &stock,
        &screen,
        source,
        tolerance,
        options.tangent_edges,
        holes,
        &mut result,
    )?;
    Ok(Some(result))
}

fn is_oblique([x, y, z]: Point) -> bool {
    x.hypot(y) > ANGULAR && z.abs() > ANGULAR
}

fn oblique_hex(
    solid: &Solid,
    source: usize,
    options: DrawingProjection,
) -> Result<Option<Elevation>, GeometryError> {
    let Some((body, cut)) = hex_geometry(solid, options) else {
        return Ok(None);
    };
    if !is_oblique(cut.outward) {
        return Ok(None);
    }
    let tolerance = solid.geometry_policy().linear_tolerance_mm;
    let mut result = Elevation::default();
    let mut holes = BTreeMap::new();
    append_cut(cut, source, tolerance, &mut holes, &mut result)?;
    for (index, face) in body.faces.into_iter().enumerate() {
        let outer = face
            .edges
            .into_iter()
            .enumerate()
            .map(|(i, edge)| {
                let mut points = body.edges[edge];
                // Each side follows its lower rim, the upward edge, then
                // traverses the upper rim and the other upright in reverse.
                if index >= 2 && i >= 2 {
                    points.reverse();
                }
                FaceTrim::Line(points)
            })
            .collect();
        let mut rings = vec![outer];
        if let Some(cut_rings) = holes.remove(&index) {
            rings.extend(cut_rings);
        }
        result.blockers.extend(
            AnalyticFace::plane(face.point, face.normal, rings, tolerance)
                .map_err(planar::projection_error)?,
        );
    }
    result.lines.extend(
        body.edges
            .into_iter()
            .map(|points| LineCandidate { source, points }),
    );
    Ok(Some(result))
}

fn unsupported_stock() -> GeometryError {
    err(
        GeometryErrorKind::Unsupported,
        "Rust circular stock projection requires closed straight-edged stock faces",
    )
}

fn append_stock(
    stock: &Solid,
    screen: &Screen,
    source: usize,
    tolerance: f64,
    tangent_edges: bool,
    mut holes: Trims,
    result: &mut Elevation,
) -> Result<(), GeometryError> {
    let snapshot = stock.rust_topology().ok_or_else(unsupported_stock)??;
    let mut edges: BTreeMap<_, _> = snapshot
        .elements
        .iter()
        .filter_map(|element| match element.geometry {
            Some(AnalyticGeometry::Line { a_mm, b_mm })
                if element.key.kind == TopologyKind::Edge =>
            {
                Some((
                    element.key.ordinal,
                    ([a_mm, b_mm].map(|point| screen.point(point)), [None; 2]),
                ))
            }
            _ => None,
        })
        .collect();
    for face in stock.faces()? {
        let (origin, normal) = face.plane.ok_or_else(unsupported_stock)?;
        let normal = screen.vector(normal);
        let mut rings = Vec::new();
        for ring in crate::rust_display::planar_loops(stock, face.index) {
            let mut trims = Vec::with_capacity(ring.len());
            for (edge, reversed) in ring {
                let (mut points, supports) = edges
                    .get_mut(&edge)
                    .map(|(points, supports)| (*points, supports))
                    .ok_or_else(unsupported_stock)?;
                let available = supports
                    .iter_mut()
                    .find(|support| support.is_none())
                    .ok_or_else(unsupported_stock)?;
                *available = Some(normal);
                if reversed {
                    points.reverse();
                }
                trims.push(FaceTrim::Line(points));
            }
            rings.push(trims);
        }
        if let Some(cut_rings) = holes.remove(&(face.index as usize)) {
            rings.extend(cut_rings);
        }
        result.blockers.extend(
            AnalyticFace::plane(screen.point(origin), normal, rings, tolerance)
                .map_err(planar::projection_error)?,
        );
    }
    if !holes.is_empty() {
        return Err(unsupported_stock());
    }
    for (points, supports) in edges.into_values() {
        let [Some(a), Some(b)] = supports else {
            return Err(unsupported_stock());
        };
        if tangent_edges || dot(a, b) < 1. - ANGULAR {
            result.lines.push(LineCandidate { source, points });
        }
    }
    Ok(())
}

fn append_cut(
    cut: Cut,
    source: usize,
    tolerance: f64,
    holes: &mut Trims,
    result: &mut Elevation,
) -> Result<(), GeometryError> {
    let axis = cut.outward.map(|value| -value);
    let [x, y, z] = axis;
    let projected = x.hypot(y);
    let u = if projected > ANGULAR {
        [-y / projected, x / projected, 0.]
    } else {
        [1., 0., 0.]
    };
    let height = dot(
        std::array::from_fn(|i| cut.centers[1][i] - cut.centers[0][i]),
        axis,
    );
    let quarters = |center, reverse| {
        let ellipse =
            Ellipse::new(center, axis, u, [cut.radius; 2]).map_err(|_| unsupported_stock())?;
        let sign = if reverse { -1. } else { 1. };
        let arcs = (0..4)
            .map(|i| {
                EllipseArc::new(ellipse, i as f64 * sign * FRAC_PI_2, sign * FRAC_PI_2)
                    .map_err(|_| unsupported_stock())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok::<_, GeometryError>((ellipse, arcs))
    };
    for (end, center) in cut.centers.into_iter().enumerate() {
        let (ellipse, arcs) = quarters(center, end == 1)?;
        if end == 0 {
            for &arc in &arcs {
                result.blockers.extend(
                    AnalyticFace::cylinder(arc, axis, height, false, tolerance)
                        .map_err(planar::projection_error)?,
                );
            }
        }
        let ring = arcs.into_iter().map(FaceTrim::Arc).collect();
        if let Some(face) = cut.faces[end] {
            holes.entry(face).or_default().push(ring);
        } else {
            result.blockers.extend(
                AnalyticFace::plane(center, cut.outward, vec![ring], tolerance)
                    .map_err(planar::projection_error)?,
            );
        }

        let v = ellipse.minor_axis();
        let minor = if z.abs() > ANGULAR {
            z.abs().min(1.)
        } else {
            0.
        };
        let screen_u = [u[0], u[1]];
        let screen_v = if minor > ANGULAR {
            [v[0] / minor, v[1] / minor]
        } else {
            [-u[1], u[0]]
        };
        result.rims.push(SpatialRimCandidate {
            source,
            ellipse: curved_projection::Ellipse {
                center: [center[0], center[1]],
                u: screen_u,
                v: screen_v,
                major: cut.radius,
                minor: cut.radius * minor,
                trim: [0., TAU],
            },
            depth: center[2],
            depth_sine: v[2] * cut.radius,
        });
    }
    if projected > ANGULAR {
        for side in [-1., 1.] {
            result.lines.push(LineCandidate {
                source,
                points: cut.centers.map(|center| {
                    std::array::from_fn(|i| u[i].mul_add(side * cut.radius, center[i]))
                }),
            });
        }
    }
    Ok(())
}
