//! Bounded interference evidence for periodic, unmodified generator pairs.
use crate::{ModelError, compiler::checkpoint, error};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId, Mutation},
    model::{self, GearPairDefinition},
    parameters::EvaluatedParameters,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use serde::Serialize;
use std::{collections::BTreeMap, f64::consts::TAU};
const SAMPLES: usize = 9;
const MAX_PAIRS: usize = 64;
pub(crate) fn validate_parameters(
    document: &Document,
    evaluated: &EvaluatedParameters,
) -> Result<(), ModelError> {
    let mut count = 0;
    for entity in document.entities() {
        if !matches!(&entity.data,EntityData::Extension{definition,..} if definition.type_id==model::GEAR_PAIR_TYPE)
        {
            continue;
        }
        count += 1;
        if count > MAX_PAIRS {
            return Err(error(
                None,
                "Native gear-pair validation exceeds the 64-pair budget",
            ));
        }
        let source =
            model::read_gear_pair(document, entity.id).map_err(|e| error(Some(entity.id), e))?;
        model::resolve_gear_pair_parameters(document, evaluated, &source.parameters)
            .map_err(|e| error(Some(entity.id), e))?;
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize)]
pub struct GearPairCheck {
    pub pair: EntityId,
    pub center_distance_mm: f64,
    pub cycle_drive_angles_rad: Vec<f64>,
    pub cycle_reused: bool,
    pub current_drive_angle_rad: f64,
    pub current_pose_checked: bool,
    pub maximum_overlap_volume_mm3: f64,
    pub scope: &'static str,
}
#[derive(Default)]
pub(crate) struct PairValidationCache {
    completed: BTreeMap<String, Vec<f64>>,
}
impl PairValidationCache {
    pub(crate) fn clear(&mut self) {
        self.completed.clear();
    }
    fn insert(&mut self, key: String, phases: Vec<f64>) {
        if self.completed.len() >= MAX_PAIRS {
            self.completed.pop_first();
        }
        self.completed.insert(key, phases);
    }
}
fn placed(solid: &Solid, pose: model::RigidPose, pair: EntityId) -> Result<Solid, ModelError> {
    solid
        .transformed(RigidTransform {
            rotation: pose.rotation,
            translation_mm: pose.translation_mm,
        })
        .map_err(|e| error(Some(pair), e))
}
fn overlap(
    pair: EntityId,
    a: &Solid,
    b: &Solid,
    drive: f64,
    policy: GeometryPolicy,
) -> Result<(), ModelError> {
    if let Some(common) = a
        .boolean(b, BooleanOperation::Intersect, policy)
        .map_err(|e| {
            error(
                Some(pair),
                format!("Native gear-pair interference check: {e}"),
            )
        })?
    {
        let volume = common.volume_mm3().map_err(|e| error(Some(pair), e))?;
        return Err(error(
            Some(pair),
            format!(
                "Gear pair overlaps by {volume:.8} mm³ at tested drive {:.6} degrees; repair its geometry or clearance",
                drive.to_degrees()
            ),
        ));
    }
    Ok(())
}
fn check_pose(
    document: &Document,
    evaluated: &EvaluatedParameters,
    source: &GearPairDefinition,
    pair: EntityId,
    solids: [&Solid; 2],
    drive: f64,
    policy: GeometryPolicy,
) -> Result<(), ModelError> {
    let mut placed_solids = Vec::with_capacity(2);
    for (index, solid) in solids.into_iter().enumerate() {
        let pose =
            model::resolve_instance_pose(document, source.outputs.instances[index], evaluated)
                .map_err(|e| error(Some(pair), e))?;
        placed_solids.push(placed(solid, pose, pair)?);
    }
    overlap(pair, &placed_solids[0], &placed_solids[1], drive, policy)
}
fn sample_document(
    document: &Document,
    pair: EntityId,
    source: &GearPairDefinition,
    drive: f64,
) -> Result<Document, ModelError> {
    let mut source = source.clone();
    source.parameters.drive_angle = Dimension::Literal {
        value: drive,
        unit: "rad".into(),
    };
    let mut entity = document
        .get(pair)
        .ok_or_else(|| error(Some(pair), "Gear pair is missing"))?
        .clone();
    entity.data = EntityData::Extension {
        definition: source.definition().map_err(|e| error(Some(pair), e))?,
        inputs: source
            .parameters
            .inputs()
            .map_err(|e| error(Some(pair), e))?,
    };
    document
        .edited(&[Mutation::Replace(entity)])
        .map_err(|e| error(Some(pair), e))
}
pub(crate) fn validate(
    document: &Document,
    evaluated: &EvaluatedParameters,
    solids: &BTreeMap<EntityId, Solid>,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
    mut cache: Option<&mut PairValidationCache>,
) -> Result<BTreeMap<EntityId, GearPairCheck>, ModelError> {
    let pairs=document.entities().filter(|e|matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==model::GEAR_PAIR_TYPE)).collect::<Vec<_>>();
    if pairs.len() > MAX_PAIRS {
        return Err(error(
            None,
            "Native gear-pair validation exceeds the 64-pair budget",
        ));
    }
    let mut reports = BTreeMap::new();
    for entity in pairs {
        checkpoint(interrupt)?;
        let pair = entity.id;
        let source = model::read_gear_pair(document, pair).map_err(|e| error(Some(pair), e))?;
        let resolved = model::resolve_gear_pair_parameters(document, evaluated, &source.parameters)
            .map_err(|e| error(Some(pair), e))?;
        let a = solids
            .get(&source.parameters.gears[0])
            .ok_or_else(|| error(Some(pair), "First paired native gear is unavailable"))?;
        let b = solids
            .get(&source.parameters.gears[1])
            .ok_or_else(|| error(Some(pair), "Second paired native gear is unavailable"))?;
        let specifications = [
            crate::gears::specification(source.parameters.gears[0], &resolved.gears[0])?,
            crate::gears::specification(source.parameters.gears[1], &resolved.gears[1])?,
        ];
        let base = source
            .parameters
            .frame
            .resolve(evaluated)
            .map_err(|e| error(Some(pair), e))?;
        // Process-local signature of every actual constructor input. Debug is
        // derived on these closed structs and never persisted as a file format.
        // Drive is excluded only from the periodic cycle proof; its actual
        // current pose is checked independently below on every compile.
        let key = format!(
            "gear-pair-cycle/1/{SAMPLES}|{pair:?}|{:?}|{:?}|{specifications:?}|{:?}|{base:?}|{policy:?}",
            source.parameters.gears,
            source.outputs,
            resolved.gears.each_ref().map(|g| g.frame)
        );
        let previous = cache
            .as_deref()
            .and_then(|c| c.completed.get(&key))
            .cloned();
        let reused = previous.is_some();
        let phases = if let Some(phases) = previous {
            phases
        } else {
            let phases = (0..SAMPLES)
                .map(|i| TAU / f64::from(resolved.gears[0].teeth) * i as f64 / (SAMPLES - 1) as f64)
                .collect::<Vec<_>>();
            for drive in &phases {
                checkpoint(interrupt)?;
                let sample = sample_document(document, pair, &source, *drive)?;
                check_pose(&sample, evaluated, &source, pair, [a, b], *drive, policy)?;
            }
            if let Some(cache) = cache.as_deref_mut() {
                cache.insert(key, phases.clone());
            }
            phases
        };
        checkpoint(interrupt)?;
        let current = resolved.phases_rad[0];
        check_pose(document, evaluated, &source, pair, [a, b], current, policy)?;
        reports.insert(pair,GearPairCheck{pair,center_distance_mm:resolved.center_distance_mm,cycle_drive_angles_rad:phases,cycle_reused:reused,current_drive_angle_rad:current,current_pose_checked:true,maximum_overlap_volume_mm3:0.,scope:"Native current pose and nine sampled phases over one pinion tooth cycle, under the unmodified periodic generator assumption. No continuous collision-free motion, strength or manufacturing-fit certification."});
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::*,
    };
    use std::cell::Cell;
    fn candidate() -> (Session, GearPairRequest, BTreeMap<EntityId, Solid>) {
        let mut session =
            Session::new(Document::new("Native pair failure controls").unwrap()).unwrap();
        let requests = std::array::from_fn(|i| {
            GearRequest::new(
                if i == 0 { "Pinion" } else { "Wheel" },
                GearParameters {
                    teeth: Dimension::Literal {
                        value: if i == 0 { 20. } else { 30. },
                        unit: "count".into(),
                    },
                    face_width: length(4.),
                    ..Default::default()
                },
            )
        });
        let request = GearPairRequest::new("Pair", requests);
        let prepared = prepare_gear_pair(&session, 0, Actor::local(Channel::Ui), &request).unwrap();
        session.commit(prepared.prepared).unwrap();
        let solids = request
            .parameters
            .gears
            .into_iter()
            .map(|id| {
                let source = read_gear(session.document(), id).unwrap();
                (
                    id,
                    crate::gears::build(
                        id,
                        &source,
                        session.parameters(),
                        GeometryPolicy::default(),
                    )
                    .unwrap(),
                )
            })
            .collect();
        (session, request, solids)
    }
    #[test]
    fn deliberately_wrong_phase_produces_a_real_native_overlap_diagnostic() {
        let (session, request, solids) = candidate();
        let source = read_gear_pair(session.document(), request.pair).unwrap();
        let a = &solids[&request.parameters.gears[0]];
        let b = &solids[&request.parameters.gears[1]];
        check_pose(
            session.document(),
            session.parameters(),
            &source,
            request.pair,
            [a, b],
            0.,
            GeometryPolicy::default(),
        )
        .unwrap();
        let wrong = placed(
            b,
            RigidPose {
                translation_mm: [50., 0., 0.],
                rotation: [[-1., 0., 0.], [0., -1., 0.], [0., 0., 1.]],
            },
            request.pair,
        )
        .unwrap();
        let diagnostic =
            overlap(request.pair, a, &wrong, 0., GeometryPolicy::default()).unwrap_err();
        assert_eq!(diagnostic.entity, Some(request.pair));
        assert!(diagnostic.message.contains("overlaps"));
    }
    #[test]
    fn superseded_cycle_never_stores_a_partial_validation_proof() {
        let (session, request, solids) = candidate();
        let mut cache = PairValidationCache::default();
        let visits = Cell::new(0);
        let interrupt = || {
            visits.set(visits.get() + 1);
            visits.get() > 2
        };
        let result = validate(
            session.document(),
            session.parameters(),
            &solids,
            GeometryPolicy::default(),
            Some(&interrupt),
            Some(&mut cache),
        );
        assert!(result.is_err());
        assert!(visits.get() > 2);
        assert!(cache.completed.is_empty());
        assert_eq!(session.revision(), 1);
        assert_eq!(
            read_gear_pair(session.document(), request.pair)
                .unwrap()
                .outputs,
            request.outputs
        );
    }
}
