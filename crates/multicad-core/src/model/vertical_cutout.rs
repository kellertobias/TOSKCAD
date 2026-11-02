//! Vertical cutouts that choose what they cut: one wall, floor slabs, or
//! both. A shaft or floor penetration is one cutout identity shared by the
//! wall's `VerticalCutout` record (if it cuts a wall) and a `SlabHole` in
//! every slab it cuts. Placing, editing or removing one rewrites every
//! affected wall and slab feature in a single transaction, through the same
//! wall planner and slab resolver as their own edits, so the window, the
//! Room tool and MCP share one canonical operation. A refused change stages
//! nothing.
use super::{
    CreatedBody, ModelPrepared,
    common::{error, number, revision},
    level::{
        SlabDefinition, SlabHole, SlabKind, polygons_meet, read_slab, rectangle_corners,
        resolve_level, resolve_slab, slabs,
    },
    source::invalid,
    wall::{VerticalCutout, WallDefinition, read_wall, resolve_wall, walls},
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, DocumentError, EntityData, EntityId, Mutation},
    parameters::{EvaluatedParameters, Unit},
};

/// A plan rectangle for slab holes: center in world X/Y, width along the
/// direction `rotation_deg` from +X, depth across it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRectangle {
    pub center: [Dimension; 2],
    pub width: Dimension,
    pub depth: Dimension,
    pub rotation_deg: f64,
}
/// What one vertical cutout cuts and where.
#[derive(Clone, Debug, PartialEq)]
pub struct CutoutPlacement {
    /// The cutout identity, shared by the wall record and every slab hole.
    pub id: EntityId,
    /// The wall cut, if any: the wall feature and its full-height cutout
    /// record, whose identity must equal `id`.
    pub wall: Option<(EntityId, VerticalCutout)>,
    /// The rectangle cut from the slabs. Without it, the wall cutout's own
    /// plan footprint is used.
    pub plan: Option<PlanRectangle>,
    /// The slab or pedestal features to cut (see `slabs_in_levels`).
    pub slabs: Vec<EntityId>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum CutoutChange {
    /// Place a new cutout, or rewrite the one with the same identity so it
    /// cuts exactly the given wall and slabs.
    Place(Box<CutoutPlacement>),
    /// Remove the cutout from its wall and from every slab.
    Remove(EntityId),
}
impl CutoutChange {
    pub fn place(placement: CutoutPlacement) -> Self {
        Self::Place(Box::new(placement))
    }
    pub fn target(&self) -> EntityId {
        match self {
            Self::Place(p) => p.id,
            Self::Remove(id) => *id,
        }
    }
}

/// Where a cutout identity currently lives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CutoutMembers {
    pub wall: Option<(EntityId, VerticalCutout)>,
    pub slabs: Vec<(EntityId, SlabHole)>,
}
impl CutoutMembers {
    pub fn is_empty(&self) -> bool {
        self.wall.is_none() && self.slabs.is_empty()
    }
}
pub fn cutout_members(document: &Document, id: EntityId) -> CutoutMembers {
    let wall = walls(document).find_map(|e| {
        let w = read_wall(document, e.id).ok()?;
        w.cutouts
            .iter()
            .find(|c| c.id == id)
            .map(|c| (e.id, c.clone()))
    });
    let slabs = slabs(document)
        .filter_map(|e| {
            let s = read_slab(document, e.id).ok()?;
            s.holes
                .iter()
                .find(|h| h.id == id)
                .map(|h| (e.id, h.clone()))
        })
        .collect();
    CutoutMembers { wall, slabs }
}
/// Every cutout identity that cuts at least one slab, in document order.
pub fn slab_cutouts(document: &Document) -> Vec<EntityId> {
    let mut out: Vec<EntityId> = vec![];
    for e in slabs(document) {
        if let Ok(s) = read_slab(document, e.id) {
            for h in s.holes {
                if !out.contains(&h.id) {
                    out.push(h.id);
                }
            }
        }
    }
    out
}

