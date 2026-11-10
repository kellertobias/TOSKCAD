//! Native mounting geometry and nut threads from retained canonical parameters.
mod custom_connection;
mod screw_profiles;
mod wood_screws;
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{
        MechanicalPartDefinition, MechanicalPartParameters, ResolvedMechanicalPart,
        ResolvedScrewDrive, ResolvedScrewHead, ThreadMode,
    },
    parameters::EvaluatedParameters,
    references::Origin,
    sketch::Frame,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid,
    rust_kernel::headed_screw::{ScrewHead, ScrewSpec},
};

use std::{cell::RefCell, collections::BTreeMap};

const SCREW_FACTORY_ENTRIES: usize = 16;
const SCREW_FACTORY_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ScrewKey {
    numeric: Vec<u64>,
    // Derived Debug preserves every resolved field, including fit semantics;
    // retaining it also makes future resolved-field additions invalidate hits.
    resolved_guard: String,
}
fn screw_key(resolved: &ResolvedMechanicalPart, policy: GeometryPolicy) -> Option<ScrewKey> {
    let ResolvedMechanicalPart::MachineScrew {
        underhead_length_mm,
        shank_radius_mm,
        head,
        drive,
        thread,
        ..
    } = resolved
    else {
        return None;
    };
    let mut numbers = vec![
        *underhead_length_mm,
        *shank_radius_mm,
        head.height_mm(),
        thread.nominal_major_radius_mm,
        thread.major_radius_mm,
        thread.pitch_mm,
        thread.lead_mm,
        thread.length_mm,
        thread.minimum_wall_mm,
        thread.profile.depth_mm,
        thread.profile.crest_width_mm,
        thread.profile.lower_flank_deg,
        thread.profile.upper_flank_deg,
        thread.profile.radial_taper,
        policy.linear_tolerance_mm,
        policy.boolean_fuzzy_mm,
        policy.minimum_feature_mm,
        policy.maximum_coordinate_mm,
    ];
    numbers.extend(thread.frame.origin);
    numbers.extend(thread.frame.u);
    numbers.extend(thread.frame.v);
    numbers.extend(thread.frame.n);
    match head {
        ResolvedScrewHead::SocketCap { radius_mm, .. } => numbers.push(*radius_mm),
        ResolvedScrewHead::ExternalHex {
            across_flats_mm, ..
        } => numbers.push(*across_flats_mm),
        ResolvedScrewHead::Pan {
            radius_mm,
            crown_radius_mm,
            top_land_radius_mm,
            neck_radius_mm,
            ..
        }
        | ResolvedScrewHead::Button {
            radius_mm,
            crown_radius_mm,
            top_land_radius_mm,
            neck_radius_mm,
            ..
        } => numbers.extend([
            *radius_mm,
            *crown_radius_mm,
            *top_land_radius_mm,
            *neck_radius_mm,
        ]),
        ResolvedScrewHead::Countersunk {
            radius_mm,
            included_angle_deg,
            edge_chamfer_mm,
            neck_radius_mm,
            ..
        } => numbers.extend([
            *radius_mm,
            *included_angle_deg,
            *edge_chamfer_mm,
            *neck_radius_mm,
        ]),
    }
    match drive {
        ResolvedScrewDrive::Hexalobular {
            major_radius_mm,
            minor_radius_mm,
            outer_lobe_radius_mm,
            depth_mm,
            bottom_angle_deg,
        } => numbers.extend([
            *major_radius_mm,
            *minor_radius_mm,
            *outer_lobe_radius_mm,
            *depth_mm,
            *bottom_angle_deg,
        ]),
        ResolvedScrewDrive::PhillipsH { profile: p } => {
            numbers.extend([
                p.mouth_diameter_mm,
                p.mouth_radius_mm,
                p.b_width_mm,
                p.g_diameter_mm,
                p.tilt_angle_deg,
                p.outer_flank_angle_deg,
                p.floor_angle_deg,
                p.outer_corner_angle_deg,
            ]);
            match p.inner {
                multicad_core::model::ResolvedPhillipsInnerForm::Rounded { radius_mm } => {
                    numbers.push(radius_mm)
                }
                multicad_core::model::ResolvedPhillipsInnerForm::ShallowV {
                    angle_deg,
                    e_width_mm,
                } => numbers.extend([angle_deg, e_width_mm]),
            }
        }
        _ => {}
    }
    if let ResolvedScrewDrive::HexSocket {
        across_flats_mm,
        depth_mm,
    } = drive
    {
        numbers.extend([*across_flats_mm, *depth_mm]);
    }
    let mut numeric = numbers.into_iter().map(f64::to_bits).collect::<Vec<_>>();
    for value in [
        thread.nominal_pitch_diameter_mm,
        thread.actual_pitch_diameter_mm,
        thread.profile.rounding_radius_mm,
    ] {
        numeric.push(u64::from(value.is_some()));
        numeric.push(value.unwrap_or(0.).to_bits());
    }
    numeric.push(u64::from(thread.starts));
    Some(ScrewKey {
        numeric,
        resolved_guard: format!("machine-screw-factory/1|{resolved:?}"),
    })
}
struct ScrewEntry {
    solid: Solid,
    owner: EntityId,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
struct ScrewFactory {
    entries: BTreeMap<ScrewKey, ScrewEntry>,
    clock: u64,
    bytes: usize,
    requests: u64,
    hits: u64,
    builds: u64,
}
thread_local! {static SCREWS: RefCell<ScrewFactory> = RefCell::new(ScrewFactory::default());}
fn cached_screw(key: &ScrewKey, feature: EntityId) -> Option<Solid> {
    SCREWS.with(|slot| {
        let mut cache = slot.borrow_mut();
        cache.clock += 1;
        cache.requests += 1;
        let clock = cache.clock;
        let entry = cache.entries.get_mut(key)?;
        let result = entry
            .solid
            .rebind_exclusive_owner(entry.owner, feature)
            .ok()?;
        entry.used = clock;
        cache.hits += 1;
        Some(result)
    })
}
fn remember_screw(key: ScrewKey, feature: EntityId, solid: &Solid) {
    // Retain only an already completed, fully owned native result. Optional
    // bookkeeping errors cannot turn successful geometry into a model error.
    let Ok(owned) = solid.rebind_exclusive_owner(feature, feature) else {
        return;
    };
    let Ok(brep) = owned.to_brep() else {
        return;
    };
    let Ok(topology) = owned.topology_snapshot(feature, 0) else {
        return;
    };
    let Ok(receipt) = serde_json::to_vec(&topology) else {
        return;
    };
    let Some(bytes) = brep.len().checked_add(receipt.len()) else {
        return;
    };
    if bytes > SCREW_FACTORY_BYTES {
        return;
    }
    SCREWS.with(|slot| {
        let mut cache = slot.borrow_mut();
        cache.builds += 1;
        if let Some(old) = cache.entries.remove(&key) {
            cache.bytes -= old.bytes;
        }
        while cache.entries.len() >= SCREW_FACTORY_ENTRIES
            || cache.bytes > SCREW_FACTORY_BYTES - bytes
        {
            let Some(oldest) = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(old) = cache.entries.remove(&oldest) {
                cache.bytes -= old.bytes;
            }
        }
        cache.bytes += bytes;
        let used = cache.clock;
        cache.entries.insert(
            key,
            ScrewEntry {
                solid: owned,
                owner: feature,
                bytes,
                used,
            },
        );
    });
}

fn hexagon(across_flats: f64) -> Vec<CurvedEdge2d> {
    let radius = across_flats / 3_f64.sqrt();
    let point = |i: usize| {
        let angle = std::f64::consts::PI / 6. + i as f64 * std::f64::consts::TAU / 6.;
        [radius * angle.cos(), radius * angle.sin()]
    };
    (0..6)
        .map(|i| CurvedEdge2d::Line {
            a: point(i),
            b: point((i + 1) % 6),
        })
        .collect()
}

/// Screw components are built in the underhead coordinate system. Each owns
/// distinct profile and cap roles before native ancestry is combined.
fn screw_prism(
    feature: EntityId,
    component: &str,
    z: f64,
    height: f64,
    outer: Vec<CurvedEdge2d>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let origins = (0..outer.len())
        .map(|i| Origin {
            source: feature,
            role: format!("screw.{component}/edge/{i}"),
        })
        .collect::<Vec<_>>();
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: [0., 0., z],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        outer,
        holes: vec![],
    };
    Solid::extrude_curved_profile_vector_named(
        &profile,
        [0., 0., height],
        feature,
        &origins,
        policy,
    )
    .and_then(|s| {
        s.with_feature_cap_roles(
            feature,
            &format!("screw.{component}/start"),
            &format!("screw.{component}/end"),
        )
    })
    .map_err(|e| error(Some(feature), e))
}
fn screw_boolean(
    feature: EntityId,
    a: &Solid,
    b: &Solid,
    operation: BooleanOperation,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let result = a
        .boolean(b, operation, policy)
        .map_err(|e| error(Some(feature), e))?
        .ok_or_else(|| error(Some(feature), "Machine screw operation produced no solid"))?;
    if result.solid_count().map_err(|e| error(Some(feature), e))? != 1 {
        return Err(error(
            Some(feature),
            "Machine screw must remain one connected solid",
        ));
    }
    Ok(result)
}
fn extrude(
    feature: EntityId,
    frame: Frame,
    height: f64,
    outer: Vec<CurvedEdge2d>,
    holes: Vec<Vec<CurvedEdge2d>>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let mut origins = (0..outer.len())
        .map(|i| Origin {
            source: feature,
            role: format!("hardware.outer/{i}"),
        })
        .collect::<Vec<_>>();
    for (loop_index, edges) in holes.iter().enumerate() {
        origins.extend((0..edges.len()).map(|i| Origin {
            source: feature,
            role: format!("hardware.bore/{loop_index}/{i}"),
        }));
    }
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: frame.origin,
            u: frame.u,
            v: frame.v,
        },
        outer,
        holes,
    };
    Solid::extrude_curved_profile_vector_named(
        &profile,
        frame.n.map(|n| n * height),
        feature,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(feature), e))
}
fn ring(
    feature: EntityId,
    frame: Frame,
    bore: f64,
    outside: f64,
    height: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let circle = |radius_mm| CurvedEdge2d::Circle {
        center: [0., 0.],
        radius_mm,
    };
    extrude(
        feature,
        frame,
        height,
        vec![circle(outside)],
        vec![vec![circle(bore)]],
        policy,
    )
}
pub(crate) fn build(
    feature: EntityId,
    source: &MechanicalPartDefinition,
    evaluated: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let resolved = source
        .parameters
        .resolve(evaluated)
        .map_err(|e| error(Some(feature), e))?;
    build_resolved(feature, resolved, policy)
}

