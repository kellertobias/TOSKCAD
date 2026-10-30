//! Placing, editing and removing one door, window or vertical cutout in an
//! existing wall. Each change rewrites only that wall's opening list and goes
//! through the same planner and transaction as a full wall edit, so the
//! window, the Room tools and MCP share one canonical operation. A refused
//! change stages nothing.
use super::{
    ModelPrepared,
    common::error,
    source::invalid,
    wall::{
        NewType, NewTypeKind, Opening, OpeningKind, ResolvedWall, VerticalCutout, WallChange,
        WallDefinition, WallPlan, prepare_wall_change_labeled, read_opening_type, read_wall,
        resolve_wall,
    },
};
use crate::{
    commands::{Actor, CommandError, Session},
    document::{Document, DocumentError, EntityId, Mutation},
};

/// One change to a wall's openings.
#[derive(Clone, Debug, PartialEq)]
pub enum WallOpeningChange {
    /// Add a door or window; its identity must be new in the wall.
    AddOpening(Opening),
    /// Add a vertical cutout; its identity must be new in the wall.
    AddCutout(VerticalCutout),
    /// Replace the door or window with the same identity.
    ReplaceOpening(Opening),
    /// Replace the vertical cutout with the same identity.
    ReplaceCutout(VerticalCutout),
    /// Remove the opening or cutout with this identity.
    Remove(EntityId),
}
impl WallOpeningChange {
    /// The wall source after this change, without planning it.
    pub fn apply(&self, wall: &WallDefinition) -> Result<WallDefinition, DocumentError> {
        let mut next = wall.clone();
        let fresh = |id: EntityId| {
            if id.is_nil() || wall.has_opening(id) {
                Err(invalid(
                    "The new opening needs an identity not used in this wall",
                ))
            } else {
                Ok(())
            }
        };
        match self {
            Self::AddOpening(o) => {
                fresh(o.id)?;
                next.openings.push(o.clone());
            }
            Self::AddCutout(c) => {
                fresh(c.id)?;
                next.cutouts.push(c.clone());
            }
            Self::ReplaceOpening(o) => {
                let slot = next
                    .openings
                    .iter_mut()
                    .find(|x| x.id == o.id)
                    .ok_or_else(|| invalid("This wall has no door or window with that identity"))?;
                *slot = o.clone();
            }
            Self::ReplaceCutout(c) => {
                let slot = next
                    .cutouts
                    .iter_mut()
                    .find(|x| x.id == c.id)
                    .ok_or_else(|| {
                        invalid("This wall has no vertical cutout with that identity")
                    })?;
                *slot = c.clone();
            }
            Self::Remove(id) => {
                let before = next.openings.len() + next.cutouts.len();
                next.openings.retain(|o| o.id != *id);
                next.cutouts.retain(|c| c.id != *id);
                if next.openings.len() + next.cutouts.len() == before {
                    return Err(invalid("This wall has no opening with that identity"));
                }
            }
        }
        Ok(next)
    }
    /// The identity of the opening or cutout this change places or edits.
    pub fn target(&self) -> EntityId {
        match self {
            Self::AddOpening(o) | Self::ReplaceOpening(o) => o.id,
            Self::AddCutout(c) | Self::ReplaceCutout(c) => c.id,
            Self::Remove(id) => *id,
        }
    }
    fn label(&self, document: &Document, new_types: &[NewType]) -> &'static str {
        let door = |o: &Opening| {
            let kind = new_types
                .iter()
                .find(|t| t.id == o.opening_type)
                .and_then(|t| match &t.kind {
                    NewTypeKind::Opening(v) => Some(v.kind.clone()),
                    NewTypeKind::Wall(_) => None,
                })
                .or_else(|| {
                    read_opening_type(document, o.opening_type)
                        .ok()
                        .map(|t| t.kind)
                });
            matches!(kind, Some(OpeningKind::Door { .. }))
        };
        match self {
            Self::AddOpening(o) if door(o) => "Place door",
            Self::AddOpening(_) => "Place window",
            Self::ReplaceOpening(o) if door(o) => "Edit door",
            Self::ReplaceOpening(_) => "Edit window",
            Self::AddCutout(_) => "Place vertical cutout",
            Self::ReplaceCutout(_) => "Edit vertical cutout",
            Self::Remove(_) => "Remove wall opening",
        }
    }
}

