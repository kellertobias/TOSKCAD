//! Bounded interference evidence for periodic, unmodified generator pairs.
use crate::{ModelError, compiler::checkpoint, error};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId, Mutation},
    model::{self, GearPairDefinition},
    parameters::EvaluatedParameters,
    references::TopologyKind,
};
use multicad_kernel::{BRepSelection, BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    f64::consts::{PI, TAU},
};
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
pub struct GearEngagementWitness {
    pub distance_mm: f64,
    pub kind: &'static str,
    /// Evaluation-local native face ordinals, never persistent source identity.
    pub faces: Option<[u32; 2]>,
    pub points_mm: [[f64; 3]; 2],
}
#[derive(Clone, Debug, Serialize)]
pub struct GearPairCheck {
    pub pair: EntityId,
    pub center_distance_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bevel: Option<model::ResolvedBevelPair>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worm: Option<model::ResolvedWormPair>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_engagement: Option<GearEngagementWitness>,
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
    overlap_checked(pair, a, b, drive, policy, false)
}
fn overlap_checked(
    pair: EntityId,
    a: &Solid,
    b: &Solid,
    drive: f64,
    policy: GeometryPolicy,
    parallel: bool,
) -> Result<(), ModelError> {
    if let Some(common) = (if parallel {
        a.intersect_parallel(b, policy)
    } else {
        a.boolean(b, BooleanOperation::Intersect, policy)
    })
    .map_err(|e| {
        error(
            Some(pair),
            format!("Native gear-pair interference check: {e}"),
        )
    })? {
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
fn engagement_flanks(
    solid: &Solid,
    feature: EntityId,
    teeth: u32,
    phase: f64,
    meridian: f64,
    pair: EntityId,
    names: (&str, &str),
) -> Result<Vec<u32>, ModelError> {
    let center = ((meridian - phase).rem_euclid(TAU) * f64::from(teeth) / TAU).round() as i64;
    let topology = solid
        .topology_snapshot(feature, 0)
        .map_err(|e| error(Some(pair), e))?;
    let mut faces = vec![];
    for offset in [0, -1, 1, -2, 2] {
        let tooth = (center + offset).rem_euclid(i64::from(teeth));
        let prefix = format!("{}/{tooth}/{}.", names.0, names.1);
        let mut found = [false; 2];
        for face in &topology.elements {
            if face.key.kind != TopologyKind::Face {
                continue;
            }
            for (i, suffix) in ["left", "right"].iter().enumerate() {
                if face.origins.iter().any(|origin| {
                    origin.source == feature && origin.role == format!("{prefix}{suffix}")
                }) {
                    found[i] = true;
                    if !faces.contains(&face.key.ordinal) {
                        faces.push(face.key.ordinal);
                    }
                }
            }
        }
        if !found.into_iter().all(|value| value) || faces.len() > 32 {
            return Err(error(
                Some(pair),
                "Gear engagement needs bounded native tooth-flank ancestry",
            ));
        }
    }
    Ok(faces)
}
fn rotary_engagement(
    pair: EntityId,
    features: [EntityId; 2],
    resolved: &model::ResolvedGearPair,
    solids: [&Solid; 2],
    policy: GeometryPolicy,
    bevel: bool,
) -> Result<GearEngagementWitness, ModelError> {
    let members = resolved.gears.each_ref().map(|gear| {
        if bevel {
            let g = gear.bevel().expect("resolved bevel pair");
            (
                g.teeth,
                g.nominal_tooth_thickness_mm - g.physical_tooth_thickness_mm,
            )
        } else {
            let g = gear.rotary().expect("resolved shifted spur pair");
            (
                g.teeth,
                g.nominal_tooth_thickness_mm - g.physical_tooth_thickness_mm,
            )
        }
    });
    let names = if bevel {
        ("bevel/tooth", "spherical_involute")
    } else {
        ("gear/whole/tooth", "involute")
    };
    let faces = [
        engagement_flanks(
            solids[0],
            features[0],
            members[0].0,
            resolved.phases_rad[0],
            0.,
            pair,
            names,
        )?,
        engagement_flanks(
            solids[1],
            features[1],
            members[1].0,
            resolved.phases_rad[1],
            PI,
            pair,
            names,
        )?,
    ];
    let clearance = members.iter().map(|gear| gear.1).sum::<f64>();
    let mut nearest = f64::INFINITY;
    // Whole-solid intersection was checked before entering this function.
    // We need an existence witness of close native flanks, not the global
    // filled-solid minimum (whose containment work would duplicate that check).
    // Every fast-path witness comes from actual named BRep faces. A miss falls
    // back to the original whole-solid query, preserving acceptance outside
    // this small search window and recording the different witness scope.
    for &a in &faces[0] {
        for &b in &faces[1] {
            let distance = solids[0]
                .minimum_distance(BRepSelection::Face(a), solids[1], BRepSelection::Face(b))
                .map_err(|e| error(Some(pair), e))?;
            nearest = nearest.min(distance.distance_mm);
            if distance.distance_mm <= clearance + policy.linear_tolerance_mm * 10. {
                let witness = &distance.witnesses[0];
                return Ok(GearEngagementWitness {
                    distance_mm: distance.distance_mm,
                    kind: if bevel {
                        "bevel_flank_faces"
                    } else {
                        "shifted_spur_flank_faces"
                    },
                    faces: Some([a, b]),
                    points_mm: [witness.a_mm, witness.b_mm],
                });
            }
        }
    }
    let distance = solids[0]
        .minimum_distance(BRepSelection::Whole, solids[1], BRepSelection::Whole)
        .map_err(|e| error(Some(pair), e))?;
    if distance.distance_mm <= clearance + policy.linear_tolerance_mm * 10. {
        let witness = &distance.witnesses[0];
        return Ok(GearEngagementWitness {
            distance_mm: distance.distance_mm,
            kind: "whole_solid_fallback",
            faces: None,
            points_mm: [witness.a_mm, witness.b_mm],
        });
    }
    Err(error(
        Some(pair),
        format!(
            "Gear members have no native engagement proximity: whole-solid gap {} mm and nearest candidate flank gap {nearest} mm exceed authored total thinning {clearance} mm",
            distance.distance_mm
        ),
    ))
}
fn worm_engagement(
    pair: EntityId,
    features: [EntityId; 2],
    resolved: &model::ResolvedGearPair,
    solids: [&Solid; 2],
    policy: GeometryPolicy,
) -> Result<GearEngagementWitness, ModelError> {
    let context = resolved.worm.as_ref().expect("resolved worm pair");
    let wi = context.worm_member.index();
    let gi = context.wheel_member.index();
    let clearance = resolved
        .gears
        .iter()
        .map(|g| g.worm().expect("worm member").thinning_mm)
        .sum::<f64>();
    let mut selected = [Vec::new(), Vec::new()];
    let worm_topology = solids[wi]
        .topology_snapshot(features[wi], 0)
        .map_err(|e| error(Some(pair), e))?;
    for face in &worm_topology.elements {
        if face.key.kind == TopologyKind::Face
            && face.origins.iter().any(|o| {
                o.source == features[wi]
                    && matches!(o.role.as_str(), "worm/flank.lower" | "worm/flank.upper")
            })
        {
            selected[wi].push(face.key.ordinal);
        }
    }
    let wheel_topology = solids[gi]
        .topology_snapshot(features[gi], 0)
        .map_err(|e| error(Some(pair), e))?;
    let teeth = i64::from(context.wheel_teeth);
    let center = (-resolved.phases_rad[gi] * teeth as f64 / TAU).round() as i64;
    for offset in [0, -1, 1, -2, 2] {
        let sector = (center + offset).rem_euclid(teeth);
        for face in &wheel_topology.elements {
            if face.key.kind == TopologyKind::Face
                && face.origins.iter().any(|o| {
                    o.source == features[gi]
                        && (o.role == format!("worm_wheel/sector/{sector}/flank.minus")
                            || o.role == format!("worm_wheel/sector/{sector}/flank.plus"))
                })
                && !selected[gi].contains(&face.key.ordinal)
            {
                selected[gi].push(face.key.ordinal);
            }
        }
    }
    // Prioritize screw faces near the central contact section. Numeric face
    // ordinals and distances remain evaluation-local, never persistent IDs.
    let centers = solids[wi]
        .faces()
        .map_err(|e| error(Some(pair), e))?
        .into_iter()
        .map(|f| (f.index, f.center_mm))
        .collect::<BTreeMap<_, _>>();
    let origin = resolved.mating[wi].origin_mm;
    selected[wi].sort_by(|a, b| {
        let score = |id| {
            centers.get(id).map_or(f64::INFINITY, |p| {
                (0..3).map(|j| (p[j] - origin[j]).powi(2)).sum::<f64>()
            })
        };
        score(a).total_cmp(&score(b))
    });
    // The whole-solid overlap check has already passed. These actual named
    // faces provide only an existence witness within the declared clearance;
    // a candidate miss always runs the unchanged whole-solid distance query.
    if selected[wi].len() <= 256 && selected[gi].len() <= 32 {
        for &a in &selected[0] {
            for &b in &selected[1] {
                let distance = solids[0]
                    .minimum_distance(BRepSelection::Face(a), solids[1], BRepSelection::Face(b))
                    .map_err(|e| error(Some(pair), e))?;
                if distance.distance_mm <= clearance + policy.linear_tolerance_mm * 10. {
                    let witness = &distance.witnesses[0];
                    return Ok(GearEngagementWitness {
                        distance_mm: distance.distance_mm,
                        kind: "worm_flank_faces",
                        faces: Some([a, b]),
                        points_mm: [witness.a_mm, witness.b_mm],
                    });
                }
            }
        }
    }
    let distance = solids[0]
        .minimum_distance(BRepSelection::Whole, solids[1], BRepSelection::Whole)
        .map_err(|e| error(Some(pair), e))?;
    if distance.distance_mm > clearance + policy.linear_tolerance_mm * 10. {
        return Err(error(
            Some(pair),
            format!(
                "Gear members have no native engagement proximity: gap {} mm exceeds authored total thinning {clearance} mm",
                distance.distance_mm
            ),
        ));
    }
    let witness = &distance.witnesses[0];
    Ok(GearEngagementWitness {
        distance_mm: distance.distance_mm,
        kind: "whole_solid_fallback",
        faces: None,
        points_mm: [witness.a_mm, witness.b_mm],
    })
}
fn check_pose(
    document: &Document,
    evaluated: &EvaluatedParameters,
    source: &GearPairDefinition,
    pair: EntityId,
    solids: [&Solid; 2],
    drive: f64,
    policy: GeometryPolicy,
) -> Result<Option<GearEngagementWitness>, ModelError> {
    let mut placed_solids = Vec::with_capacity(2);
    for (index, solid) in solids.into_iter().enumerate() {
        let pose =
            model::resolve_instance_pose(document, source.outputs.instances[index], evaluated)
                .map_err(|e| error(Some(pair), e))?;
        placed_solids.push(placed(solid, pose, pair)?);
    }
    let resolved = model::resolve_gear_pair_parameters(document, evaluated, &source.parameters)
        .map_err(|e| error(Some(pair), e))?;
    if resolved.worm.is_some() {
        overlap_checked(
            pair,
            &placed_solids[0],
            &placed_solids[1],
            drive,
            policy,
            true,
        )?;
    } else {
        overlap(pair, &placed_solids[0], &placed_solids[1], drive, policy)?;
    }
    if resolved.bevel.is_some() {
        return rotary_engagement(
            pair,
            source.parameters.gears,
            &resolved,
            [&placed_solids[0], &placed_solids[1]],
            policy,
            true,
        )
        .map(Some);
    }
    if resolved.worm.is_some() {
        return worm_engagement(
            pair,
            source.parameters.gears,
            &resolved,
            [&placed_solids[0], &placed_solids[1]],
            policy,
        )
        .map(Some);
    }
    let shifted_spur = resolved.gears.iter().all(|gear| {
        gear.rotary()
            .is_some_and(|r| r.family == model::GearFamily::Spur)
    }) && source.parameters.gears.iter().any(|&feature| {
        model::read_gear(document, feature).is_ok_and(|g| g.parameters.shift_mate.is_some())
    });
    if shifted_spur {
        return rotary_engagement(
            pair,
            source.parameters.gears,
            &resolved,
            [&placed_solids[0], &placed_solids[1]],
            policy,
            false,
        )
        .map(Some);
    }
    let engagement_clearance = if let Some(travel) = &resolved.rack_travel {
        let rack = resolved.gears[travel.rack_member.index()]
            .rack()
            .expect("rack member");
        let pinion = resolved.gears[travel.pinion_member.index()]
            .rotary()
            .expect("pinion member");
        // A no-overlap result alone also accepts separated bodies. Require an
        // actual native proximity witness bounded by the total authored thinning.
        // This is sampled engagement evidence, never a contact/load certificate.
        Some(
            rack.nominal_tooth_thickness_mm - rack.physical_tooth_thickness_mm
                + pinion.nominal_tooth_thickness_mm
                - pinion.physical_tooth_thickness_mm,
        )
    } else {
        None
    };
    if let Some(clearance) = engagement_clearance {
        let distance = placed_solids[0]
            .minimum_distance(
                BRepSelection::Whole,
                &placed_solids[1],
                BRepSelection::Whole,
            )
            .map_err(|e| error(Some(pair), e))?;
        if distance.distance_mm > clearance + policy.linear_tolerance_mm * 10. {
            return Err(error(
                Some(pair),
                format!(
                    "Gear members have no native engagement proximity: gap {} mm exceeds authored total thinning {} mm",
                    distance.distance_mm, clearance
                ),
            ));
        }
    }
    Ok(None)
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
        let shifted_engagement = source.parameters.gears.iter().any(|&feature| {
            model::read_gear(document, feature)
                .is_ok_and(|gear| gear.parameters.shift_mate.is_some())
        });
        let key = format!(
            "gear-pair-cycle/3/{SAMPLES}|{pair:?}|{:?}|{:?}|{specifications:?}|{:?}|{base:?}|{policy:?}|shifted-engagement:{shifted_engagement}",
            source.parameters.gears,
            source.outputs,
            resolved.gears.each_ref().map(|g| g.frame())
        );
        let key = if let Some(worm) = &resolved.worm {
            format!("{key}|{worm:?}")
        } else if let Some(travel) = &resolved.rack_travel {
            format!(
                "{key}|{:?}|{:?}|{}|{}|{}",
                travel.rack_member,
                travel.pinion_member,
                travel.min_drive_angle_rad,
                travel.max_drive_angle_rad,
                travel.pinion_period_rad
            )
        } else {
            key
        };
        let previous = cache
            .as_deref()
            .and_then(|c| c.completed.get(&key))
            .cloned();
        let reused = previous.is_some();
        let phases = if let Some(phases) = previous {
            phases
        } else {
            let phases = if let Some(travel) = &resolved.rack_travel {
                let mut phases = (0..SAMPLES)
                    .map(|i| travel.pinion_period_rad * (i as f64 / (SAMPLES - 1) as f64 - 0.5))
                    .collect::<Vec<_>>();
                phases.extend([travel.min_drive_angle_rad, travel.max_drive_angle_rad]);
                phases
            } else if let Some(worm) = &resolved.worm {
                (0..SAMPLES)
                    .map(|i| worm.worm_period_rad * i as f64 / (SAMPLES - 1) as f64)
                    .collect::<Vec<_>>()
            } else {
                (0..SAMPLES)
                    .map(|i| {
                        TAU / f64::from(resolved.gears[0].teeth()) * i as f64 / (SAMPLES - 1) as f64
                    })
                    .collect::<Vec<_>>()
            };
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
        let current = source
            .parameters
            .drive_angle
            .value_in(multicad_core::parameters::Unit::Radian, Some(evaluated))
            .map_err(|e| error(Some(pair), e))?;
        let current_engagement =
            check_pose(document, evaluated, &source, pair, [a, b], current, policy)?;
        reports.insert(pair,GearPairCheck{pair,current_engagement,center_distance_mm:resolved.center_distance_mm,bevel:resolved.bevel.clone(),worm:resolved.worm.clone(),cycle_drive_angles_rad:phases,cycle_reused:reused,current_drive_angle_rad:current,current_pose_checked:true,maximum_overlap_volume_mm3:0.,scope:if resolved.worm.is_some() { "Native current pose and nine sampled phases over one full ZA worm turn with native no-overlap and engagement proximity witnesses for its generated wheel. No continuous collision-free motion, strength or manufacturing-fit certification." } else if resolved.rack_travel.is_some() { "Native current pose, nine sampled phases over one pinion tooth period and both conservative finite travel endpoints. No continuous collision-free motion, strength or manufacturing-fit certification." } else if resolved.bevel.is_some() { "Native current pose and nine sampled phases over one spherical-involute bevel tooth cycle with native no-overlap and engagement proximity witnesses. No continuous collision-free motion, strength or manufacturing-fit certification." } else if shifted_engagement { "Native current pose and nine sampled phases over one matched shifted Spur tooth cycle, with whole-solid no-overlap and native named-flank proximity witnesses or whole-solid fallback. No continuous collision-free motion, strength or manufacturing-fit certification." } else { "Native current pose and nine sampled phases over one pinion tooth cycle, under the unmodified periodic generator assumption. No continuous collision-free motion, strength or manufacturing-fit certification." }});
    }
    Ok(reports)
}

#[cfg(test)]
#[path = "gear_pairs/bevel_tests.rs"]
mod bevel_tests;

#[cfg(test)]
#[path = "gear_pairs/coverage_tests.rs"]
mod coverage_tests;

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
    #[test]
    fn separated_rack_and_wrong_phase_cannot_pass_native_pair_validation() {
        let mut session =
            Session::new(Document::new("Native rack negative controls").unwrap()).unwrap();
        let request = GearPairRequest::new(
            "Rack pair",
            [
                GearRequest::new(
                    "Rack",
                    GearParameters {
                        family: GearFamily::Rack,
                        teeth: Dimension::Literal {
                            value: 20.,
                            unit: "count".into(),
                        },
                        rack: Some(RackDimensions {
                            backing_thickness: length(3.),
                        }),
                        module: length(1.),
                        face_width: length(2.),
                        bore_diameter: length(0.),
                        ..Default::default()
                    },
                ),
                GearRequest::new(
                    "Pinion",
                    GearParameters {
                        module: length(1.),
                        face_width: length(2.),
                        ..Default::default()
                    },
                ),
            ],
        );
        session
            .commit(
                prepare_gear_pair(&session, 0, Actor::local(Channel::Ui), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let source = read_gear_pair(session.document(), request.pair).unwrap();
        let policy = GeometryPolicy::default();
        let solids: Vec<_> = request
            .parameters
            .gears
            .iter()
            .map(|id| {
                let recipe = read_gear(session.document(), *id).unwrap();
                crate::gears::build(*id, &recipe, session.parameters(), policy).unwrap()
            })
            .collect();
        let separated = solids[1]
            .transformed(RigidTransform {
                translation_mm: [1000., 0., 0.],
                rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            })
            .unwrap();
        let failure = check_pose(
            session.document(),
            session.parameters(),
            &source,
            request.pair,
            [&solids[0], &separated],
            0.,
            policy,
        )
        .unwrap_err();
        assert!(
            failure
                .to_string()
                .contains("no native engagement proximity")
        );
        let wrong_phase = solids[0]
            .transformed(RigidTransform {
                translation_mm: [std::f64::consts::PI / 2., 0., 0.],
                rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            })
            .unwrap();
        let failure = check_pose(
            session.document(),
            session.parameters(),
            &source,
            request.pair,
            [&wrong_phase, &solids[1]],
            0.,
            policy,
        )
        .unwrap_err();
        assert!(failure.to_string().contains("overlaps by"));
        assert_eq!(session.revision(), 1);
    }
}