pub(crate) fn build_rust(
    feature: EntityId,
    source: &MechanicalPartDefinition,
    evaluated: &EvaluatedParameters,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message| error(Some(feature), message);
    let MechanicalPartParameters::MachineScrew {
        thread: authored_thread,
        ..
    } = &source.parameters
    else {
        return Err(fail(
            "Rust mechanical parts currently support modeled machine screws",
        ));
    };
    crate::threads::validate_rust_family(feature, authored_thread.family)?;
    let ResolvedMechanicalPart::MachineScrew {
        underhead_length_mm,
        shank_radius_mm,
        head,
        drive,
        thread,
        ..
    } = source
        .parameters
        .resolve(evaluated)
        .map_err(|cause| error(Some(feature), cause))?
    else {
        unreachable!("Resolved an authored machine screw");
    };
    let (profile, root_radius, frame) = crate::threads::rust_profile(feature, &thread)?;
    let head = match (head, drive) {
        (
            ResolvedScrewHead::SocketCap {
                radius_mm,
                height_mm,
            },
            ResolvedScrewDrive::HexSocket {
                across_flats_mm,
                depth_mm,
            },
        ) => ScrewHead::SocketCap {
            radius_mm,
            height_mm,
            socket_across_flats_mm: across_flats_mm,
            socket_depth_mm: depth_mm,
        },
        (
            ResolvedScrewHead::ExternalHex {
                across_flats_mm,
                height_mm,
            },
            ResolvedScrewDrive::ExternalHex,
        ) => ScrewHead::ExternalHex {
            across_flats_mm,
            height_mm,
        },
        _ => {
            return Err(fail(
                "Rust machine screws support a socket cap with a flat hex socket or an external hex head",
            ));
        }
    };
    Solid::rust_machine_screw(
        profile,
        root_radius,
        ScrewSpec {
            head,
            shank_length_mm: underhead_length_mm - thread.length_mm,
            shank_radius_mm,
        },
        feature,
        frame,
        policy,
    )
    .map_err(|cause| error(Some(feature), cause))
}