/// The checked result of a change: the new wall source, its resolved values
/// and plan. Used for live diagnostics before anything is staged.
#[derive(Clone, Debug)]
pub struct PlannedWallOpening {
    pub wall: WallDefinition,
    pub resolved: ResolvedWall,
    pub plan: WallPlan,
}

/// Plan one opening change against the document plus any new types, with
/// the same checks as the transaction. Nothing is staged.
pub fn plan_wall_opening(
    document: &Document,
    feature: EntityId,
    new_types: &[NewType],
    change: &WallOpeningChange,
) -> Result<PlannedWallOpening, CommandError> {
    let mut inserts = vec![];
    for t in new_types {
        if document.get(t.id).is_some() {
            return Err(error("New type identities must be new"));
        }
        inserts.push(Mutation::Insert(t.entity()?));
    }
    let candidate = document.edited(&inserts)?;
    let wall = change.apply(&read_wall(&candidate, feature)?)?;
    wall.validate()?;
    let parameters = crate::parameters::evaluate(&candidate)?;
    let resolved = resolve_wall(&candidate, &parameters, &wall)?;
    let plan = resolved.plan()?;
    resolved.check_openings(&plan)?;
    Ok(PlannedWallOpening {
        wall,
        resolved,
        plan,
    })
}

/// Stage one opening change, plus any new types it uses, as one transaction
/// on the wall feature. A native evaluator still builds the solid.
pub fn prepare_wall_opening(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    new_types: &[NewType],
    change: &WallOpeningChange,
) -> Result<ModelPrepared, CommandError> {
    let wall = change.apply(&read_wall(session.document(), feature)?)?;
    prepare_wall_change_labeled(
        session,
        expected_revision,
        actor,
        new_types,
        &WallChange::Edit {
            feature,
            wall: &wall,
        },
        Some(change.label(session.document(), new_types)),
    )
}

/// Where a plan point lies on a wall: the segment, the distance along it from
/// its start and the signed distance across it (left positive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallLocation {
    pub segment: usize,
    pub along_mm: f64,
    pub across_mm: f64,
}
/// The wall segment under a plan point, if the point lies within the wall
/// band widened by `tolerance_mm`. The closest segment wins at joints.
pub fn locate_on_wall(plan: &WallPlan, point: [f64; 2], tolerance_mm: f64) -> Option<WallLocation> {
    let (low, high) = (
        plan.right_mm.min(plan.left_mm),
        plan.right_mm.max(plan.left_mm),
    );
    let mut best: Option<(f64, WallLocation)> = None;
    for (index, seg) in plan.segments.iter().enumerate() {
        let d = [point[0] - seg.start[0], point[1] - seg.start[1]];
        let along = d[0] * seg.direction[0] + d[1] * seg.direction[1];
        let across = d[0] * seg.normal[0] + d[1] * seg.normal[1];
        let outside_along = (-along).max(along - seg.length_mm).max(0.);
        let outside_across = (low - across).max(across - high).max(0.);
        let miss = outside_along.hypot(outside_across);
        if miss > tolerance_mm {
            continue;
        }
        // Inside the band, prefer the segment whose centerline is nearest.
        let score = miss * 1e6 + (across - (low + high) / 2.).abs();
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((
                score,
                WallLocation {
                    segment: index,
                    along_mm: along.clamp(0., seg.length_mm),
                    across_mm: across,
                },
            ));
        }
    }
    best.map(|(_, location)| location)
}

