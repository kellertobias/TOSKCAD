//! A rectangular sheet with one full-length bend or two opposite bends has
//! one continuous cross-section. Sweep it without faces at the roots.

use super::*;
use crate::sheet_forming::section;
use multicad_core::references::ReferenceFrame;

mod piercing;
pub(super) use piercing::{append_profile, developed_hole};

pub(super) const WEB_A: &str = "sheet.web/cap.start";
pub(super) const WEB_B: &str = "sheet.web/cap.end";
const UNSUPPORTED: &str = "Rust formed sheet requires one full-length bend or two opposite full-length bends without reliefs";

pub(super) fn build(
    layout: &mut SheetLayout,
    section_width_mm: f64,
    policy: GeometryPolicy,
) -> Result<Solid, String> {
    let (first, second) = match layout.flanges.as_slice() {
        [first] => (first, None),
        [first, second] if first.flange.edge.abs_diff(second.flange.edge) == 2 => {
            (first, Some(second))
        }
        _ => return Err(UNSUPPORTED.into()),
    };
    if layout.flanges.iter().any(|flange| {
        let f = &flange.flange;
        f.start_inset_mm != 0.
            || f.end_inset_mm != 0.
            || f.relief_mm.is_some()
            || !matches!(
                f.segments.as_slice(),
                [StripSegment::Bend { .. }, StripSegment::Straight { .. }]
            )
    }) {
        return Err(UNSUPPORTED.into());
    }
    let [u, v] = first.outward.map(|c| -c);
    let across = [u, v];
    let frame = PlaneFrame {
        origin: layout.frame.point(first.start),
        u: std::array::from_fn(|i| u * layout.frame.u[i] + v * layout.frame.v[i]),
        v: layout.thickness_direction,
    };
    let [du, dv] = first.along;
    let along = std::array::from_fn(|i| du * layout.frame.u[i] + dv * layout.frame.v[i]);
    let height = first.width.copysign(dot(frame.normal(), along));
    let t = layout.thickness_mm;
    let left = section(&first.flange, t);
    let mut boundary = Vec::new();
    // Replace each flange's temporary root edge with the web boundary.
    let left_count = left.edges.len() - 1;
    boundary.extend(
        left.edges
            .into_iter()
            .zip(left.roles)
            .take(left_count)
            .map(|(edge, role)| (section_edge(edge, 0., true), role)),
    );
    boundary.push((
        CurvedEdge2d::Line {
            a: [0., t],
            b: [section_width_mm, t],
        },
        WEB_B.into(),
    ));
    if let Some(second) = second {
        let right = section(&second.flange, t);
        boundary.extend(
            right
                .edges
                .into_iter()
                .zip(right.roles)
                .rev()
                .skip(1)
                .map(|(edge, role)| (section_edge(edge, section_width_mm, false), role)),
        );
    } else {
        let opposite = (first.flange.edge + 2) % 4;
        boundary.push((
            CurvedEdge2d::Line {
                a: [section_width_mm, t],
                b: [section_width_mm, 0.],
            },
            format!("sheet.base.edge.{opposite}"),
        ));
    }
    boundary.push((
        CurvedEdge2d::Line {
            a: [section_width_mm, 0.],
            b: [0., 0.],
        },
        WEB_A.into(),
    ));
    let (outer, origins): (Vec<_>, Vec<_>) = boundary
        .into_iter()
        .map(|(edge, role)| {
            (
                edge,
                Origin {
                    source: layout.forming_feature,
                    role,
                },
            )
        })
        .unzip();
    let profile = CurvedProfile {
        frame,
        outer,
        holes: vec![],
    };
    let solid = Solid::rust_extrude_normal_profile_named(
        &profile,
        height,
        layout.forming_feature,
        &origins,
        policy,
    )
    .map_err(|e| e.to_string())?;

    let developed_left = first.flange.developed_mm(t, layout.k_factor);
    let developed_right = second.map_or(0., |f| f.flange.developed_mm(t, layout.k_factor));
    let developed_corners = [
        [0., -developed_left],
        [first.width, -developed_left],
        [first.width, section_width_mm + developed_right],
        [0., section_width_mm + developed_right],
    ]
    .map(|[s, a]| std::array::from_fn(|i| first.start[i] + s * first.along[i] + a * across[i]));
    let mut outer = PlanarLoop {
        outer: true,
        edges: (0..4)
            .map(|i| PlanarEdge::Line {
                a: developed_corners[i],
                b: developed_corners[(i + 1) % 4],
            })
            .collect(),
    };
    outer.normalize_winding();
    let contours = PlanarContours {
        frame: ReferenceFrame {
            origin_mm: layout.frame.origin,
            u: layout.frame.u,
            v: layout.frame.v,
        },
        loops: vec![outer],
    };
    let (solid, contours) = piercing::apply(layout, solid, contours, policy)?;
    contours.validate(policy.linear_tolerance_mm)?;
    layout.developed = Some(contours);
    Ok(solid)
}

fn section_edge(edge: CurvedEdge2d, offset: f64, left: bool) -> CurvedEdge2d {
    let sign = if left { -1. } else { 1. };
    let point = |[x, y]: [f64; 2]| [offset + sign * x, y];
    // Reflect the left section into the channel frame and walk the right
    // section in reverse. Both operations invert circular traversal.
    let ends = |a, b| {
        if left {
            [point(a), point(b)]
        } else {
            [point(b), point(a)]
        }
    };
    match edge {
        CurvedEdge2d::Line { a, b } => {
            let [a, b] = ends(a, b);
            CurvedEdge2d::Line { a, b }
        }
        CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        } => {
            let [a, b] = ends(a, b);
            CurvedEdge2d::Arc {
                center: point(center),
                a,
                b,
                clockwise: !clockwise,
            }
        }
        _ => unreachable!("A flange section contains only lines and circular arcs"),
    }
}