pub(super) fn build_resolved(
    feature: EntityId,
    resolved: ResolvedMechanicalPart,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let factory_key = screw_key(&resolved, policy);
    match resolved {
        ResolvedMechanicalPart::CustomConnection { parameters } => {
            custom_connection::build(feature, *parameters, policy)
        }
        ResolvedMechanicalPart::WoodScrew { parameters } => {
            wood_screws::build(feature, &parameters, policy)
        }
        ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            shank_radius_mm,
            head,
            drive,
            mut thread,
            ..
        } => {
            let frame = thread.frame;
            let shank_length = underhead_length_mm - thread.length_mm;
            if let ResolvedScrewDrive::HexSocket {
                across_flats_mm,
                depth_mm,
            } = &drive
                && matches!(head, ResolvedScrewHead::SocketCap { .. })
                && (head.height_mm() - depth_mm < policy.minimum_feature_mm
                    || head.minimum_radius_mm() - across_flats_mm / 3_f64.sqrt()
                        < policy.minimum_feature_mm)
            {
                return Err(error(
                    Some(feature),
                    "Hex socket floor or corner wall is below native minimum feature size",
                ));
            }
            policy.validate().map_err(|e| error(Some(feature), e))?;
            screw_profiles::validate_native_clearance(
                feature,
                &head,
                &drive,
                shank_radius_mm,
                underhead_length_mm,
                &thread,
                policy,
            )?;
            let factory_key = factory_key
                .ok_or_else(|| error(Some(feature), "Missing resolved screw factory key"))?;
            if let Some(cached) = cached_screw(&factory_key, feature) {
                return Ok(cached);
            }
            let circle = |radius_mm| {
                vec![CurvedEdge2d::Circle {
                    center: [0., 0.],
                    radius_mm,
                }]
            };
            let new_profile = screw_profiles::new_head(&head);
            let height = head.height_mm();
            let early_socket = matches!(head, ResolvedScrewHead::SocketCap { .. })
                && matches!(drive, ResolvedScrewDrive::HexSocket { .. });
            let mut result = if new_profile {
                screw_profiles::blank(feature, &head, shank_radius_mm, shank_length, policy)?
            } else {
                let head_outer = match head {
                    ResolvedScrewHead::SocketCap { radius_mm, .. } => circle(radius_mm),
                    ResolvedScrewHead::ExternalHex {
                        across_flats_mm, ..
                    } => hexagon(across_flats_mm),
                    _ => unreachable!("New head profile dispatched above"),
                };
                screw_prism(feature, "head", -height, height, head_outer, policy)?
            };
            // Preserve the old flat-cap/flat-hex-socket construction exactly.
            if early_socket
                && let ResolvedScrewDrive::HexSocket {
                    across_flats_mm,
                    depth_mm,
                } = &drive
            {
                let socket = screw_prism(
                    feature,
                    "socket",
                    -height,
                    *depth_mm,
                    hexagon(*across_flats_mm),
                    policy,
                )?;
                result = screw_boolean(
                    feature,
                    &result,
                    &socket,
                    BooleanOperation::Subtract,
                    policy,
                )?;
            }
            if !new_profile && shank_length > 0. {
                let shank = screw_prism(
                    feature,
                    "shank",
                    0.,
                    shank_length,
                    circle(shank_radius_mm),
                    policy,
                )?;
                result = screw_boolean(feature, &result, &shank, BooleanOperation::Union, policy)?;
            }
            thread.frame = Frame {
                origin: [0., 0., shank_length],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                n: [0., 0., 1.],
            };
            let threaded = crate::threads::build_resolved(
                feature,
                *thread,
                ThreadMode::NewExternal,
                None,
                policy,
            )
            .map_err(|e| error(Some(feature), format!("Screw standalone thread: {e}")))?
            .with_feature_cap_roles(feature, "screw.thread/start", "screw.thread/end")
            .map_err(|e| error(Some(feature), e))?;
            result = screw_boolean(feature, &result, &threaded, BooleanOperation::Union, policy)
                .map_err(|e| error(Some(feature), format!("Screw thread fuse: {e}")))?;
            // New recesses may legitimately reach through a countersunk head
            // into its neck. Cut the complete head/shank/thread body so a later
            // union cannot refill the retained cavity.
            if !early_socket
                && let Some(cutter) = screw_profiles::cutter(feature, &head, &drive, policy)?
            {
                result = screw_boolean(
                    feature,
                    &result,
                    &cutter,
                    BooleanOperation::Subtract,
                    policy,
                )?;
            }
            let topology = result
                .topology_snapshot(feature, 0)
                .map_err(|e| error(Some(feature), e))?;
            if topology.elements.iter().any(|e| {
                e.key.kind == multicad_core::references::TopologyKind::Face && e.origins.is_empty()
            }) {
                return Err(error(
                    Some(feature),
                    "Machine screw has incomplete native face ancestry",
                ));
            }
            let completed = result
                .transformed(RigidTransform {
                    rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.n[i]]),
                    translation_mm: frame.origin,
                })
                .map_err(|e| error(Some(feature), format!("Screw world transform: {e}")))?;
            remember_screw(factory_key, feature, &completed);
            Ok(completed)
        }
        ResolvedMechanicalPart::Washer {
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            frame,
        } => ring(
            feature,
            frame,
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            policy,
        ),
        ResolvedMechanicalPart::BearingEnvelope {
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            frame,
        } => ring(
            feature,
            frame,
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            policy,
        ),
        ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } => {
            let radius = across_flats_mm / 3_f64.sqrt();
            let point = |i: usize| {
                let angle = std::f64::consts::PI / 6. + i as f64 * std::f64::consts::TAU / 6.;
                [radius * angle.cos(), radius * angle.sin()]
            };
            let outer = (0..6)
                .map(|i| CurvedEdge2d::Line {
                    a: point(i),
                    b: point((i + 1) % 6),
                })
                .collect();
            let blank = extrude(
                feature,
                thread.frame,
                thread.length_mm,
                outer,
                vec![],
                policy,
            )?;
            crate::threads::build_resolved(
                feature,
                *thread,
                ThreadMode::CutInternal,
                Some(&blank),
                policy,
            )
        }
    }
}

