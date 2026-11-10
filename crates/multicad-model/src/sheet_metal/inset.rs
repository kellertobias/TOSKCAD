//! Inset flanges change the section along their span. Join their exact
//! root boundaries to one relieved web rather than fusing display solids.
use super::*;
use crate::sheet_forming::section;
use multicad_core::references::ReferenceFrame;
use multicad_kernel::rust_kernel::{
    normal_sweep::{Edge, signed_area},
    warped_union::{AxialCut, SheetFlange, SheetWebEdge, formed_sheet},
};

pub(super) fn build(layout: &mut SheetLayout, policy: GeometryPolicy) -> Result<Solid, String> {
    let thickness = layout.thickness_mm;
    let count = layout.flanges.len();
    if count > 2
        || (count == 2
            && layout.flanges[0]
                .flange
                .edge
                .abs_diff(layout.flanges[1].flange.edge)
                != 2)
        || layout.flanges.iter().any(|f| {
            !matches!(
                f.flange.segments.as_slice(),
                [StripSegment::Bend { .. }, StripSegment::Straight { .. }]
            )
        })
    {
        return Err("Rust inset formed sheet requires one bend or two opposite bends".into());
    }
    let mut origins = Vec::new();
    let mut source = |role| {
        origins.push(Origin {
            source: layout.forming_feature,
            role,
        });
        origins.len() - 1
    };
    let mut outer = Vec::new();
    let mut developed = Vec::new();
    for (index, edge) in layout.base.outer.iter().enumerate() {
        let CurvedEdge2d::Line { a, b } = *edge else {
            return Err("Inset formed sheet needs straight web edges".into());
        };
        let Some((hinge, f)) = layout
            .flanges
            .iter()
            .enumerate()
            .find(|(_, f)| f.flange.edge == index)
        else {
            outer.push(SheetWebEdge {
                ends: [a, b],
                source: source(format!("sheet.base.edge.{index}")),
                flange: None,
            });
            developed.push(PlanarEdge::Line { a, b });
            continue;
        };
        let points = edge_points(f, a, b, 0.)?;
        let root = [point(f, 0., 0.), point(f, f.width, 0.)];
        for (i, pair) in points.windows(2).enumerate() {
            outer.push(SheetWebEdge {
                ends: [pair[0], pair[1]],
                source: source(format!("sheet.base.edge.{index}.segment.{i}")),
                flange: (pair == root).then_some(hinge),
            });
        }
        let extension = f.flange.developed_mm(thickness, layout.k_factor);
        developed.extend(edge_points(f, a, b, extension)?.windows(2).map(|pair| {
            PlanarEdge::Line {
                a: pair[0],
                b: pair[1],
            }
        }));
    }
    let mut loops = vec![PlanarLoop {
        outer: true,
        edges: developed,
    }];
    let mut holes = Vec::new();
    let mut flange_cuts = Vec::new();
    let mut flange_area = 0.;
    for cut in &layout.cutouts {
        let name = cut.source.id();
        let flat = flat_cutout(cut, &layout.flanges, thickness, layout.k_factor)
            .ok_or_else(|| format!("{name} names a missing flange"))?;
        loops.push(channel::developed_hole(&flat)?);
        if cut.face != CutoutFace::Base {
            let model::ResolvedShape::Circle {
                center,
                diameter_mm,
            } = cut.shape
            else {
                return Err("Rust inset formed sheet requires circular flange openings".into());
            };
            let (frame, normal) = layout.space().face_frame(cut.face, &layout.flanges)?;
            let center = frame.point(center);
            let overshoot = 8. * policy.linear_tolerance_mm;
            let local = |level| {
                let offset =
                    std::array::from_fn(|i| center[i] + normal[i] * level - layout.frame.origin[i]);
                [layout.frame.u, layout.frame.v, layout.frame.normal()]
                    .map(|axis| dot(offset, axis))
            };
            let [start_mm, end_mm] = [-overshoot, thickness + overshoot].map(local);
            let origin = source(format!("sheet.{name}.edge.0"));
            flange_cuts.push(AxialCut {
                start_mm,
                end_mm,
                radius_mm: diameter_mm / 2.,
                source: 0,
                wall_source_face: origin,
                cap_source_faces: [origin; 2],
                recess: None,
            });
            flange_area += cut.shape.area_mm2();
            continue;
        }
        let outline = crate::sheet_forming::shape_edges(&cut.shape);
        let mut ring = outline
            .iter()
            .enumerate()
            .map(|(i, edge)| {
                Ok((
                    profile_edge(edge)?,
                    source(format!("sheet.{name}.edge.{i}")),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let curves = ring.iter().map(|&(edge, _)| edge).collect::<Vec<_>>();
        if signed_area(&curves) > 0. {
            ring.reverse();
            ring.iter_mut()
                .for_each(|(edge, _)| *edge = edge.reversed());
        }
        holes.push(ring);
    }
    let sections = layout
        .flanges
        .iter()
        .map(|f| {
            let profile = section(&f.flange, thickness);
            let edges = profile
                .edges
                .iter()
                .zip(profile.roles)
                .map(|(edge, role)| Ok((profile_edge(edge)?, source(role))))
                .collect::<Result<Vec<_>, String>>()?;
            let caps = ["start", "end"]
                .map(|end| source(format!("sheet.flange.{}.cap.{end}", f.flange.index)));
            Ok((edges, caps))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let flanges = sections
        .iter()
        .map(|(section, ends)| SheetFlange {
            section,
            ends: *ends,
        })
        .collect::<Vec<_>>();
    let shell = formed_sheet(
        &outer,
        &holes,
        [channel::WEB_A, channel::WEB_B].map(|role| source(role.into())),
        &flanges,
        thickness.copysign(dot(layout.frame.normal(), layout.thickness_direction)),
        policy.linear_tolerance_mm,
        policy.minimum_feature_mm,
    )
    .map_err(|cause| format!("The inset formed sheet boundary cannot be certified: {cause:?}"))?;
    let shell = if flange_cuts.is_empty() {
        shell
    } else {
        let drilled = shell
            .cut_axial(&flange_cuts, policy.linear_tolerance_mm)
            .map_err(|cause| format!("The inset flange openings cannot be certified: {cause:?}"))?;
        let expected = flange_area * thickness;
        let removed = shell.volume_mm3() - drilled.volume_mm3();
        if (expected - removed).abs() > 1e-7 * expected.max(1.) {
            return Err(
                "An inset flange opening does not go through the full sheet thickness".into(),
            );
        }
        drilled
    };
    let solid = Solid::rust_place_formed_sheet(
        shell,
        layout.frame,
        layout.forming_feature,
        &origins,
        policy,
    )
    .map_err(|cause| cause.to_string())?;
    let contours = PlanarContours {
        frame: ReferenceFrame {
            origin_mm: layout.frame.origin,
            u: layout.frame.u,
            v: layout.frame.v,
        },
        loops,
    };
    contours.validate(policy.linear_tolerance_mm)?;
    layout.developed = Some(contours);
    Ok(solid)
}

fn profile_edge(edge: &CurvedEdge2d) -> Result<Edge, String> {
    match *edge {
        CurvedEdge2d::Line { a, b } => Ok(Edge::Line { a, b }),
        CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        } => Ok(Edge::Arc {
            center,
            a,
            b,
            clockwise,
        }),
        CurvedEdge2d::Circle { center, radius_mm } => Ok(Edge::Circle {
            center,
            radius_mm,
            clockwise: true,
        }),
        CurvedEdge2d::BSpline(_) => {
            Err("Inset formed sheet requires straight or circular boundaries".into())
        }
    }
}

fn point(f: &PlacedFlange, along: f64, out: f64) -> [f64; 2] {
    std::array::from_fn(|i| f.start[i] + f.along[i] * along + f.outward[i] * out)
}

/// One web edge and its reliefs, with optional outward flat extension.
pub(super) fn edge_points(
    f: &PlacedFlange,
    a: [f64; 2],
    b: [f64; 2],
    extension: f64,
) -> Result<Vec<[f64; 2]>, String> {
    let at = |along, out| point(f, along, out);
    let mut points = vec![a];
    let relief = f.flange.relief_mm;
    if f.flange.start_inset_mm > 1e-9 {
        if let Some([w, depth]) = relief {
            if w > f.flange.start_inset_mm + 1e-9 {
                return Err(format!(
                    "Flange {} relief is wider than its start inset",
                    f.flange.index + 1
                ));
            }
            points.extend([at(-w, 0.), at(-w, -depth), at(0., -depth)]);
        }
        points.push(at(0., 0.));
    }
    points.extend([at(0., extension), at(f.width, extension)]);
    if f.flange.end_inset_mm > 1e-9 {
        points.push(at(f.width, 0.));
        if let Some([w, depth]) = relief {
            if w > f.flange.end_inset_mm + 1e-9 {
                return Err(format!(
                    "Flange {} relief is wider than its end inset",
                    f.flange.index + 1
                ));
            }
            points.extend([
                at(f.width, -depth),
                at(f.width + w, -depth),
                at(f.width + w, 0.),
            ]);
        }
    }
    points.push(b);
    points.dedup_by(|p, q| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-9);
    Ok(points)
}
