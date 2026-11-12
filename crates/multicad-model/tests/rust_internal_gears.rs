//! Rust internal ring and spur pinion pairs: compile, certify, edit, history
//! and saved files through the Rust geometry backend.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::*,
    project_file::{self, FileLimits, ProjectFile},
    references::TopologyKind,
};
use multicad_kernel::{RigidTransform, Solid, rust_kernel::spur_gear::clearance::ResultKind};
use multicad_model::{Compiler, GeometryBackend, Model};
use std::{
    collections::BTreeMap,
    f64::consts::{PI, TAU},
};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}

fn count(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "count".into(),
    }
}

fn ring(teeth: f64) -> GearParameters {
    GearParameters {
        family: GearFamily::InternalRing,
        module: length(1.),
        face_width: length(2.),
        teeth: count(teeth),
        bore_diameter: length(0.),
        outer_diameter: Some(length(teeth + 8.)),
        ..Default::default()
    }
}

fn pinion() -> GearParameters {
    GearParameters {
        module: length(1.),
        face_width: length(2.),
        ..Default::default()
    }
}

fn request(teeth: f64, ring_first: bool) -> GearPairRequest {
    let ring = GearRequest::new("Ring", ring(teeth));
    let pinion = GearRequest::new("Pinion", pinion());
    GearPairRequest::new(
        "Internal pair",
        if ring_first {
            [ring, pinion]
        } else {
            [pinion, ring]
        },
    )
}

fn rust(document: &Document, revision: u64) -> Result<Model, multicad_model::ModelError> {
    compiler().compile_with_backend(document, revision, GeometryBackend::Rust, None)
}

fn roles(solid: &Solid, owner: EntityId) -> BTreeMap<String, usize> {
    let mut roles = BTreeMap::new();
    for element in solid.topology_snapshot(owner, 0).unwrap().elements {
        if element.key.kind == TopologyKind::Face {
            for origin in element.origins {
                *roles.entry(origin.role).or_insert(0) += 1;
            }
        }
    }
    roles
}

/// Spin a placed gear about its own axis, through its start cap center.
fn spun(solid: &Solid, angle: f64) -> Solid {
    let start = solid
        .faces()
        .unwrap()
        .into_iter()
        .find(|face| face.index == 0)
        .unwrap();
    let (point, [x, y, z]) = start.plane.unwrap();
    let (s, c) = angle.sin_cos();
    let t = 1. - c;
    let rotation = [
        [c + x * x * t, x * y * t - z * s, x * z * t + y * s],
        [y * x * t + z * s, c + y * y * t, y * z * t - x * s],
        [z * x * t - y * s, z * y * t + x * s, c + z * z * t],
    ];
    solid
        .transformed(RigidTransform {
            rotation,
            translation_mm: std::array::from_fn(|i| {
                point[i] - (0..3).map(|j| rotation[i][j] * point[j]).sum::<f64>()
            }),
        })
        .unwrap()
}

#[test]
fn rust_ring_pairs_certify_both_member_orders_and_match_native_members() {
    for teeth in [48., 60.] {
        for ring_first in [true, false] {
            let session = Session::new(Document::new("Rust internal pair").unwrap()).unwrap();
            let request = request(teeth, ring_first);
            let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
            let document = prepared.prepared.document();
            let model = rust(document, 1).unwrap();
            let report = &model.gear_pair_checks()[&request.pair];
            assert_eq!(report.cycle_drive_angles_rad.len(), 9);
            assert!(report.current_pose_checked);
            assert_eq!(report.maximum_overlap_volume_mm3, 0.);
            assert_eq!(report.center_distance_mm, (teeth - 24.) / 2.);
            assert!(report.scope.contains("internal ring"), "{}", report.scope);
            let ring_index = usize::from(!ring_first);
            let ring_instance = request.outputs.instances[ring_index];
            let pinion_instance = request.outputs.instances[1 - ring_index];
            let [ring_solid, pinion_solid] =
                [ring_instance, pinion_instance].map(|i| model.solid(i).unwrap());
            for solid in [ring_solid, pinion_solid] {
                assert!(solid.uses_rust_geometry());
                assert_eq!(solid.solid_count().unwrap(), 1);
            }
            let ring_roles = roles(ring_solid, ring_instance);
            assert!(ring_roles.contains_key("gear/rim.outer"));
            assert_eq!(
                ring_roles.len(),
                3 + 6 * teeth as usize,
                "caps, rim and six roles per tooth"
            );
            // The placed pair certifies at its current pose; a half-pitch
            // pinion turn or a pinion pushed outward does not.
            let policy = compiler().policy;
            assert_eq!(
                ring_solid
                    .certify_rust_internal_pair(pinion_solid, policy, None)
                    .unwrap(),
                ResultKind::Clear
            );
            let half = spun(pinion_solid, PI / 24.);
            assert_eq!(
                ring_solid
                    .certify_rust_internal_pair(&half, policy, None)
                    .unwrap(),
                ResultKind::CommonInterior
            );
            // Push the pinion 0.4 mm outward along the line of centers.
            let axis_point = |solid: &Solid| solid.faces().unwrap()[0].plane.unwrap().0;
            let [a, b] = [axis_point(ring_solid), axis_point(pinion_solid)];
            let distance = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
            assert!((distance - report.center_distance_mm).abs() < 1e-9);
            let pushed = pinion_solid
                .transformed(RigidTransform {
                    translation_mm: std::array::from_fn(|i| 0.4 * (b[i] - a[i]) / distance),
                    ..Default::default()
                })
                .unwrap();
            assert_ne!(
                ring_solid
                    .certify_rust_internal_pair(&pushed, policy, None)
                    .unwrap(),
                ResultKind::Clear
            );
            if multicad_kernel::OCCT_LINKED {
                let native = compiler()
                    .compile_with_backend(document, 1, GeometryBackend::OpenCascade, None)
                    .unwrap();
                let oracle = &native.gear_pair_checks()[&request.pair];
                assert_eq!(oracle.cycle_drive_angles_rad, report.cycle_drive_angles_rad);
                assert_eq!(oracle.center_distance_mm, report.center_distance_mm);
                for instance in [ring_instance, pinion_instance] {
                    let [r, n] = [&model, &native].map(|m| m.solid(instance).unwrap());
                    let [vr, vn] = [r, n].map(|s| s.volume_mm3().unwrap());
                    assert!((vr - vn).abs() < 1e-6 * vn, "{vr} {vn}");
                    let [br, bn] = [r, n].map(|s| s.bounds().unwrap());
                    for i in 0..3 {
                        assert!((br.min[i] - bn.min[i]).abs() < 1e-5);
                        assert!((br.max[i] - bn.max[i]).abs() < 1e-5);
                    }
                    assert_eq!(roles(r, instance), roles(n, instance));
                }
                println!(
                    "{{\"case\":\"model internal pair\",\"ring_teeth\":{teeth},\"ring_first\":{ring_first},\"rust_cycle_ms\":{},\"rust_current_ms\":{},\"native_cycle_ms\":{},\"native_current_ms\":{}}}",
                    report.sampled_cycle_ms,
                    report.current_pose_ms,
                    oracle.sampled_cycle_ms,
                    oracle.current_pose_ms
                );
            }
        }
    }
}

