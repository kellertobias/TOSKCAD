use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, EvaluationCache};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn request(family: GearFamily) -> GearPairRequest {
    let gears = std::array::from_fn(|i| {
        GearRequest::new(
            if i == 0 { "Pinion" } else { "Wheel" },
            GearParameters {
                family,
                teeth: Dimension::Literal {
                    value: if i == 0 { 20. } else { 30. },
                    unit: "count".into(),
                },
                face_width: length(4.),
                helix_angle: angle(if family == GearFamily::Spur { 0. } else { 15. }),
                hand: if family == GearFamily::Spur {
                    GearHand::None
                } else if i == 0 {
                    GearHand::Left
                } else {
                    GearHand::Right
                },
                center_gap: length(if family == GearFamily::DoubleHelical {
                    1.
                } else {
                    0.
                }),
                frame: PrimitiveFrame {
                    origin: [length(10. * i as f64), length(20.), length(30.)],
                    u: [1., 0., 0.],
                    v: [0., 0., 1.],
                },
                ..Default::default()
            },
        )
    });
    let mut r = GearPairRequest::new("Native gear pair", gears);
    r.parameters.frame = PrimitiveFrame {
        origin: [length(100.), length(200.), length(300.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    r
}
#[test]
fn actual_native_pairs_cover_all_supported_families_and_only_two_exported_physical_leaves() {
    for family in [
        GearFamily::Spur,
        GearFamily::Helical,
        GearFamily::Herringbone,
        GearFamily::DoubleHelical,
    ] {
        let mut s = Session::new(Document::new("Native pair").unwrap()).unwrap();
        let r = request(family);
        let p = prepare_gear_pair(&s, 0, actor(), &r).unwrap();
        let candidate = compiler().compile(p.prepared.document(), 1).unwrap();
        let report = &candidate.gear_pair_checks()[&r.pair];
        assert_eq!(report.cycle_drive_angles_rad.len(), 9);
        assert!(report.current_pose_checked);
        assert!(!report.cycle_reused);
        assert_eq!(report.maximum_overlap_volume_mm3, 0.);
        assert_eq!(candidate.snapshot().leaf_occurrences.len(), 2);
        let dir = tempfile::tempdir().unwrap();
        for (i, path) in candidate.snapshot().leaf_occurrences.keys().enumerate() {
            assert_eq!(path.root, r.placement.instance);
            let leaf = candidate.leaf_solid(path).unwrap();
            assert_eq!(leaf.solid_count().unwrap(), 1);
            let bounds = leaf.bounds().unwrap();
            assert!((bounds.min[0] - 100.).abs() < 1e-5);
            assert!((bounds.max[0] - 104.).abs() < 1e-5);
            let expected_y = 200.
                + if path.members.contains(&r.outputs.instances[0]) {
                    0.
                } else {
                    report.center_distance_mm
                };
            assert!(((bounds.min[1] + bounds.max[1]) / 2. - expected_y).abs() < 1e-5);
            assert!(((bounds.min[2] + bounds.max[2]) / 2. - 300.).abs() < 1e-5);
            let output = dir.path().join(format!("member-{i}.step"));
            leaf.write_step(&output).unwrap();
            let readback = Solid::read_step(&output, GeometryPolicy::default()).unwrap();
            assert_eq!(readback.solid_count().unwrap(), 1);
            assert!((leaf.volume_mm3().unwrap() - readback.volume_mm3().unwrap()).abs() < 0.001);
        }
        s.commit(p.prepared).unwrap();
        assert_eq!(s.revision(), 1);
        assert_eq!(
            read_gear_pair(s.document(), r.pair).unwrap().outputs,
            r.outputs
        );
    }
}
#[test]
fn drive_and_recipe_edits_reuse_only_valid_cycle_proofs_and_reopen_preserves_placement() {
    let mut s = Session::new(Document::new("Cached native pair").unwrap()).unwrap();
    let r = request(GearFamily::Spur);
    let p = prepare_gear_pair(&s, 0, actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(!first.gear_pair_checks()[&r.pair].cycle_reused);
    let again = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(again.gear_pair_checks()[&r.pair].cycle_reused);
    assert!(cache.last_compile().evaluated.is_empty());
    let mut parameters = r.parameters.clone();
    parameters.drive_angle = angle(30.);
    let p = prepare_gear_pair_edit(&s, 1, actor(), r.pair, &parameters).unwrap();
    s.commit(p.prepared).unwrap();
    let driven = compiler()
        .compile_cached(&mut cache, s.document(), 2, None)
        .unwrap();
    assert!(driven.gear_pair_checks()[&r.pair].cycle_reused);
    assert!(driven.gear_pair_checks()[&r.pair].current_pose_checked);
    assert!(
        (driven.gear_pair_checks()[&r.pair].current_drive_angle_rad - 30_f64.to_radians()).abs()
            < 1e-12
    );
    assert!(cache.last_compile().evaluated.is_empty());
    let mut gear = r.gears[0].parameters.clone();
    gear.teeth = Dimension::Literal {
        value: 24.,
        unit: "count".into(),
    };
    let p = prepare_gear_edit(&s, 2, actor(), r.gears[0].feature, &gear).unwrap();
    s.commit(p.prepared).unwrap();
    let changed = compiler()
        .compile_cached(&mut cache, s.document(), 3, None)
        .unwrap();
    assert_eq!(changed.snapshot().leaf_occurrences.len(), 2);
    assert!(!changed.gear_pair_checks()[&r.pair].cycle_reused);
    assert_eq!(changed.gear_pair_checks()[&r.pair].center_distance_mm, 54.);
    assert!(cache.last_compile().evaluated.contains(&r.gears[0].feature));
    assert!(cache.last_compile().reused.contains(&r.gears[1].feature));
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let rebuilt = compiler().compile(&reopened.document, 0).unwrap();
    assert_eq!(
        rebuilt
            .snapshot()
            .leaf_occurrences
            .keys()
            .collect::<Vec<_>>(),
        changed
            .snapshot()
            .leaf_occurrences
            .keys()
            .collect::<Vec<_>>()
    );
    for path in changed.snapshot().leaf_occurrences.keys() {
        let a = changed.leaf_solid(path).unwrap().bounds().unwrap();
        let b = rebuilt.leaf_solid(path).unwrap().bounds().unwrap();
        for (x, y) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            assert!((x - y).abs() < 1e-7);
        }
    }
    cache.clear();
    let cleared = compiler()
        .compile_cached(&mut cache, s.document(), 3, None)
        .unwrap();
    assert!(!cleared.gear_pair_checks()[&r.pair].cycle_reused);
    let mut incompatible = gear;
    incompatible.module = length(3.);
    let p = prepare_gear_edit(&s, 3, actor(), r.gears[0].feature, &incompatible).unwrap();
    let failure = compiler()
        .compile(p.prepared.document(), 4)
        .err()
        .expect("incompatible pair must not compile");
    assert_eq!(failure.entity, Some(r.pair), "{failure:?}");
    assert!(failure.message.contains("matching family"));
    assert_eq!(s.revision(), 3);
}