/// A resolved plan rectangle in millimeters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanRectangleMm {
    pub center: [f64; 2],
    pub width: f64,
    pub depth: f64,
    pub rotation_deg: f64,
}
impl PlanRectangleMm {
    pub fn corners(&self) -> [[f64; 2]; 4] {
        rectangle_corners(self.center, self.width, self.depth, self.rotation_deg)
    }
    /// The rectangle as literal source dimensions.
    pub fn source(&self) -> PlanRectangle {
        let l = |v: f64| super::length((v * 1e6).round() / 1e6);
        PlanRectangle {
            center: [l(self.center[0]), l(self.center[1])],
            width: l(self.width),
            depth: l(self.depth),
            rotation_deg: (self.rotation_deg * 1e9).round() / 1e9,
        }
    }
}
fn resolve_rectangle(
    rect: &PlanRectangle,
    parameters: &EvaluatedParameters,
) -> Result<PlanRectangleMm, DocumentError> {
    let mm = |d: &Dimension| number(d, Unit::Millimeter, parameters);
    if !rect.rotation_deg.is_finite() {
        return Err(invalid("Cutout rotation must be finite"));
    }
    Ok(PlanRectangleMm {
        center: [mm(&rect.center[0])?, mm(&rect.center[1])?],
        width: mm(&rect.width)?,
        depth: mm(&rect.depth)?,
        rotation_deg: rect.rotation_deg,
    })
}

/// The wall after placing `cutout` (replacing one with the same identity),
/// checked by the wall planner, and the cutout's plan footprint.
fn wall_with_cutout(
    document: &Document,
    parameters: &EvaluatedParameters,
    feature: EntityId,
    cutout: &VerticalCutout,
) -> Result<(WallDefinition, PlanRectangleMm), CommandError> {
    let mut wall = read_wall(document, feature)?;
    if wall.openings.iter().any(|o| o.id == cutout.id) {
        return Err(error(
            "A door or window of this wall already uses that identity",
        ));
    }
    match wall.cutouts.iter_mut().find(|c| c.id == cutout.id) {
        Some(slot) => *slot = cutout.clone(),
        None => wall.cutouts.push(cutout.clone()),
    }
    wall.validate()?;
    let resolved = resolve_wall(document, parameters, &wall)?;
    let plan = resolved.plan()?;
    resolved.check_openings(&plan)?;
    let c = resolved
        .cutouts
        .iter()
        .find(|c| c.id == cutout.id)
        .ok_or_else(|| error("The planned cutout is missing"))?;
    let seg = plan.segments[c.segment];
    let ([a, b], [low, high]) = resolved.cutout_span(&plan, c);
    let (along, across) = ((a + b) / 2., (low + high) / 2.);
    Ok((
        wall,
        PlanRectangleMm {
            center: [
                seg.start[0] + seg.direction[0] * along + seg.normal[0] * across,
                seg.start[1] + seg.direction[1] * along + seg.normal[1] * across,
            ],
            width: b - a,
            depth: high - low,
            rotation_deg: seg.direction[1].atan2(seg.direction[0]).to_degrees(),
        },
    ))
}

/// The slab footprint of a placement: its plan rectangle, or the wall
/// cutout's footprint. Used by the UI and MCP to find slabs by level.
pub fn cutout_footprint(
    document: &Document,
    parameters: &EvaluatedParameters,
    placement: &CutoutPlacement,
) -> Result<PlanRectangleMm, CommandError> {
    match (&placement.plan, &placement.wall) {
        (Some(rect), _) => Ok(resolve_rectangle(rect, parameters)?),
        (None, Some((feature, cutout))) => {
            Ok(wall_with_cutout(document, parameters, *feature, cutout)?.1)
        }
        (None, None) => Err(error("Give the cutout a plan rectangle or a wall to cut")),
    }
}

