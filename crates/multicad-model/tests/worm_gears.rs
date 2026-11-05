//! Actual ZA screw and generated-wheel bodies, canonical poses and history.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BRepSelection, GeometryPolicy, Solid};
use multicad_model::{Compiler, EvaluationCache};
use std::f64::consts::{PI, TAU};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn count(n: f64) -> Dimension {
    Dimension::Literal {
        value: n,
        unit: "count".into(),
    }
}
fn recipes(wheel_first: bool) -> [GearParameters; 2] {
    std::array::from_fn(|i| {
        let wheel = (i == 0) == wheel_first;
        GearParameters {
            family: GearFamily::Worm,
            worm: Some(WormDimensions {
                role: if wheel {
                    WormMember::Wheel
                } else {
                    WormMember::Worm
                },
                tooth_system: WormToothSystem::ZaAxialStraight,
                mate_teeth: count(if wheel { 1. } else { 32. }),
                diameter_quotient: ratio(10.),
            }),
            module_system: GearModuleSystem::Axial,
            module: length(1.),
            teeth: count(if wheel { 32. } else { 1. }),
            hand: GearHand::Right,
            face_width: length(if wheel { 2. } else { 16. }),
            bore_diameter: length(if wheel { 4. } else { 2. }),
            root_fillet_ratio: ratio(0.),
            backlash: length(0.03),
            fabrication_allowance: length(0.02),
            // Definition-local frames differ from the common pair frame.
            // Actual occurrences must remove these before mating the axes.
            frame: PrimitiveFrame {
                origin: [length(i as f64 * 7.), length(3.), length(-4.)],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            ..Default::default()
        }
    })
}
fn request(wheel_first: bool) -> GearPairRequest {
    let mut r = GearPairRequest::new(
        "ZA generated pair",
        recipes(wheel_first).map(|p| GearRequest::new("ZA member", p)),
    );
    r.parameters.frame = PrimitiveFrame {
        origin: [length(30.), length(40.), length(50.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    r
}
fn near(a: f64, b: f64, budget: f64) {
    assert!((a - b).abs() <= budget, "{a} != {b}, budget {budget}");
}
fn volume_budget(s: &Solid) -> f64 {
    let p = GeometryPolicy::default();
    // Existing native linear-policy surface envelope plus unchanged relative
    // adaptive quadrature acceptance, not an invented fixed STEP epsilon.
    p.linear_tolerance_mm * s.faces().unwrap().iter().map(|f| f.area_mm2).sum::<f64>()
        + 1e-7 * s.volume_mm3().unwrap().abs()
}
fn expected_worm_volume() -> f64 {
    // Independent radial integration of axial trapezoid occupancy. The
    // constant signed-lead rotation preserves this area for fractional turns.
    let k = 20_f64.to_radians().tan() * 1.01_f64.sqrt();
    let crest = PI / 2. - 0.05 - 2. * k;
    let integral =
        (crest + 12. * k) * (36. - 3.75_f64.powi(2)) / 2. - 2. * k * (216. - 3.75_f64.powi(3)) / 3.;
    16. * (PI * 3.75_f64.powi(2) + 2. * integral - PI)
}
#[test]
fn native_generated_pair_has_actual_centered_bodies_world_axes_and_step_evidence() {
    let session = Session::new(Document::new("Generated ZA candidate").unwrap()).unwrap();
    let request = request(false);
    let before = session.document().clone();
    let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    let built = compiler().compile(prepared.prepared.document(), 1).unwrap();
    assert_eq!(session.document(), &before);
    let check = &built.gear_pair_checks()[&request.pair];
    let worm = check.worm.as_ref().expect("named worm-pair context");
    assert_eq!(worm.worm_member, GearPairMember::A);
    assert_eq!(worm.wheel_member, GearPairMember::B);
    near(check.center_distance_mm, 21., 1e-12);
    near(worm.shaft_angle_rad, PI / 2., 1e-12);
    assert_eq!(check.cycle_drive_angles_rad.len(), 9);
    near(check.cycle_drive_angles_rad[0], 0., 1e-12);
    near(check.cycle_drive_angles_rad[8], TAU, 1e-12);
    assert!(check.current_pose_checked && !check.cycle_reused);
    let engagement = check
        .current_engagement
        .as_ref()
        .expect("actual native engagement");
    assert!(engagement.distance_mm <= 0.1 + GeometryPolicy::default().linear_tolerance_mm * 10.);
    near(
        engagement.points_mm[0]
            .iter()
            .zip(engagement.points_mm[1])
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt(),
        engagement.distance_mm,
        1e-6,
    );
    assert!(check.scope.contains("full ZA worm turn"));
    assert_eq!(built.snapshot().leaf_occurrences.len(), 2);
    let dir = tempfile::tempdir().unwrap();
    for (i, member) in request.gears.iter().enumerate() {
        let solid = built.solid(member.feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let expected = if i == 0 {
            expected_worm_volume()
        } else {
            // Independent exact-source Green-boundary integration. The stored
            // artifact uses bore radius2/diameter4, not diameter2.
            1563.90212973608
        };
        near(solid.volume_mm3().unwrap(), expected, volume_budget(solid));
        let topology = solid.topology_snapshot(member.feature, 1).unwrap();
        let prefix = if i == 0 { "worm/" } else { "worm_wheel/" };
        assert!(topology.elements.iter().any(|e| {
            e.origins
                .iter()
                .any(|o| o.source == member.feature && o.role.starts_with(prefix))
        }));
        assert!(topology.elements.iter().all(|e| !e.origins.is_empty()));
        let (path, _) = built
            .snapshot()
            .leaf_occurrences
            .iter()
            .find(|(_, leaf)| leaf.producer == member.feature)
            .unwrap();
        assert_eq!(path.root, request.placement.instance);
        let placed = built.leaf_solid(path).unwrap();
        let bounds = placed.bounds().unwrap();
        if i == 0 {
            // Pair frame maps worm axis to +X; source caps are centered.
            near(bounds.min[0], 22., 2e-6);
            near(bounds.max[0], 38., 2e-6);
        } else {
            // Wheel axis is base+V, world+Z. Center is base−21*U.
            near(bounds.min[2], 49., 2e-6);
            near(bounds.max[2], 51., 2e-6);
            near((bounds.min[1] + bounds.max[1]) / 2., 19., 2e-5);
        }
        let file = dir.path().join(format!("za-{i}.step"));
        placed.write_step(&file).unwrap();
        let opened = Solid::read_step(&file, GeometryPolicy::default()).unwrap();
        assert_eq!(opened.solid_count().unwrap(), 1);
        near(
            opened.volume_mm3().unwrap(),
            placed.volume_mm3().unwrap(),
            volume_budget(placed) + volume_budget(&opened),
        );
        let read_bounds = opened.bounds().unwrap();
        for axis in 0..3 {
            near(bounds.min[axis], read_bounds.min[axis], 2e-5);
            near(bounds.max[axis], read_bounds.max[axis], 2e-5);
        }
    }
    let leaves = built
        .snapshot()
        .leaf_occurrences
        .keys()
        .map(|p| built.leaf_solid(p).unwrap())
        .collect::<Vec<_>>();
    assert!(
        leaves[0]
            .minimum_distance(BRepSelection::Whole, leaves[1], BRepSelection::Whole)
            .unwrap()
            .distance_mm
            <= 0.100001
    );
}
#[test]
fn wheel_first_drive_reuses_actual_geometry_and_atomic_edit_history_reopen() {
    let mut session = Session::new(Document::new("ZA lifecycle").unwrap()).unwrap();
    let request = request(true);
    session
        .commit(
            prepare_gear_pair(&session, 0, actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, session.document(), 1, None)
        .unwrap();
    let mut pair = request.parameters.clone();
    pair.drive_angle = angle(37.);
    session
        .commit(
            prepare_gear_pair_edit(&session, 1, actor(), request.pair, &pair)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let driven = compiler()
        .compile_cached(&mut cache, session.document(), 2, None)
        .unwrap();
    let check = &driven.gear_pair_checks()[&request.pair];
    assert!(check.cycle_reused && check.current_pose_checked);
    assert_eq!(check.worm.as_ref().unwrap().worm_member, GearPairMember::B);
    near(check.cycle_drive_angles_rad[8], TAU, 1e-12);
    assert!(cache.last_compile().evaluated.is_empty());
    for member in &request.gears {
        assert!(cache.last_compile().reused.contains(&member.feature));
    }
    let before = session.document().clone();
    let recipes = recipes(true).map(|mut p| {
        p.module = length(2.);
        p.face_width = length(if p.worm.as_ref().unwrap().role == WormMember::Worm {
            32.
        } else {
            4.
        });
        p.bore_diameter = length(if p.worm.as_ref().unwrap().role == WormMember::Worm {
            4.
        } else {
            8.
        });
        p.backlash = length(0.06);
        p.fabrication_allowance = length(0.04);
        p
    });
    let recipes = couple_worm_pair_recipes(&recipes).unwrap();
    assert!(
        prepare_gear_edit(&session, 2, actor(), request.gears[0].feature, &recipes[0]).is_err()
    );
    let prepared =
        prepare_gear_pair_geometry_edit(&session, 2, actor(), request.pair, &pair, &recipes)
            .unwrap();
    let edited = compiler()
        .compile_cached(&mut cache, prepared.prepared.document(), 3, None)
        .unwrap();
    assert_eq!(session.document(), &before);
    assert!(!edited.gear_pair_checks()[&request.pair].cycle_reused);
    near(
        edited.gear_pair_checks()[&request.pair].center_distance_mm,
        42.,
        1e-12,
    );
    for member in &request.gears {
        assert!(cache.last_compile().evaluated.contains(&member.feature));
        near(
            edited.solid(member.feature).unwrap().volume_mm3().unwrap(),
            first.solid(member.feature).unwrap().volume_mm3().unwrap() * 8.,
            volume_budget(edited.solid(member.feature).unwrap()) * 2.,
        );
    }
    session.commit(prepared.prepared).unwrap();
    let committed = session.document().clone();
    assert_eq!(
        read_gear_pair(&committed, request.pair).unwrap().outputs,
        request.outputs
    );
    session.undo(3, actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(4, actor()).unwrap();
    assert_eq!(session.document(), &committed);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(committed.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, committed);
    let fresh = compiler().compile(&reopened.document, 0).unwrap();
    assert_eq!(fresh.snapshot().leaf_occurrences.len(), 2);
    assert!(!fresh.gear_pair_checks()[&request.pair].cycle_reused);
    for member in &request.gears {
        near(
            fresh.solid(member.feature).unwrap().volume_mm3().unwrap(),
            edited.solid(member.feature).unwrap().volume_mm3().unwrap(),
            volume_budget(fresh.solid(member.feature).unwrap()) * 2.,
        );
    }
}

#[test]
fn zero_thinning_pair_preserves_strict_native_no_overlap_over_full_turn() {
    let session = Session::new(Document::new("Zero thinning ZA pair").unwrap()).unwrap();
    let mut parameters = recipes(false);
    for recipe in &mut parameters {
        recipe.backlash = length(0.);
        recipe.fabrication_allowance = length(0.);
    }
    let request = GearPairRequest::new(
        "Zero thinning pair",
        parameters.map(|p| GearRequest::new("Member", p)),
    );
    let before = session.document().clone();
    let candidate = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    let native = compiler()
        .compile(candidate.prepared.document(), 1)
        .unwrap();
    assert_eq!(session.document(), &before);
    let check = &native.gear_pair_checks()[&request.pair];
    assert_eq!(check.cycle_drive_angles_rad.len(), 9);
    near(check.cycle_drive_angles_rad[8], TAU, 1e-12);
    assert!(check.current_pose_checked);
    assert_eq!(check.maximum_overlap_volume_mm3, 0.);
    assert!(
        check.current_engagement.as_ref().unwrap().distance_mm
            <= GeometryPolicy::default().linear_tolerance_mm * 10.
    );
}
