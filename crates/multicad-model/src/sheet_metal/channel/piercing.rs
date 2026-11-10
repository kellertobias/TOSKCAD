//! Pair each canonical sheet opening with its two folded wall origins and its
//! developed contour. The kernel certifies containment and separation.

use super::*;
use crate::sheet_forming::shape_edges;
use multicad_kernel::SweepOpening;
use std::f64::consts::TAU;

pub(super) fn apply(
    layout: &SheetLayout,
    solid: Solid,
    mut developed: PlanarContours,
    policy: GeometryPolicy,
) -> Result<(Solid, PlanarContours), String> {
    if layout.cutouts.is_empty() {
        return Ok((solid, developed));
    }
    let mut openings = Vec::with_capacity(layout.cutouts.len());
    for cutout in &layout.cutouts {
        let name = cutout.source.id();
        let (frame, normal) = layout.space().face_frame(cutout.face, &layout.flanges)?;
        let roles = match cutout.face {
            CutoutFace::Base => [WEB_A.into(), WEB_B.into()],
            CutoutFace::Flange { index } => [
                format!("sheet.flange.{index}.leg.a"),
                format!("sheet.flange.{index}.leg.b"),
            ],
        };
        let walls = roles.map(|role| Origin {
            source: layout.forming_feature,
            role,
        });
        let outline = shape_edges(&cutout.shape);
        let origins = (0..outline.len())
            .map(|i| Origin {
                source: layout.forming_feature,
                role: format!("sheet.{name}.edge.{i}"),
            })
            .collect();
        openings.push(SweepOpening {
            walls,
            frame,
            through_mm: scale(normal, layout.thickness_mm),
            outline,
            origins,
        });

        let flat = flat_cutout(
            cutout,
            &layout.flanges,
            layout.thickness_mm,
            layout.k_factor,
        )
        .ok_or_else(|| format!("{name} names a missing flange"))?;
        developed.loops.push(developed_hole(flat));
    }
    let pierced = solid
        .rust_pierce_normal_profile(&openings, policy)
        .map_err(|e| e.to_string())?;
    Ok((pierced, developed))
}

fn developed_hole(outline: Vec<CurvedEdge2d>) -> PlanarLoop {
    let mut loop_ = PlanarLoop {
        outer: false,
        edges: outline
            .into_iter()
            .map(|edge| match edge {
                CurvedEdge2d::Line { a, b } => PlanarEdge::Line { a, b },
                CurvedEdge2d::Circle { center, radius_mm } => {
                    let a = [center[0] + radius_mm, center[1]];
                    PlanarEdge::Arc {
                        a,
                        b: a,
                        center,
                        sweep_rad: -TAU,
                    }
                }
                _ => unreachable!("A dimensional cutout is rectangular or circular"),
            })
            .collect(),
    };
    loop_.normalize_winding();
    loop_
}