#[cfg(test)]
mod tests {
    use super::super::{
        BodyOutput, Hinge, Justification, NewType, NewTypeKind, OpeningTypeDefinition, Swing,
        WallFace, WallRequest, WallTypeDefinition, length, prepare_wall_change, walls,
    };
    use super::*;
    use crate::commands::Channel;

    struct Fixture {
        session: Session,
        wall: EntityId,
        door: EntityId,
        window: EntityId,
    }
    fn actor() -> Actor {
        Actor::local(Channel::Ui)
    }
    /// An open 6 m wall (one segment) or a closed 5 × 4 m room.
    fn fixture(closed: bool) -> Fixture {
        let mut session = Session::new(Document::new("Openings").unwrap()).unwrap();
        let ty = NewType::new(
            "Block 200",
            NewTypeKind::Wall(WallTypeDefinition {
                thickness: length(200.),
                height: length(2500.),
                justification: Justification::Center,
                material: None,
            }),
        );
        let opening = |kind, w, h, s| {
            NewType::new(
                "Opening",
                NewTypeKind::Opening(OpeningTypeDefinition {
                    kind,
                    width: length(w),
                    height: length(h),
                    sill: length(s),
                }),
            )
        };
        let door = opening(
            OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Left,
            },
            900.,
            2100.,
            0.,
        );
        let window = opening(OpeningKind::Window, 1200., 1200., 900.);
        let path = if closed {
            vec![[0., 0.], [5000., 0.], [5000., 4000.], [0., 4000.]]
        } else {
            vec![[0., 0.], [6000., 0.]]
        };
        let request = WallRequest::new(
            "Wall",
            WallDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                wall_type: ty.id,
                path: path.iter().map(|[x, y]| [length(*x), length(*y)]).collect(),
                closed,
                level: None,
                base_z: length(0.),
                thickness: None,
                height: None,
                justification: None,
                openings: vec![],
                cutouts: vec![],
            },
        );
        let prepared = prepare_wall_change(
            &session,
            0,
            actor(),
            &[ty, door.clone(), window.clone()],
            &WallChange::Create(&request),
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        let wall = walls(session.document()).next().unwrap().id;
        Fixture {
            wall,
            session,
            door: door.id,
            window: window.id,
        }
    }
    fn opening(ty: EntityId, segment: usize, offset: f64) -> Opening {
        Opening {
            id: EntityId::new(),
            opening_type: ty,
            segment,
            offset: length(offset),
            width: None,
            height: None,
            sill: None,
        }
    }
    fn cutout(segment: usize, offset: f64, width: f64, depth: Option<f64>) -> VerticalCutout {
        VerticalCutout {
            id: EntityId::new(),
            segment,
            offset: length(offset),
            width: length(width),
            depth: depth.map(length),
            face: WallFace::Left,
        }
    }
    fn refused(f: &Fixture, change: WallOpeningChange) -> String {
        let before = f.session.document().clone();
        let message = prepare_wall_opening(&f.session, 1, actor(), f.wall, &[], &change)
            .unwrap_err()
            .to_string();
        assert_eq!(f.session.document(), &before);
        message
    }

    #[test]
    fn placing_a_door_is_one_undoable_transaction_and_edits_keep_its_identity() {
        let mut f = fixture(false);
        let before = f.session.document().clone();
        let door = opening(f.door, 0, 1000.);
        let prepared = prepare_wall_opening(
            &f.session,
            1,
            actor(),
            f.wall,
            &[],
            &WallOpeningChange::AddOpening(door.clone()),
        )
        .unwrap();
        assert_eq!(prepared.outputs[0].feature, f.wall);
        f.session.commit(prepared.prepared).unwrap();
        assert_eq!(f.session.revision(), 2);
        let wall = read_wall(f.session.document(), f.wall).unwrap();
        assert_eq!(wall.openings, vec![door.clone()]);

        let mut moved = door.clone();
        moved.offset = length(3000.);
        moved.width = Some(length(1000.));
        let prepared = prepare_wall_opening(
            &f.session,
            2,
            actor(),
            f.wall,
            &[],
            &WallOpeningChange::ReplaceOpening(moved.clone()),
        )
        .unwrap();
        f.session.commit(prepared.prepared).unwrap();
        let wall = read_wall(f.session.document(), f.wall).unwrap();
        assert_eq!(wall.openings, vec![moved]);

        let prepared = prepare_wall_opening(
            &f.session,
            3,
            actor(),
            f.wall,
            &[],
            &WallOpeningChange::Remove(door.id),
        )
        .unwrap();
        f.session.commit(prepared.prepared).unwrap();
        assert!(
            read_wall(f.session.document(), f.wall)
                .unwrap()
                .openings
                .is_empty()
        );
        for revision in 4..7 {
            f.session.undo(revision, actor()).unwrap();
        }
        assert_eq!(f.session.document(), &before);
    }

    #[test]
    fn misfit_overlap_and_stale_identities_are_refused_without_staging() {
        let mut f = fixture(false);
        let door = opening(f.door, 0, 1000.);
        let prepared = prepare_wall_opening(
            &f.session,
            1,
            actor(),
            f.wall,
            &[],
            &WallOpeningChange::AddOpening(door.clone()),
        )
        .unwrap();
        f.session.commit(prepared.prepared).unwrap();
        let refuse = |change| {
            let before = f.session.document().clone();
            let message = prepare_wall_opening(&f.session, 2, actor(), f.wall, &[], &change)
                .unwrap_err()
                .to_string();
            assert_eq!(f.session.document(), &before);
            message
        };
        assert!(
            refuse(WallOpeningChange::AddOpening(opening(f.window, 0, 5500.)))
                .contains("does not fit")
        );
        assert!(
            refuse(WallOpeningChange::AddOpening(opening(f.window, 0, 1500.))).contains("overlaps")
        );
        assert!(
            refuse(WallOpeningChange::AddOpening(opening(f.window, 1, 0.))).contains("segment 2")
        );
        assert!(refuse(WallOpeningChange::AddOpening(door.clone())).contains("identity"));
        assert!(refuse(WallOpeningChange::Remove(EntityId::new())).contains("identity"));
        assert!(
            refuse(WallOpeningChange::ReplaceCutout(cutout(0, 0., 100., None)))
                .contains("vertical cutout")
        );
        let mut tall = opening(f.window, 0, 3000.);
        tall.height = Some(length(2000.));
        assert!(refuse(WallOpeningChange::AddOpening(tall)).contains("wall top"));
        // A stale revision is refused too.
        assert!(
            prepare_wall_opening(
                &f.session,
                1,
                actor(),
                f.wall,
                &[],
                &WallOpeningChange::AddOpening(opening(f.window, 0, 3000.))
            )
            .is_err()
        );
    }

    #[test]
    fn a_new_opening_type_commits_with_the_placement() {
        let f = fixture(false);
        let ty = NewType::new(
            "Door 1000",
            NewTypeKind::Opening(OpeningTypeDefinition {
                kind: OpeningKind::Door {
                    hinge: Hinge::End,
                    swing: Swing::Right,
                },
                width: length(1000.),
                height: length(2100.),
                sill: length(0.),
            }),
        );
        let change = WallOpeningChange::AddOpening(opening(ty.id, 0, 2000.));
        let planned = plan_wall_opening(
            f.session.document(),
            f.wall,
            std::slice::from_ref(&ty),
            &change,
        )
        .unwrap();
        assert_eq!(planned.resolved.openings[0].width_mm, 1000.);
        let prepared =
            prepare_wall_opening(&f.session, 1, actor(), f.wall, &[ty.clone()], &change).unwrap();
        let doc = prepared.prepared.document();
        assert!(read_opening_type(doc, ty.id).is_ok());
        assert_eq!(read_wall(doc, f.wall).unwrap().openings.len(), 1);
        // Without the new type the placement cannot resolve.
        assert!(plan_wall_opening(f.session.document(), f.wall, &[], &change).is_err());
    }

    #[test]
    fn vertical_cutouts_are_full_height_and_must_leave_one_wall() {
        let f = fixture(false);
        let add = |c| WallOpeningChange::AddCutout(c);
        // A chase inside the wall keeps it connected.
        let chase = plan_wall_opening(
            f.session.document(),
            f.wall,
            &[],
            &add(cutout(0, 2000., 300., Some(80.))),
        )
        .unwrap();
        let c = &chase.resolved.cutouts[0];
        assert!(!c.through && c.depth_mm == 80.);
        assert_eq!(
            chase.resolved.cutout_span(&chase.plan, c),
            ([2000., 2300.], [20., 100.])
        );
        // A full-depth gap at a wall end shortens the wall.
        let end = plan_wall_opening(
            f.session.document(),
            f.wall,
            &[],
            &add(cutout(0, 0., 500., None)),
        )
        .unwrap();
        assert!(end.resolved.cutouts[0].through);
        assert!(refused(&f, add(cutout(0, 2000., 300., None))).contains("splits it into two"));
        assert!(refused(&f, add(cutout(0, 0., 6000., None))).contains("whole wall"));
        assert!(refused(&f, add(cutout(0, 2000., 300., Some(250.)))).contains("200.0 mm thick"));
        assert!(refused(&f, add(cutout(0, 5900., 300., Some(50.)))).contains("does not fit"));
        assert!(refused(&f, add(cutout(0, 2000., 0., Some(50.)))).contains("positive"));
    }

    #[test]
    fn a_room_accepts_one_full_depth_gap_and_cutouts_respect_openings() {
        let mut f = fixture(true);
        let prepared = prepare_wall_opening(
            &f.session,
            1,
            actor(),
            f.wall,
            &[],
            &WallOpeningChange::AddCutout(cutout(0, 1000., 1200., None)),
        )
        .unwrap();
        f.session.commit(prepared.prepared).unwrap();
        let refuse = |change| {
            prepare_wall_opening(&f.session, 2, actor(), f.wall, &[], &change)
                .unwrap_err()
                .to_string()
        };
        assert!(
            refuse(WallOpeningChange::AddCutout(cutout(2, 1000., 500., None)))
                .contains("split this room")
        );
        // Joints keep one wall thickness on closed rooms.
        assert!(
            refuse(WallOpeningChange::AddCutout(cutout(1, 0., 500., Some(50.))))
                .contains("does not fit")
        );
        assert!(
            refuse(WallOpeningChange::AddOpening(opening(f.door, 0, 1800.))).contains("overlaps")
        );
        assert!(
            prepare_wall_opening(
                &f.session,
                2,
                actor(),
                f.wall,
                &[],
                &WallOpeningChange::AddCutout(cutout(2, 1000., 500., Some(100.)))
            )
            .is_ok()
        );
    }

    #[test]
    fn plan_points_locate_the_segment_and_distance_along_it() {
        let f = fixture(true);
        let planned = plan_wall_opening(
            f.session.document(),
            f.wall,
            &[],
            &WallOpeningChange::AddCutout(cutout(0, 1000., 100., Some(10.))),
        )
        .unwrap();
        let plan = &planned.plan;
        let at = locate_on_wall(plan, [1500., 60.], 1.).unwrap();
        assert_eq!((at.segment, at.along_mm, at.across_mm), (0, 1500., 60.));
        let east = locate_on_wall(plan, [5050., 2000.], 1.).unwrap();
        assert_eq!((east.segment, east.along_mm), (1, 2000.));
        assert!(locate_on_wall(plan, [2500., 2000.], 50.).is_none());
        // Just outside the face within tolerance still finds the wall.
        assert_eq!(
            locate_on_wall(plan, [1500., -130.], 50.).unwrap().segment,
            0
        );
    }
}