#[cfg(test)]
mod screw_factory_tests {
    use super::*;
    use multicad_core::{
        document::Document,
        model::{self, BodyOutput, MechanicalPartParameters, ThreadRepresentation, length},
        parameters,
    };
    fn reset() {
        SCREWS.with(|slot| *slot.borrow_mut() = ScrewFactory::default());
    }
    fn source() -> MechanicalPartDefinition {
        MechanicalPartDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            parameters: model::default_machine_screw(),
            provenance: None,
        }
    }
    fn stats() -> (u64, u64, u64, usize, usize) {
        SCREWS.with(|slot| {
            let c = slot.borrow();
            (c.requests, c.hits, c.builds, c.entries.len(), c.bytes)
        })
    }
    #[test]
    fn completed_screw_reuses_full_native_result_for_fresh_owners_and_isolated_threads() {
        reset();
        let document = Document::new("Whole screw factory").unwrap();
        let values = parameters::evaluate(&document).unwrap();
        let recipe = source();
        let policy = GeometryPolicy::default();
        let first = EntityId::new();
        let second = EntityId::new();
        let a = build(first, &recipe, &values, policy).unwrap();
        let geometry = a.to_brep().unwrap();
        let before = a.topology_snapshot(first, 0).unwrap();
        let native_before = multicad_kernel::thread_factory_stats().unwrap();
        let b = build(second, &recipe, &values, policy).unwrap();
        assert_eq!(stats().0, 2);
        assert_eq!(stats().1, 1);
        assert_eq!(stats().2, 1);
        assert_eq!(
            multicad_kernel::thread_factory_stats().unwrap(),
            native_before,
            "Whole screw hit must not even request a thread subshape"
        );
        assert_eq!(b.to_brep().unwrap(), geometry);
        assert_eq!(a.topology_snapshot(first, 0).unwrap(), before);
        let rebound = b.topology_snapshot(second, 0).unwrap();
        assert_eq!(before.elements.len(), rebound.elements.len());
        for (a, b) in before.elements.iter().zip(&rebound.elements) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.geometry, b.geometry);
            assert_eq!(
                a.origins.iter().map(|o| &o.role).collect::<Vec<_>>(),
                b.origins.iter().map(|o| &o.role).collect::<Vec<_>>()
            );
            assert_eq!(
                a.boundary.iter().map(|o| &o.role).collect::<Vec<_>>(),
                b.boundary.iter().map(|o| &o.role).collect::<Vec<_>>()
            );
            assert_eq!(
                a.frames
                    .iter()
                    .map(|f| (&f.origin.role, f.frame))
                    .collect::<Vec<_>>(),
                b.frames
                    .iter()
                    .map(|f| (&f.origin.role, f.frame))
                    .collect::<Vec<_>>()
            );
        }
        let independent = Solid::from_brep(&b.to_brep().unwrap(), policy).unwrap();
        for (a, b) in b.faces().unwrap().iter().zip(independent.faces().unwrap()) {
            assert_eq!(a.index, b.index);
            assert_eq!(a.plane, b.plane);
            assert!((a.area_mm2 - b.area_mm2).abs() < 1e-7 * (a.area_mm2 + b.area_mm2));
            assert!(
                a.center_mm
                    .into_iter()
                    .zip(b.center_mm)
                    .all(|(x, y)| (x - y).abs() < policy.linear_tolerance_mm)
            );
        }

        assert!(
            rebound
                .elements
                .iter()
                .flat_map(|e| e
                    .origins
                    .iter()
                    .chain(&e.boundary)
                    .chain(e.frames.iter().map(|f| &f.origin)))
                .all(|o| o.source == second)
        );
        assert!(
            rebound
                .elements
                .iter()
                .any(|e| e.origins.iter().any(|o| o.role.starts_with("screw.head/")))
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("whole-screw.step");
        b.write_step(&path).unwrap();
        let read = Solid::read_step(&path, policy).unwrap();
        // Match the established new-screw metrology policy: boundary error
        // is bounded by linear tolerance times measured surface area, plus
        // the unchanged native GK relative convergence budget for each solid.
        let volume_budget = policy.linear_tolerance_mm
            * (b.faces().unwrap().iter().map(|f| f.area_mm2).sum::<f64>()
                + read
                    .faces()
                    .unwrap()
                    .iter()
                    .map(|f| f.area_mm2)
                    .sum::<f64>())
            + 1e-7 * (b.volume_mm3().unwrap().abs() + read.volume_mm3().unwrap().abs());
        assert!((read.volume_mm3().unwrap() - b.volume_mm3().unwrap()).abs() < volume_budget);
        assert_eq!(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
        assert_eq!(a.bounds().unwrap(), b.bounds().unwrap());
        let uncached_path = directory.path().join("ordinary-first-build.step");
        a.write_step(&uncached_path).unwrap();
        let uncached_read = Solid::read_step(&uncached_path, policy).unwrap();
        assert_eq!(
            read.volume_mm3().unwrap(),
            uncached_read.volume_mm3().unwrap(),
            "Cached and ordinary first-build exports must retain identical geometry"
        );
        let detached = std::thread::spawn(|| {
            reset();
            assert_eq!(stats(), (0, 0, 0, 0, 0));
            let document = Document::new("Independent screw owner thread").unwrap();
            let values = parameters::evaluate(&document).unwrap();
            let recipe = source();
            let first =
                build(EntityId::new(), &recipe, &values, GeometryPolicy::default()).unwrap();
            let second =
                build(EntityId::new(), &recipe, &values, GeometryPolicy::default()).unwrap();
            assert_eq!(first.to_brep().unwrap(), second.to_brep().unwrap());
            let state = stats();
            (state.0, state.1, state.2, state.3)
        })
        .join()
        .unwrap();
        assert_eq!(detached, (2, 1, 1, 1));
        assert_eq!(stats().1, 1);
        assert!(stats().3 <= SCREW_FACTORY_ENTRIES && stats().4 <= SCREW_FACTORY_BYTES);
        let mut bad = recipe.clone();
        if let MechanicalPartParameters::MachineScrew { head, .. } = &mut bad.parameters {
            *head = model::ScrewHead::SocketCap {
                diameter: length(0.001),
                height: length(0.001),
            };
        }
        let stable = stats();
        assert!(build(EntityId::new(), &bad, &values, policy).is_err());
        assert_eq!(stats(), stable);
        assert_eq!(a.to_brep().unwrap(), geometry);
    }
    #[test]
    fn superseded_compilation_reuses_only_complete_geometry_without_publishing_document_edits() {
        use multicad_core::commands::{Actor, Channel, Session};
        reset();
        let mut session = Session::new(Document::new("Interrupted whole screw").unwrap()).unwrap();
        let request = model::MechanicalPartRequest::new("Screw", model::default_machine_screw());
        let prepared =
            model::prepare_mechanical_part(&session, 0, Actor::local(Channel::Mcp), &request)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        let source_before = serde_json::to_vec(session.document()).unwrap();
        let compiler = crate::Compiler::default();
        assert!(
            compiler
                .compile_interruptible(session.document(), session.revision(), Some(&|| true))
                .is_err()
        );
        assert_eq!(stats(), (0, 0, 0, 0, 0));
        // Once the provider starts, its native build is synchronous. A later
        // supersession may retain the completely checked immutable geometry,
        // but cannot publish a model or canonical document mutation.
        let interrupted = compiler.compile_interruptible(
            session.document(),
            session.revision(),
            Some(&|| stats().0 > 0),
        );
        assert!(interrupted.is_err());
        assert_eq!(stats().2, 1);
        assert_eq!(stats().3, 1);
        assert_eq!(
            serde_json::to_vec(session.document()).unwrap(),
            source_before
        );
        let values = parameters::evaluate(session.document()).unwrap();
        let source = model::read_mechanical_part(session.document(), request.feature).unwrap();
        let complete = build(EntityId::new(), &source, &values, GeometryPolicy::default()).unwrap();
        assert_eq!(stats().1, 1);
        assert!(complete.volume_mm3().unwrap() > 0.);
        assert_eq!(
            serde_json::to_vec(session.document()).unwrap(),
            source_before
        );
    }

    #[test]
    fn full_screw_factory_evicts_old_geometry_and_never_retains_failed_or_foreign_results() {
        reset();
        let document = Document::new("Bounded whole screw cache").unwrap();
        let values = parameters::evaluate(&document).unwrap();
        let policy = GeometryPolicy::default();
        let original = source();
        let owner = EntityId::new();
        let retained = build(owner, &original, &values, policy).unwrap();
        let geometry = retained.to_brep().unwrap();
        let foreign = retained
            .share()
            .unwrap()
            .with_occurrence_origin(EntityId::new(), EntityId::new())
            .unwrap();
        let key = screw_key(&original.parameters.resolve(&values).unwrap(), policy).unwrap();
        let before = stats();
        remember_screw(key.clone(), owner, &foreign);
        assert_eq!(stats(), before);
        for index in 1..=SCREW_FACTORY_ENTRIES {
            let mut recipe = original.clone();
            if let MechanicalPartParameters::MachineScrew {
                underhead_length, ..
            } = &mut recipe.parameters
            {
                *underhead_length = Some(length(10. + index as f64 * 0.1));
            }
            build(EntityId::new(), &recipe, &values, policy).unwrap();
            assert!(stats().3 <= SCREW_FACTORY_ENTRIES && stats().4 <= SCREW_FACTORY_BYTES);
        }
        assert!(SCREWS.with(|slot| !slot.borrow().entries.contains_key(&key)));
        assert_eq!(retained.to_brep().unwrap(), geometry);
        let hits = stats().1;
        build(EntityId::new(), &original, &values, policy).unwrap();
        assert_eq!(stats().1, hits);
    }

    #[test]
    fn exact_screw_key_covers_resolved_fields_pose_representation_and_policy() {
        let document = Document::new("Exact factory keys").unwrap();
        let values = parameters::evaluate(&document).unwrap();
        let resolved = source().parameters.resolve(&values).unwrap();
        let policy = GeometryPolicy::default();
        let base = screw_key(&resolved, policy).unwrap();
        let mut changed = resolved.clone();
        if let ResolvedMechanicalPart::MachineScrew { thread, .. } = &mut changed {
            thread.frame.origin[0] = -0.;
        }
        assert!(
            screw_key(&changed, policy).unwrap() != base,
            "Exact zero sign must not alias"
        );
        for field in 0..10 {
            let mut changed = resolved.clone();
            let ResolvedMechanicalPart::MachineScrew {
                underhead_length_mm,
                shank_radius_mm,
                head,
                drive,
                thread,
                ..
            } = &mut changed
            else {
                unreachable!()
            };
            let value = match field {
                0 => underhead_length_mm,
                1 => shank_radius_mm,
                2 => &mut thread.length_mm,
                3 => &mut thread.pitch_mm,
                4 => &mut thread.profile.depth_mm,
                5 => &mut thread.frame.origin[2],
                6 => &mut thread.frame.u[0],
                7 => match head {
                    ResolvedScrewHead::SocketCap { radius_mm, .. } => radius_mm,
                    _ => unreachable!(),
                },
                8 => match drive {
                    ResolvedScrewDrive::HexSocket { depth_mm, .. } => depth_mm,
                    _ => unreachable!(),
                },
                _ => &mut thread.minimum_wall_mm,
            };
            *value = f64::from_bits(value.to_bits() + 1);
            assert!(screw_key(&changed, policy).unwrap() != base);
        }
        let mut cosmetic = resolved.clone();
        if let ResolvedMechanicalPart::MachineScrew { thread, .. } = &mut cosmetic {
            thread.representation = ThreadRepresentation::CosmeticEnvelope;
        }
        assert!(screw_key(&cosmetic, policy).unwrap() != base);
        for field in 0..4 {
            let mut p = policy;
            let value = match field {
                0 => &mut p.linear_tolerance_mm,
                1 => &mut p.boolean_fuzzy_mm,
                2 => &mut p.minimum_feature_mm,
                _ => &mut p.maximum_coordinate_mm,
            };
            *value = f64::from_bits(value.to_bits() + 1);
            assert!(screw_key(&resolved, p).unwrap() != base);
        }
    }
}

