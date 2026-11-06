use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use serde_json::Value;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Shifted external geometry").unwrap()).unwrap()
}
fn scalar(value: f64, unit: &str) -> Dimension {
    Dimension::Literal {
        value,
        unit: unit.into(),
    }
}
fn near(a: f64, b: f64) {
    assert!(
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.),
        "{a} != {b}"
    );
}
fn number(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap()
}
fn recipe(
    family: GearFamily,
    system: GearModuleSystem,
    z: f64,
    x: f64,
    hand: GearHand,
) -> GearParameters {
    GearParameters {
        family,
        module_system: system,
        module: length(1.),
        teeth: scalar(z, "count"),
        profile_shift: ratio(x),
        helix_angle: angle(25.),
        hand,
        face_width: length(4.),
        bore_diameter: length(2.),
        backlash: length(0.02),
        minimum_wall: length(0.5),
        center_gap: length(if family == GearFamily::DoubleHelical {
            1.
        } else {
            0.
        }),
        hub: Some(GearHub {
            diameter: length(8.),
            axial_start: length(-2.),
            axial_end: length(6.),
        }),
        ..Default::default()
    }
}
fn matched(mut recipes: [GearParameters; 2]) -> GearPairRequest {
    couple_shifted_gear_pair_recipes(&mut recipes).unwrap();
    GearPairRequest::new(
        "Shifted pair",
        recipes.map(|p| GearRequest::new("Member", p)),
    )
}
#[test]
fn all_helical_bands_planes_signs_and_hands_match_independent_primary_source_scalars() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/shifted_external_reference.json")).unwrap();
    let s = session();
    let rows = data["fixtures"].as_array().unwrap();
    assert_eq!(rows.len(), 36);
    for fixture in rows {
        let system = if fixture["selected_plane"] == "normal" {
            GearModuleSystem::Normal
        } else {
            GearModuleSystem::Transverse
        };
        let recipes = std::array::from_fn(|i| {
            let e = &fixture["members"][i];
            recipe(
                serde_json::from_value(e["family"].clone()).unwrap(),
                system,
                number(e, "teeth"),
                number(e, "shift_selected_plane"),
                serde_json::from_value(e["hand"].clone()).unwrap(),
            )
        });
        let req = matched(recipes);
        let r = req
            .parameters
            .resolve_recipes(req.gears.each_ref().map(|g| &g.parameters), s.parameters())
            .unwrap();
        near(r.center_distance_mm, number(fixture, "center_distance_mm"));
        let a = r.gears[0].rotary().unwrap();
        let b = r.gears[1].rotary().unwrap();
        let context = shifted_gear_pair_geometry(a, b).unwrap();
        near(
            context.working_pressure_angle_rad.to_degrees(),
            number(fixture, "working_transverse_pressure_angle_deg"),
        );
        near(
            context.contact_ratio,
            number(fixture, "transverse_contact_ratio"),
        );
        near(
            context.tip_shortening_coefficient,
            number(fixture, "tip_shortening_coeff"),
        );
        for (i, g) in [a, b].into_iter().enumerate() {
            let expected = &fixture["members"][i];
            for (actual, key) in [
                (g.pitch_radius_mm, "pitch_radius_mm"),
                (g.base_radius_mm, "base_radius_mm"),
                (g.tip_radius_mm, "tip_radius_mm"),
                (g.root_radius_mm, "root_radius_mm"),
                (g.half_base_angle_rad, "half_base_angle_rad"),
                (
                    g.nominal_tooth_thickness_mm,
                    "nominal_transverse_thickness_mm",
                ),
                (
                    g.physical_tooth_thickness_mm,
                    "physical_transverse_thickness_mm",
                ),
                (g.twist_rad_per_mm, "twist_rad_per_mm"),
            ] {
                near(actual, number(expected, key));
            }
        }
    }
}
#[test]
fn source_helical_tables_and_normal_conversion_are_not_spur_equations_or_virtual_credit() {
    let s = session();
    let mut a = recipe(
        GearFamily::Helical,
        GearModuleSystem::Transverse,
        12.,
        0.34462,
        GearHand::Left,
    );
    let mut b = recipe(
        GearFamily::Helical,
        GearModuleSystem::Transverse,
        60.,
        0.,
        GearHand::Right,
    );
    for p in [&mut a, &mut b] {
        p.module = length(3.);
        p.helix_angle = angle(30.);
        p.hub = None;
        p.bore_diameter = length(0.);
    }
    let req = matched([a, b]);
    let r = req
        .parameters
        .resolve_recipes(req.gears.each_ref().map(|g| &g.parameters), s.parameters())
        .unwrap();
    near(r.center_distance_mm, 108.999992597323);
    near(r.gears[0].rotary().unwrap().root_radius_mm * 2., 30.56772);
    near(r.gears[1].rotary().unwrap().root_radius_mm * 2., 172.5);
    // The source's normal worked example is admitted by direct rack projection,
    // without a virtual-count approximation or treating our custom fillet as hob radius.
    let mut normal = req.gears.each_ref().map(|g| g.parameters.clone());
    for p in &mut normal {
        p.module_system = GearModuleSystem::Normal;
        p.shift_mate = None;
    }
    normal[0].profile_shift = ratio(0.09809);
    let normal = matched(normal);
    let normal = normal
        .parameters
        .resolve_recipes(
            normal.gears.each_ref().map(|g| &g.parameters),
            s.parameters(),
        )
        .unwrap();
    near(normal.center_distance_mm, 125.0000011794189);
    near(
        normal.gears[0].rotary().unwrap().tip_radius_mm * 2.,
        48.15390545057252,
    );
    let mut mixed = req.gears.each_ref().map(|g| g.parameters.clone());
    mixed[1].module_system = GearModuleSystem::Normal;
    assert!(
        req.parameters
            .resolve_recipes(mixed.each_ref(), s.parameters())
            .is_err()
    );
    // Selected 25-degree pressure is an explicit valid source input, not an
    // arbitrary 20-degree-only gate. Actual geometry still constrains it.
    let mut recipes = [
        recipe(
            GearFamily::Helical,
            GearModuleSystem::Normal,
            36.,
            0.2,
            GearHand::Right,
        ),
        recipe(
            GearFamily::Helical,
            GearModuleSystem::Normal,
            48.,
            0.2,
            GearHand::Left,
        ),
    ];
    for p in &mut recipes {
        p.pressure_angle = angle(25.);
    }
    let req = matched(recipes);
    req.parameters
        .resolve_recipes(req.gears.each_ref().map(|g| &g.parameters), s.parameters())
        .unwrap();
}
#[test]
fn coupled_external_edits_preserve_typed_mates_ids_history_and_reopen() {
    let mut s = session();
    let input = Entity::new(
        "mate_shift",
        EntityData::Parameter {
            expression: "0.3".into(),
            unit: "ratio".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = input.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Source".into(),
        edits: vec![Mutation::Insert(input)],
    })
    .unwrap();
    let mut p = [
        recipe(
            GearFamily::Herringbone,
            GearModuleSystem::Normal,
            24.,
            0.2,
            GearHand::Right,
        ),
        recipe(
            GearFamily::Herringbone,
            GearModuleSystem::Normal,
            36.,
            0.3,
            GearHand::Left,
        ),
    ];
    p[1].profile_shift = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$mate_shift", "ratio", None).unwrap(),
    };
    let req = matched(p);
    s.commit(prepare_gear_pair(&s, 1, actor(), &req).unwrap().prepared)
        .unwrap();
    let original = s.document().clone();
    let mut edited = req.gears.each_ref().map(|g| g.parameters.clone());
    edited[0].profile_shift = ratio(0.25);
    assert!(
        prepare_gear_edit(&s, s.revision(), actor(), req.gears[0].feature, &edited[0]).is_err()
    );
    for p in &mut edited {
        p.shift_mate = None;
    }
    couple_shifted_gear_pair_recipes(&mut edited).unwrap();
    assert_eq!(
        edited[0].shift_mate.as_ref().unwrap().profile_shift,
        req.gears[1].parameters.profile_shift
    );
    s.commit(
        prepare_gear_pair_geometry_edit(
            &s,
            s.revision(),
            actor(),
            req.pair,
            &req.parameters,
            &edited,
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    for gear in &req.gears {
        let EntityData::Feature { inputs, .. } = &s.document().get(gear.feature).unwrap().data
        else {
            panic!()
        };
        assert!(inputs.iter().any(|l| l.target == id));
    }
    assert_eq!(
        read_gear_pair(s.document(), req.pair).unwrap().outputs,
        req.outputs
    );
    let saved = s.document().clone();
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
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
}
#[test]
fn expanded_reader_covers_suppressed_and_template_intent_without_reclassifying_legacy_zero_expressions()
 {
    let mut s = session();
    let p = recipe(
        GearFamily::Helical,
        GearModuleSystem::Normal,
        24.,
        0.2,
        GearHand::Right,
    );
    let req = GearRequest::new("Shifted helix", p);
    s.commit(prepare_gear(&s, 0, actor(), &req).unwrap().prepared)
        .unwrap();
    let reader = |s: &Session| {
        let project = ProjectFile::new(s.document().clone()).unwrap();
        assert!(
            project
                .capabilities()
                .contains(SHIFTED_EXTERNAL_GEAR_CAPABILITY)
        );
        let mut bytes = vec![];
        project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let mut old = project_file::supported_capabilities();
        old.remove(SHIFTED_EXTERNAL_GEAR_CAPABILITY);
        assert!(
            project_file::read(&mut bytes.as_slice(), Default::default(), &old)
                .unwrap_err()
                .contains(SHIFTED_EXTERNAL_GEAR_CAPABILITY)
        );
    };
    reader(&s);
    let mut feature = s.document().get(req.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut feature.data else {
        panic!()
    };
    *suppressed = true;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Suppress".into(),
        edits: vec![Mutation::Replace(feature)],
    })
    .unwrap();
    reader(&s);
    s.undo(s.revision(), actor()).unwrap();
    let author = SymbolAuthorRequest::new("Helix", req.body, SymbolKind::Static, vec![], None);
    s.commit(
        symbols::prepare_author(&s, s.revision(), actor(), &author)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let content = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut receiving = session();
    receiving
        .commit(
            symbols::prepare_import(&receiving, 0, actor(), &content)
                .unwrap()
                .prepared,
        )
        .unwrap();
    reader(&receiving);
    let mut legacy = session();
    let value = Entity::new(
        "unshifted",
        EntityData::Parameter {
            expression: "0".into(),
            unit: "ratio".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    legacy
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Zero".into(),
            edits: vec![Mutation::Insert(value)],
        })
        .unwrap();
    let mut p = recipe(
        GearFamily::Helical,
        GearModuleSystem::Normal,
        24.,
        0.,
        GearHand::Right,
    );
    p.profile_shift = Dimension::Expression {
        expression: BoundExpression::bind(legacy.document(), "=$unshifted", "ratio", None).unwrap(),
    };
    let req = GearRequest::new("Old zero expression", p);
    legacy
        .commit(prepare_gear(&legacy, 1, actor(), &req).unwrap().prepared)
        .unwrap();
    assert!(
        !ProjectFile::new(legacy.document().clone())
            .unwrap()
            .capabilities()
            .contains(SHIFTED_EXTERNAL_GEAR_CAPABILITY)
    );
    let mut p = GearParameters::default();
    p.profile_shift = ratio(0.2);
    let old = GearRequest::new("Original shifted Spur20", p);
    let mut s = session();
    s.commit(prepare_gear(&s, 0, actor(), &old).unwrap().prepared)
        .unwrap();
    assert!(
        !ProjectFile::new(s.document().clone())
            .unwrap()
            .capabilities()
            .contains(SHIFTED_EXTERNAL_GEAR_CAPABILITY)
    );
}

#[test]
fn coupled_band_symmetry_edits_require_atomic_pair_authority() {
    let mut s = session();
    let req = matched([
        recipe(
            GearFamily::DoubleHelical,
            GearModuleSystem::Normal,
            24.,
            0.2,
            GearHand::Right,
        ),
        recipe(
            GearFamily::DoubleHelical,
            GearModuleSystem::Normal,
            36.,
            0.3,
            GearHand::Left,
        ),
    ]);
    s.commit(prepare_gear_pair(&s, 0, actor(), &req).unwrap().prepared)
        .unwrap();
    let saved = s.document().clone();
    for field in 0..3 {
        let mut p = req.gears[0].parameters.clone();
        match field {
            0 => p.helix_angle = angle(26.),
            1 => p.hand = GearHand::Left,
            _ => p.center_gap = length(1.2),
        }
        p.resolve(s.parameters())
            .expect("Individually valid source; coupled symmetry requires both recipes");
        assert!(
            prepare_gear_edit(&s, 1, actor(), req.gears[0].feature, &p).is_err(),
            "coupled field {field}"
        );
        assert_eq!(s.document(), &saved);
        assert_eq!(s.revision(), 1);
    }
}
