use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, EvaluationCache};
use std::f64::consts::PI;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn request(teeth: f64, ring_first: bool) -> GearPairRequest {
    let count = |value| Dimension::Literal {
        value,
        unit: "count".into(),
    };
    let shared = GearParameters {
        module: length(1.),
        face_width: length(2.),
        ..Default::default()
    };
    let ring = GearRequest::new(
        "Ring",
        GearParameters {
            family: GearFamily::InternalRing,
            teeth: count(teeth),
            bore_diameter: length(0.),
            outer_diameter: Some(length(teeth + 8.)),
            ..shared.clone()
        },
    );
    let pinion = GearRequest::new("Pinion", shared);
    GearPairRequest::new(
        "Internal pair",
        if ring_first {
            [ring, pinion]
        } else {
            [pinion, ring]
        },
    )
}
fn placed(s: &Solid, phase: f64, x: f64) -> Solid {
    s.transformed(RigidTransform {
        rotation: [
            [phase.cos(), -phase.sin(), 0.],
            [phase.sin(), phase.cos(), 0.],
            [0., 0., 1.],
        ],
        translation_mm: [x, 0., 0.],
    })
    .unwrap()
}
#[test]
fn both_ring_sizes_and_member_orders_have_real_sampled_clearance_step_and_bad_phase_controls() {
    for teeth in [48., 60.] {
        for ring_first in [false, true] {
            let mut s = Session::new(Document::new("Native internal mating").unwrap()).unwrap();
            let req = request(teeth, ring_first);
            let before = s.document().clone();
            let prepared = prepare_gear_pair(&s, 0, actor(), &req).unwrap();
            let native = compiler().compile(prepared.prepared.document(), 1).unwrap();
            assert_eq!(s.document(), &before);
            assert_eq!(s.revision(), 0);
            let report = &native.gear_pair_checks()[&req.pair];
            assert_eq!(report.cycle_drive_angles_rad.len(), 9);
            assert!(report.current_pose_checked);
            assert_eq!(report.maximum_overlap_volume_mm3, 0.);
            assert_eq!(report.center_distance_mm, (teeth - 24.) / 2.);
            let resolved = req
                .parameters
                .resolve_recipes(
                    [&req.gears[0].parameters, &req.gears[1].parameters],
                    s.parameters(),
                )
                .unwrap();
            let a = native.solid(req.gears[0].feature).unwrap();
            let b = native.solid(req.gears[1].feature).unwrap();
            let a = placed(
                a,
                resolved.phases_rad[0] + PI / f64::from(resolved.gears[0].teeth()),
                0.,
            );
            let b = placed(b, resolved.phases_rad[1], resolved.center_distance_mm);
            let wrong = a
                .boolean(&b, BooleanOperation::Intersect, GeometryPolicy::default())
                .unwrap()
                .expect("half-pitch misphase must really collide");
            assert!(wrong.volume_mm3().unwrap() > 0.01);
            let dir = tempfile::tempdir().unwrap();
            assert_eq!(native.snapshot().leaf_occurrences.len(), 2);
            for (i, path) in native.snapshot().leaf_occurrences.keys().enumerate() {
                let solid = native.leaf_solid(path).unwrap();
                assert_eq!(solid.solid_count().unwrap(), 1);
                let step = dir.path().join(format!("member{i}.step"));
                solid.write_step(&step).unwrap();
                let back = Solid::read_step(&step, GeometryPolicy::default()).unwrap();
                assert!((back.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-4);
            }
            s.commit(prepared.prepared).unwrap();
            assert_eq!(
                read_gear_pair(s.document(), req.pair).unwrap().outputs,
                req.outputs
            );
        }
    }
}
#[test]
fn ring_geometry_edits_drive_history_and_independent_reopen_keep_pair_identities() {
    let mut s = Session::new(Document::new("Internal history").unwrap()).unwrap();
    let req = request(48., false);
    let p = prepare_gear_pair(&s, 0, actor(), &req).unwrap();
    s.commit(p.prepared).unwrap();
    let original = s.document().clone();
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    let mut drive = req.parameters.clone();
    drive.drive_angle = angle(17.);
    let p = prepare_gear_pair_edit(&s, s.revision(), actor(), req.pair, &drive).unwrap();
    s.commit(p.prepared).unwrap();
    let driven = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(driven.gear_pair_checks()[&req.pair].cycle_reused);
    assert!(cache.last_compile().evaluated.is_empty());
    let mut ring = req.gears[1].parameters.clone();
    ring.teeth = Dimension::Literal {
        value: 60.,
        unit: "count".into(),
    };
    ring.outer_diameter = Some(length(68.));
    let p = prepare_gear_edit(&s, s.revision(), actor(), req.gears[1].feature, &ring).unwrap();
    s.commit(p.prepared).unwrap();
    let edited = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(!edited.gear_pair_checks()[&req.pair].cycle_reused);
    assert_eq!(edited.gear_pair_checks()[&req.pair].center_distance_mm, 18.);
    assert!(
        cache
            .last_compile()
            .evaluated
            .contains(&req.gears[1].feature)
    );
    assert!(cache.last_compile().reused.contains(&req.gears[0].feature));
    assert_ne!(
        first
            .solid(req.gears[1].feature)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        edited
            .solid(req.gears[1].feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
    );
    let saved = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, saved);
    let native = compiler()
        .compile(&reopened.project.document, s.revision())
        .unwrap();
    assert_eq!(native.gear_pair_checks()[&req.pair].center_distance_mm, 18.);
    assert_eq!(
        read_gear_pair(&reopened.project.document, req.pair)
            .unwrap()
            .outputs,
        req.outputs
    );
}