#[test]
fn rust_ring_edits_history_failed_edits_and_reopen_keep_the_previous_model() {
    let mut session = Session::new(Document::new("Rust internal history").unwrap()).unwrap();
    let request = request(48., false);
    let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = session.document().clone();
    let ring_feature = request.gears[1].feature;
    let first = rust(session.document(), session.revision()).unwrap();
    let first_volume = first.solid(ring_feature).unwrap().volume_mm3().unwrap();

    // Drive the pair, then edit the ring teeth and rim together.
    let mut drive = request.parameters.clone();
    drive.drive_angle = angle(17.);
    let p = prepare_gear_pair_edit(&session, session.revision(), actor(), request.pair, &drive)
        .unwrap();
    session.commit(p.prepared).unwrap();
    let driven = rust(session.document(), session.revision()).unwrap();
    let report = &driven.gear_pair_checks()[&request.pair];
    assert!((report.current_drive_angle_rad - 17_f64.to_radians()).abs() < 1e-12);
    let p = prepare_gear_edit(
        &session,
        session.revision(),
        actor(),
        ring_feature,
        &ring(60.),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let edited = rust(session.document(), session.revision()).unwrap();
    assert_eq!(
        edited.gear_pair_checks()[&request.pair].center_distance_mm,
        18.
    );
    let edited_volume = edited.solid(ring_feature).unwrap().volume_mm3().unwrap();
    assert!(edited_volume > first_volume);
    let saved = session.document().clone();
    let revision = session.revision();

    // Zero thinning leaves exact conjugate contact; Rust refuses the edit
    // and the session keeps the previous document and model.
    let mut gears = [pinion(), ring(60.)];
    for gear in &mut gears {
        gear.backlash = length(0.);
        gear.fabrication_allowance = length(0.);
    }
    let candidate =
        prepare_gear_pair_geometry_edit(&session, revision, actor(), request.pair, &drive, &gears)
            .unwrap();
    let Err(failure) = rust(candidate.prepared.document(), revision + 1) else {
        panic!("zero-thinning internal pair must not be certified");
    };
    assert!(
        failure.to_string().contains("certified positive clearance")
            || failure.to_string().contains("overlaps"),
        "{failure}"
    );
    // A ring too small for the pinion is refused before any geometry.
    assert!(prepare_gear_edit(&session, revision, actor(), ring_feature, &ring(30.)).is_err());
    assert_eq!(session.revision(), revision);
    assert_eq!(session.document(), &saved);
    let retained = rust(session.document(), revision).unwrap();
    assert!(
        (retained.solid(ring_feature).unwrap().volume_mm3().unwrap() - edited_volume).abs() < 1e-9
    );

    session.undo(session.revision(), actor()).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    let undone = rust(session.document(), session.revision()).unwrap();
    assert!(
        (undone.solid(ring_feature).unwrap().volume_mm3().unwrap() - first_volume).abs() < 1e-9
    );
    session.redo(session.revision(), actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &saved);

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, saved);
    let readback = rust(&reopened.project.document, 0).unwrap();
    let proof = &readback.gear_pair_checks()[&request.pair];
    assert_eq!(proof.center_distance_mm, 18.);
    assert_eq!(
        proof.cycle_drive_angles_rad,
        (0..9)
            .map(|i| TAU / 24. * f64::from(i) / 8.)
            .collect::<Vec<_>>()
    );
    assert!(
        (readback.solid(ring_feature).unwrap().volume_mm3().unwrap() - edited_volume).abs() < 1e-9
    );
}
