//! Independent primary-source scalars and Green volumes for the complete
//! shifted external helical-band family envelope; no display proxy oracle.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, EvaluationCache, Model};
use serde_json::Value;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn n(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap()
}
fn data() -> Value {
    serde_json::from_str(include_str!(
        "../../multicad-core/tests/fixtures/shifted_external_reference.json"
    ))
    .unwrap()
}
fn scalar(value: f64, unit: &str) -> Dimension {
    Dimension::Literal {
        value,
        unit: unit.into(),
    }
}
fn near(actual: f64, expected: f64, budget: f64) {
    assert!(
        (actual - expected).abs() <= budget,
        "{actual:.12} != {expected:.12}; budget {budget}"
    );
}
fn request(fixture: &Value) -> GearPairRequest {
    let system = if fixture["selected_plane"] == "normal" {
        GearModuleSystem::Normal
    } else {
        GearModuleSystem::Transverse
    };
    let mut recipes = std::array::from_fn(|i| {
        let member = &fixture["members"][i];
        let hub = &member["hub"];
        GearParameters {
            family: serde_json::from_value(member["family"].clone()).unwrap(),
            pitch_input: GearPitchInput::DiametralPitch,
            module: scalar(25.4 / n(fixture, "selected_module_mm"), "teeth_per_in"),
            module_system: system,
            teeth: scalar(n(member, "teeth"), "count"),
            profile_shift: ratio(n(member, "shift_selected_plane")),
            pressure_angle: angle(n(fixture, "selected_pressure_angle_deg")),
            helix_angle: angle(n(fixture, "helix_angle_deg")),
            hand: serde_json::from_value(member["hand"].clone()).unwrap(),
            face_width: length(n(member, "face_width_mm")),
            center_gap: length(n(member, "center_gap_mm")),
            bore_diameter: length(2. * n(member, "bore_radius_mm")),
            backlash: length(n(member, "transverse_thinning_mm")),
            fabrication_allowance: length(0.),
            root_fillet_ratio: ratio(0.2),
            minimum_wall: length(0.5),
            hub: Some(GearHub {
                diameter: length(2. * n(hub, "radius_mm")),
                axial_start: length(n(hub, "axial_start_mm")),
                axial_end: length(n(hub, "axial_end_mm")),
            }),
            frame: PrimitiveFrame {
                origin: [length(10. + 20. * i as f64), length(20.), length(30.)],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            ..Default::default()
        }
    });
    couple_shifted_gear_pair_recipes(&mut recipes).unwrap();
    let mut request = GearPairRequest::new(
        "Independent shifted bands",
        recipes.map(|p| GearRequest::new("Member", p)),
    );
    request.parameters.frame = PrimitiveFrame {
        origin: [length(100.), length(200.), length(300.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    request
}
fn check_witness(model: &Model, req: &GearPairRequest, reused: bool) {
    let check = &model.gear_pair_checks()[&req.pair];
    assert_eq!(check.cycle_reused, reused);
    assert!(check.current_pose_checked);
    assert_eq!(check.cycle_drive_angles_rad.len(), 9);
    assert_eq!(check.maximum_overlap_volume_mm3, 0.);
    let witness = check
        .current_engagement
        .as_ref()
        .expect("Native engagement witness");
    assert!(witness.distance_mm.is_finite());
    assert!(witness.points_mm.iter().flatten().all(|v| v.is_finite()));
    match req.gears[0].parameters.family {
        GearFamily::Herringbone | GearFamily::DoubleHelical => {
            assert_eq!(witness.kind, "shifted_chevron_both_bands")
        }
        GearFamily::Helical => assert!(matches!(
            witness.kind,
            "shifted_helical_flank_faces" | "whole_solid_fallback"
        )),
        _ => panic!("Unexpected fixture family"),
    }
    assert_eq!(model.snapshot().leaf_occurrences.len(), 2);
}
fn volume_budget(solid: &Solid, expected: f64) -> f64 {
    // The native profile linear approximation and unchanged aggregate GK
    // relative-volume guard are distinct contributors to this oracle budget.
    let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * expected.abs()
}
fn check_native_and_step(model: &Model, req: &GearPairRequest, fixture: &Value) {
    let directory = tempfile::tempdir().unwrap();
    for (i, gear) in req.gears.iter().enumerate() {
        let solid = model.solid(gear.feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        // Oracle includes each tooth band's true Green area, the continuous
        // root annulus through a DoubleHelical gap, and ONLY hub material
        // outside the already-present root core. No double-counted overlap.
        let expected = n(&fixture["members"][i], "combined_volume_mm3");
        near(
            solid.volume_mm3().unwrap(),
            expected,
            volume_budget(solid, expected),
        );
        let path = directory.path().join(format!("shifted-{i}.step"));
        solid.write_step(&path).unwrap();
        let imported = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(imported.solid_count().unwrap(), 1);
        near(
            imported.volume_mm3().unwrap(),
            expected,
            volume_budget(&imported, expected),
        );
        let bounds = solid.bounds().unwrap();
        let other = imported.bounds().unwrap();
        for axis in 0..3 {
            near(bounds.min[axis], other.min[axis], 2e-6);
            near(bounds.max[axis], other.max[axis], 2e-6);
        }
        // Local axial Z maps to world X in these independent definition frames.
        let hub = &fixture["members"][i]["hub"];
        near(
            bounds.min[0],
            10. + 20. * i as f64 + n(hub, "axial_start_mm"),
            2e-6,
        );
        near(
            bounds.max[0],
            10. + 20. * i as f64 + n(hub, "axial_end_mm"),
            2e-6,
        );
    }
}
fn lifecycle(s: &mut Session, req: &GearPairRequest, cache: &mut EvaluationCache) {
    let original = s.document().clone();
    let recipes = req
        .gears
        .each_ref()
        .map(|g| read_gear(s.document(), g.feature).unwrap().parameters);
    let mut driven = req.parameters.clone();
    driven.drive_angle = angle(37.);
    s.commit(
        prepare_gear_pair_edit(s, s.revision(), actor(), req.pair, &driven)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let native = compiler()
        .compile_cached(cache, s.document(), s.revision(), None)
        .unwrap();
    check_witness(&native, req, true);
    assert!(cache.last_compile().evaluated.is_empty());
    for (i, gear) in req.gears.iter().enumerate() {
        assert_eq!(
            read_gear(s.document(), gear.feature).unwrap().parameters,
            recipes[i]
        );
    }
    let driven_source = s.document().clone();
    let mut changed = recipes;
    changed[0].profile_shift = ratio(0.25);
    assert!(
        prepare_gear_edit(s, s.revision(), actor(), req.gears[0].feature, &changed[0]).is_err()
    );
    assert_eq!(s.document(), &driven_source);
    for p in &mut changed {
        p.shift_mate = None;
    }
    couple_shifted_gear_pair_recipes(&mut changed).unwrap();
    s.commit(
        prepare_gear_pair_geometry_edit(s, s.revision(), actor(), req.pair, &driven, &changed)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let native = compiler()
        .compile_cached(cache, s.document(), s.revision(), None)
        .unwrap();
    check_witness(&native, req, false);
    assert!(
        cache
            .last_compile()
            .evaluated
            .contains(&req.gears[0].feature)
    );
    assert!(
        cache
            .last_compile()
            .evaluated
            .contains(&req.gears[1].feature)
    );
    assert_eq!(
        read_gear_pair(s.document(), req.pair).unwrap().outputs,
        req.outputs
    );
    let edited = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &driven_source);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &edited);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(edited.clone()).unwrap(),
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
    assert_eq!(reopened.project.document, edited);
    let independent = Session::new(reopened.project.document).unwrap();
    let native = compiler().compile(independent.document(), 0).unwrap();
    check_witness(&native, req, false);
    for (i, gear) in req.gears.iter().enumerate() {
        assert_eq!(
            read_gear(independent.document(), gear.feature)
                .unwrap()
                .parameters,
            changed[i]
        );
    }
    assert_ne!(original, edited);
}
#[test]
fn all_shifted_helical_families_in_both_planes_have_native_mating_complete_volume_and_step() {
    let data = data();
    let names = [
        "normal_helical_positive_right",
        "transverse_helical_negative_positive_left",
        "normal_herringbone_both_negative_right",
        "transverse_herringbone_positive_left",
        "normal_double_helical_negative_positive_left",
        "transverse_double_helical_both_negative_right",
    ];
    for (index, name) in names.into_iter().enumerate() {
        eprintln!("Shifted native fixture: {name}");
        let fixture = data["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap();
        let mut session = Session::new(Document::new(name).unwrap()).unwrap();
        let request = request(fixture);
        let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
        assert_eq!(session.revision(), 0);
        let mut cache = EvaluationCache::default();
        let candidate = compiler()
            .compile_cached(&mut cache, prepared.prepared.document(), 1, None)
            .unwrap();
        check_witness(&candidate, &request, false);
        near(
            candidate.gear_pair_checks()[&request.pair].center_distance_mm,
            n(fixture, "center_distance_mm"),
            1e-8,
        );
        check_native_and_step(&candidate, &request, fixture);
        session.commit(prepared.prepared).unwrap();
        if index == 0 {
            lifecycle(&mut session, &request, &mut cache);
        }
    }
}
#[test]
fn both_primary_khk_shifted_helical_worked_candidates_have_real_native_bodies_and_step() {
    let data = data();
    for fixture in data["source_examples"].as_array().unwrap() {
        eprintln!("KHK native worked candidate: {}", fixture["selected_plane"]);
        let session = Session::new(Document::new("KHK worked helical").unwrap()).unwrap();
        let request = request(fixture);
        let prepared = prepare_gear_pair(&session, 0, actor(), &request).unwrap();
        let candidate = compiler().compile(prepared.prepared.document(), 1).unwrap();
        check_witness(&candidate, &request, false);
        near(
            candidate.gear_pair_checks()[&request.pair].center_distance_mm,
            n(fixture, "center_distance_mm"),
            1e-8,
        );
        check_native_and_step(&candidate, &request, fixture);
        assert_eq!(session.revision(), 0);
        assert!(!session.can_undo());
    }
}
