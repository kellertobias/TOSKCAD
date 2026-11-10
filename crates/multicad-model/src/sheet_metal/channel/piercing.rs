//! Pair each canonical sheet opening with its two folded wall origins and its
//! developed contour. The kernel certifies containment and separation.

use super::*;
use crate::sheet_forming::shape_edges;
use crate::sheet_metal::profile_cut::ProfileCut;
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
        let outline = shape_edges(&cutout.shape);
        let origins = (0..outline.len())
            .map(|i| Origin {
                source: layout.forming_feature,
                role: format!("sheet.{name}.edge.{i}"),
            })
            .collect();
        openings.push(opening(layout, cutout.face, outline, origins)?);

        let flat = flat_cutout(
            cutout,
            &layout.flanges,
            layout.thickness_mm,
            layout.k_factor,
        )
        .ok_or_else(|| format!("{name} names a missing flange"))?;
        developed.loops.push(developed_hole(&flat)?);
    }
    let pierced = solid
        .rust_pierce_normal_profile(&openings, policy)
        .map_err(|e| e.to_string())?;
    Ok((pierced, developed))
}

/// Append an authored sketch opening to the certified holes of the previous
/// folded solid and extend its verified development only on success.
pub(in crate::sheet_metal) fn append_profile(
    layout: &SheetLayout,
    sheet: &Solid,
    folded_outline: Vec<CurvedEdge2d>,
    cut: &ProfileCut,
    policy: GeometryPolicy,
) -> Result<(Solid, PlanarContours), String> {
    let mut developed = layout
        .developed
        .clone()
        .ok_or("The folded sheet has no verified development")?;
    developed.loops.push(developed_hole(&cut.outline)?);
    developed.validate(policy.linear_tolerance_mm)?;
    let opening = opening(layout, cut.face, folded_outline, cut.origins.clone())?;
    let pierced = sheet
        .rust_pierce_normal_profile(&[opening], policy)
        .map_err(|e| e.to_string())?;
    Ok((pierced, developed))
}

fn opening(
    layout: &SheetLayout,
    face: CutoutFace,
    outline: Vec<CurvedEdge2d>,
    origins: Vec<Origin>,
) -> Result<SweepOpening, String> {
    let (frame, normal) = layout.space().face_frame(face, &layout.flanges)?;
    let roles = match face {
        CutoutFace::Base => [WEB_A.into(), WEB_B.into()],
        CutoutFace::Flange { index } => [
            format!("sheet.flange.{index}.leg.a"),
            format!("sheet.flange.{index}.leg.b"),
        ],
    };
    Ok(SweepOpening {
        walls: roles.map(|role| Origin {
            source: layout.forming_feature,
            role,
        }),
        frame,
        through_mm: scale(normal, layout.thickness_mm),
        outline,
        origins,
    })
}

fn developed_hole(outline: &[CurvedEdge2d]) -> Result<PlanarLoop, String> {
    let mut loop_ = PlanarLoop {
        outer: false,
        edges: outline
            .iter()
            .map(|edge| {
                Ok(match *edge {
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
                    _ => {
                        return Err(
                            "Rust folded sheet requires a rectangular or circular opening".into(),
                        );
                    }
                })
            })
            .collect::<Result<_, String>>()?,
    };
    loop_.normalize_winding();
    Ok(loop_)
}