/// Floor slabs (not pedestals) on levels from `from` to `to` (inclusive, by
/// elevation; `to` defaults to `from`) that the footprint touches. A slab
/// the footprint only partly covers is included so placing the cutout
/// reports that it crosses the slab edge.
pub fn slabs_in_levels(
    document: &Document,
    parameters: &EvaluatedParameters,
    from: EntityId,
    to: Option<EntityId>,
    footprint: &[[f64; 2]; 4],
) -> Result<Vec<EntityId>, CommandError> {
    let a = resolve_level(document, parameters, from)?.elevation_mm;
    let b = match to {
        Some(t) => resolve_level(document, parameters, t)?.elevation_mm,
        None => a,
    };
    let (lo, hi) = (a.min(b) - 1e-6, a.max(b) + 1e-6);
    let mut found: Vec<(f64, EntityId)> = vec![];
    for e in slabs(document) {
        let Ok(slab) = read_slab(document, e.id) else {
            continue;
        };
        if slab.kind != SlabKind::Slab {
            continue;
        }
        let Ok(level) = resolve_level(document, parameters, slab.level) else {
            continue;
        };
        if !(lo..=hi).contains(&level.elevation_mm) {
            continue;
        }
        let Ok(resolved) = resolve_slab(document, parameters, &slab) else {
            continue;
        };
        if polygons_meet(&resolved.outline, footprint) {
            found.push((level.elevation_mm, e.id));
        }
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    Ok(found.into_iter().map(|(_, id)| id).collect())
}

/// The checked result of a change, before anything is staged.
#[derive(Clone, Debug)]
pub struct PlannedCutout {
    /// Replacement wall and slab features, wall first.
    pub edits: Vec<Mutation>,
    /// Changed features and their output Parts, wall first.
    pub outputs: Vec<CreatedBody>,
    /// The wall this cutout now cuts.
    pub wall: Option<EntityId>,
    /// The slabs this cutout now cuts.
    pub slabs: Vec<EntityId>,
    /// The slab footprint, when it cuts slabs.
    pub footprint: Option<PlanRectangleMm>,
    pub label: &'static str,
}

fn name(document: &Document, id: EntityId) -> String {
    document
        .get(id)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| id.to_string())
}
fn replace_feature(
    document: &Document,
    feature: EntityId,
    definition: crate::document::Definition,
    inputs: Vec<crate::document::Link>,
) -> Result<Mutation, CommandError> {
    let mut entity = document
        .get(feature)
        .ok_or_else(|| error("Feature is missing"))?
        .clone();
    let EntityData::Feature { suppressed, .. } = entity.data else {
        return Err(error("Choose a feature"));
    };
    entity.data = EntityData::Feature {
        definition,
        inputs,
        suppressed,
    };
    Ok(Mutation::Replace(entity))
}
fn slab_output(document: &Document, feature: EntityId) -> Result<CreatedBody, CommandError> {
    Ok(CreatedBody {
        feature,
        body: read_slab(document, feature)?.output.body,
    })
}

/// Check one change against the document with the same rules as the
/// transaction. Nothing is staged.
pub fn plan_vertical_cutout(
    document: &Document,
    change: &CutoutChange,
) -> Result<PlannedCutout, CommandError> {
    let parameters = crate::parameters::evaluate(document)?;
    let id = change.target();
    if id.is_nil() {
        return Err(error("A vertical cutout needs a non-nil identity"));
    }
    let current = cutout_members(document, id);
    let mut edits = vec![];
    let mut outputs = vec![];
    let (placement, label) = match change {
        CutoutChange::Remove(_) => {
            if current.is_empty() {
                return Err(error(
                    "No wall or slab has a vertical cutout with that identity",
                ));
            }
            (None, "Remove vertical cutout")
        }
        CutoutChange::Place(p) => {
            if p.wall.is_none() && p.slabs.is_empty() {
                return Err(error(
                    "Choose what the cutout cuts: a wall, one or more floor slabs, or both",
                ));
            }
            if let Some((_, c)) = &p.wall
                && c.id != p.id
            {
                return Err(error("The wall cutout must carry the cutout identity"));
            }
            for (k, s) in p.slabs.iter().enumerate() {
                if p.slabs[..k].contains(s) {
                    return Err(error("Each slab can be cut only once by a cutout"));
                }
                read_slab(document, *s).map_err(|e| {
                    error(format!(
                        "{} is not a floor slab or pedestal: {e}",
                        name(document, *s)
                    ))
                })?;
            }
            let label = if current.is_empty() {
                "Place vertical cutout"
            } else {
                "Edit vertical cutout"
            };
            (Some(&**p), label)
        }
    };

    // Walls: drop the cutout from a wall it leaves, place it in its wall.
    let mut footprint = None;
    let target_wall = placement.and_then(|p| p.wall.as_ref());
    if let Some((feature, _)) = &current.wall
        && target_wall.is_none_or(|(w, _)| w != feature)
    {
        let mut wall = read_wall(document, *feature)?;
        wall.cutouts.retain(|c| c.id != id);
        wall.validate()?;
        let resolved = resolve_wall(document, &parameters, &wall)?;
        resolved.check_openings(&resolved.plan()?)?;
        edits.push(replace_feature(
            document,
            *feature,
            wall.definition()?,
            wall.inputs(),
        )?);
        outputs.push(CreatedBody {
            feature: *feature,
            body: wall.output.body,
        });
    }
    if let Some((feature, cutout)) = target_wall {
        let (wall, rect) = wall_with_cutout(document, &parameters, *feature, cutout)
            .map_err(|e| error(format!("{}: {e}", name(document, *feature))))?;
        footprint = Some(rect);
        edits.insert(
            0,
            replace_feature(document, *feature, wall.definition()?, wall.inputs())?,
        );
        outputs.insert(
            0,
            CreatedBody {
                feature: *feature,
                body: wall.output.body,
            },
        );
    }

    // Slabs: drop the hole where it no longer cuts, write it where it does.
    let targets: &[EntityId] = placement.map_or(&[], |p| p.slabs.as_slice());
    let hole = match placement {
        Some(p) if !p.slabs.is_empty() => {
            let rect = match &p.plan {
                Some(r) => {
                    footprint = Some(resolve_rectangle(r, &parameters)?);
                    r.clone()
                }
                None => footprint
                    .ok_or_else(|| error("Give the cutout a plan rectangle or a wall to cut"))?
                    .source(),
            };
            Some(SlabHole {
                id,
                center: rect.center,
                width: rect.width,
                depth: rect.depth,
                rotation_deg: rect.rotation_deg,
            })
        }
        _ => None,
    };
    let mut affected: Vec<EntityId> = current.slabs.iter().map(|(s, _)| *s).collect();
    for s in targets {
        if !affected.contains(s) {
            affected.push(*s);
        }
    }
    for slab_id in affected {
        let mut slab: SlabDefinition = read_slab(document, slab_id)?;
        let position = slab.holes.iter().position(|h| h.id == id);
        match (&hole, targets.contains(&slab_id), position) {
            (Some(h), true, Some(i)) => slab.holes[i] = h.clone(),
            (Some(h), true, None) => slab.holes.push(h.clone()),
            (_, _, Some(i)) => {
                slab.holes.remove(i);
            }
            _ => continue,
        }
        slab.validate()?;
        resolve_slab(document, &parameters, &slab)
            .map_err(|e| error(format!("{}: {e}", name(document, slab_id))))?;
        edits.push(replace_feature(
            document,
            slab_id,
            slab.definition()?,
            slab.inputs(),
        )?);
        outputs.push(slab_output(document, slab_id)?);
    }
    if edits.is_empty() {
        return Err(error("The cutout change edits nothing"));
    }
    Ok(PlannedCutout {
        edits,
        outputs,
        wall: target_wall.map(|(w, _)| *w),
        slabs: targets.to_vec(),
        footprint: hole.is_some().then_some(footprint).flatten(),
        label,
    })
}

