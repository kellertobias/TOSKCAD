//! Independent KHK scalar/Green-area expectations, actual native pair checks,
//! retained pitch/hub intent and source-safe cache/history behavior.
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
fn fixture(teeth: [u32; 2], shifts: [f64; 2], module: f64) -> GearPairRequest {
    let bore = if module == 3. { 4. } else { 2. };
    let hub = if module == 3. { 16. } else { 8. };
    let mut parameters = std::array::from_fn(|i| GearParameters {
        pitch_input: GearPitchInput::DiametralPitch,
        module: Dimension::Literal {
            value: 25.4 / module,
            unit: "teeth_per_in".into(),
        },
        teeth: Dimension::Literal {
            value: f64::from(teeth[i]),
            unit: "count".into(),
        },
        profile_shift: ratio(shifts[i]),
        face_width: length(4.),
        bore_diameter: length(bore),
        backlash: length(if module == 3. { 0.05 } else { 0.02 }),
        root_fillet_ratio: ratio(0.2),
        minimum_wall: length(0.5),
        hub: Some(GearHub {
            diameter: length(hub),
            axial_start: length(-2.),
            axial_end: length(6.),
        }),
        frame: PrimitiveFrame {
            origin: [length(10. * i as f64), length(20.), length(30.)],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        ..Default::default()
    });
    couple_shifted_spur_pair_recipes(&mut parameters).unwrap();
    let mut pair = GearPairRequest::new(
        "Shifted DP pair",
        parameters.map(|p| GearRequest::new("Hub gear", p)),
    );
    pair.parameters.frame = PrimitiveFrame {
        origin: [length(100.), length(200.), length(300.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    pair
}

#[test]
fn independently_sourced_positive_and_negative_shift_pairs_have_native_engagement_and_step_solids()
{
    // Expectations come from an independent exact-boundary Green integration
    // of line/circle/involute sections, not from production tessellation.
    // Hub additions are ONLY the external annulus; the overlapped core is not counted twice.
    for (teeth, shifts, module, center, volumes) in [
        (
            [12, 24],
            [0.6, 0.36],
            3.,
            56.49986972030518,
            [5373.1703714477, 17614.44941520163],
        ),
        (
            [24, 48],
            [-0.2, 0.2],
            1.,
            36.,
            [1881.6856280847285, 7481.41652754775],
        ),
        (
            [36, 48],
            [-0.15, -0.1],
            1.,
            41.743978573752464,
            [4127.922298400048, 7295.2268945622145],
        ),
    ] {
        let mut s = Session::new(Document::new("Sourced shifted pair").unwrap()).unwrap();
        let request = fixture(teeth, shifts, module);
        let p = prepare_gear_pair(&s, 0, actor(), &request).unwrap();
        let candidate = compiler().compile(p.prepared.document(), 1).unwrap();
        let report = &candidate.gear_pair_checks()[&request.pair];
        assert!((report.center_distance_mm - center).abs() < 1e-9);
        assert_eq!(report.cycle_drive_angles_rad.len(), 9);
        assert_eq!(report.maximum_overlap_volume_mm3, 0.);
        assert!(report.current_pose_checked);
        let witness = report
            .current_engagement
            .as_ref()
            .expect("Native proximity, not separated solids");
        assert!(matches!(
            witness.kind,
            "shifted_spur_flank_faces" | "whole_solid_fallback"
        ));
        for (i, gear) in request.gears.iter().enumerate() {
            let solid = candidate.solid(gear.feature).unwrap();
            assert_eq!(solid.solid_count().unwrap(), 1);
            assert!(
                (solid.volume_mm3().unwrap() - volumes[i]).abs() < 0.005,
                "{teeth:?} member{i}"
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("shifted-hub.step");
            solid.write_step(&path).unwrap();
            let read = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            assert_eq!(read.solid_count().unwrap(), 1);
            assert!((read.volume_mm3().unwrap() - volumes[i]).abs() < 0.005);
        }
        s.commit(p.prepared).unwrap();
        let old = s.document().clone();
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(old.clone()).unwrap(),
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
        // Verify the exact retained recipes independently of application view state.
        for gear in &request.gears {
            assert_eq!(
                read_gear(&reopened.document, gear.feature)
                    .unwrap()
                    .parameters,
                gear.parameters
            );
        }
        assert_eq!(s.document(), &old);
    }
}

#[test]
fn each_retained_hub_dimension_invalidates_cycle_proof_while_drive_preserves_both_recipes() {
    let mut s = Session::new(Document::new("Shifted hub cache").unwrap()).unwrap();
    let request = fixture([24, 48], [-0.2, 0.2], 1.);
    let p = prepare_gear_pair(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(!first.gear_pair_checks()[&request.pair].cycle_reused);
    let recipes = request
        .gears
        .each_ref()
        .map(|gear| read_gear(s.document(), gear.feature).unwrap().parameters);
    let mut pair = request.parameters.clone();
    pair.drive_angle = angle(37.);
    let p = prepare_gear_pair_edit(&s, 1, actor(), request.pair, &pair).unwrap();
    s.commit(p.prepared).unwrap();
    let driven = compiler()
        .compile_cached(&mut cache, s.document(), 2, None)
        .unwrap();
    assert!(driven.gear_pair_checks()[&request.pair].cycle_reused);
    assert!(driven.gear_pair_checks()[&request.pair].current_pose_checked);
    assert!(cache.last_compile().evaluated.is_empty());
    for (i, gear) in request.gears.iter().enumerate() {
        assert_eq!(
            read_gear(s.document(), gear.feature).unwrap().parameters,
            recipes[i]
        );
    }
    let mut changed = recipes;
    for field in 0..3 {
        let hub = changed[0].hub.as_mut().unwrap();
        match field {
            0 => hub.diameter = length(9.),
            1 => hub.axial_start = length(-3.),
            _ => hub.axial_end = length(7.),
        }
        let before = s.document().clone();
        assert!(
            prepare_gear_edit(
                &s,
                s.revision(),
                actor(),
                request.gears[0].feature,
                &changed[0]
            )
            .is_err()
        );
        assert_eq!(s.document(), &before);
        let p = prepare_gear_pair_geometry_edit(
            &s,
            s.revision(),
            actor(),
            request.pair,
            &pair,
            &changed,
        )
        .unwrap();
        s.commit(p.prepared).unwrap();
        let native = compiler()
            .compile_cached(&mut cache, s.document(), s.revision(), None)
            .unwrap();
        assert!(
            !native.gear_pair_checks()[&request.pair].cycle_reused,
            "hub field{field}"
        );
        assert!(native.gear_pair_checks()[&request.pair].current_pose_checked);
        assert!(
            cache
                .last_compile()
                .evaluated
                .contains(&request.gears[0].feature)
        );
        assert!(
            cache
                .last_compile()
                .reused
                .contains(&request.gears[1].feature)
        );
        let after = s.document().clone();
        s.undo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &after);
    }
}

#[test]
fn adding_zero_shift_matched_intent_requires_a_new_engagement_cycle_proof() {
    let mut s = Session::new(Document::new("Proof mode transition").unwrap()).unwrap();
    let request = fixture([24, 48], [0., 0.], 1.);
    assert!(
        request
            .gears
            .iter()
            .all(|g| g.parameters.shift_mate.is_none())
    );
    let p = prepare_gear_pair(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let mut cache = EvaluationCache::default();
    let before = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(
        before.gear_pair_checks()[&request.pair]
            .current_engagement
            .is_none()
    );
    let mut parameters = request.gears.each_ref().map(|g| g.parameters.clone());
    parameters[0].shift_mate = Some(ShiftedSpurMate {
        teeth: parameters[1].teeth.clone(),
        profile_shift: parameters[1].profile_shift.clone(),
    });
    couple_shifted_spur_pair_recipes(&mut parameters).unwrap();
    let p = prepare_gear_pair_geometry_edit(
        &s,
        1,
        actor(),
        request.pair,
        &request.parameters,
        &parameters,
    )
    .unwrap();
    s.commit(p.prepared).unwrap();
    let after = compiler()
        .compile_cached(&mut cache, s.document(), 2, None)
        .unwrap();
    assert!(!after.gear_pair_checks()[&request.pair].cycle_reused);
    assert!(
        after.gear_pair_checks()[&request.pair]
            .current_engagement
            .is_some()
    );
    for gear in &request.gears {
        assert!(
            (before.solid(gear.feature).unwrap().volume_mm3().unwrap()
                - after.solid(gear.feature).unwrap().volume_mm3().unwrap())
            .abs()
                < 1e-6
        );
    }
}