#[cfg(test)]
mod screw_profile_shared_thread_tests {
    use super::*;
    use multicad_core::{
        document::Document,
        model::{self, BodyOutput, MechanicalPartParameters, ScrewDrive, ScrewHead, length},
        parameters,
    };
    #[test]
    fn new_screw_profile_booleans_and_step_preserve_shared_native_thread_factory_source() {
        SCREWS.with(|cache| *cache.borrow_mut() = ScrewFactory::default());
        let doc = Document::new("Shared thread source").unwrap();
        let evaluated = parameters::evaluate(&doc).unwrap();
        let mut p = model::default_machine_screw();
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            head,
            drive,
            thread,
            ..
        } = &mut p
        else {
            unreachable!()
        };
        *underhead_length = Some(length(6.));
        thread.length = length(3.);
        *head = ScrewHead::Pan {
            profile: Box::new(model::CrownedScrewHeadParameters {
                diameter: length(5.6),
                height: length(2.4),
                crown_radius: length(5.),
                top_land_diameter: length(4.1),
                neck_radius: length(0.1),
            }),
        };
        *drive = ScrewDrive::Hexalobular {
            profile: Box::new(model::HexalobularDriveParameters {
                major_diameter: length(2.8),
                minor_diameter: length(2.05),
                outer_lobe_radius: length(0.28),
                depth: length(1.1),
                bottom_angle: model::angle(120.),
            }),
        };
        let resolved = thread.resolve(&evaluated, ThreadMode::NewExternal).unwrap();
        let spec = multicad_kernel::HelicalThread {
            major_radius_mm: resolved.major_radius_mm,
            pitch_mm: resolved.pitch_mm,
            length_mm: resolved.length_mm,
            depth_mm: resolved.profile.depth_mm,
            crest_width_mm: resolved.profile.crest_width_mm,
            lower_flank_deg: resolved.profile.lower_flank_deg,
            upper_flank_deg: resolved.profile.upper_flank_deg,
            radial_taper: resolved.profile.radial_taper,
            starts: resolved.starts,
            left_hand: false,
        };
        let owner = EntityId::new();
        let policy = GeometryPolicy::default();
        let retained = Solid::helical_thread_named(spec, owner, policy).unwrap();
        let topology = retained.topology_snapshot(owner, 0).unwrap();
        let volume = retained.volume_mm3().unwrap();
        let bounds = retained.bounds().unwrap();
        let faces = retained.faces().unwrap();
        let bytes = retained.to_brep().unwrap();
        let before = multicad_kernel::thread_factory_stats().unwrap();
        let feature = EntityId::new();
        let source = MechanicalPartDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            parameters: p,
            provenance: None,
        };
        let completed = build(feature, &source, &evaluated, policy).unwrap();
        assert!(
            multicad_kernel::thread_factory_stats().unwrap().hits > before.hits,
            "Actual construction must exercise a shared thread factory hit"
        );
        assert_eq!(
            retained.to_brep().unwrap(),
            bytes,
            "Constructing a new head/recess must not modify the cached thread's native source"
        );
        let dir = tempfile::tempdir().unwrap();
        completed
            .write_step(&dir.path().join("profile.step"))
            .unwrap();
        assert_eq!(
            retained.to_brep().unwrap(),
            bytes,
            "Exporting the completed screw must not modify its shared thread source"
        );
        assert_eq!(retained.volume_mm3().unwrap(), volume);
        assert_eq!(retained.bounds().unwrap(), bounds);
        assert_eq!(retained.faces().unwrap(), faces);
        assert_eq!(retained.topology_snapshot(owner, 0).unwrap(), topology);
        let own = completed.topology_snapshot(feature, 0).unwrap();
        assert!(
            own.elements
                .iter()
                .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
                .all(|e| !e.origins.is_empty() && e.origins.iter().all(|o| o.source == feature))
        );
    }
}