/// Stage one cutout change as one transaction on every wall and slab it
/// touches. A native evaluator still builds the solids.
pub fn prepare_vertical_cutout(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    change: &CutoutChange,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let planned = plan_vertical_cutout(session.document(), change)?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: planned.label.into(),
        edits: planned.edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: planned.outputs,
    })
}

#[cfg(test)]
mod tests {
    use super::super::level::{convex_overlap, inside_outline};
    use super::*;

    #[test]
    fn hole_rectangles_must_sit_inside_concave_outlines_and_apart() {
        // An L-shaped slab: the notch at the top right is not slab.
        let l = [
            [0., 0.],
            [4000., 0.],
            [4000., 2000.],
            [2000., 2000.],
            [2000., 4000.],
            [0., 4000.],
        ];
        let inside = rectangle_corners([1000., 1000.], 600., 600., 0.);
        assert!(inside_outline(&l, &inside));
        // Rotated 45 degrees it still fits.
        assert!(inside_outline(
            &l,
            &rectangle_corners([1000., 1000.], 600., 600., 45.)
        ));
        // Across the inner corner of the L, all four corners lie in the slab
        // but the notch vertex lies inside the rectangle.
        let across = rectangle_corners([2000., 2000.], 900., 900., 45.);
        assert!(!inside_outline(&l, &across));
        // In the notch, or touching the outer edge.
        assert!(!inside_outline(
            &l,
            &rectangle_corners([3000., 3000.], 400., 400., 0.)
        ));
        assert!(!inside_outline(
            &l,
            &rectangle_corners([300., 1000.], 600., 600., 0.)
        ));
        // Apart, touching and overlapping holes.
        let a = rectangle_corners([1000., 1000.], 600., 600., 0.);
        assert!(!convex_overlap(
            &a,
            &rectangle_corners([1700., 1000.], 600., 600., 0.)
        ));
        assert!(convex_overlap(
            &a,
            &rectangle_corners([1600., 1000.], 600., 600., 0.)
        ));
        assert!(convex_overlap(
            &a,
            &rectangle_corners([1200., 1200.], 200., 200., 30.)
        ));
    }
}
