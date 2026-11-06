use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::{self, BoundExpression, Unit},
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Gear coverage").unwrap()).unwrap()
}
fn scalar(value: f64, unit: &str) -> Dimension {
    Dimension::Literal {
        value,
        unit: unit.into(),
    }
}
fn shifted(z: f64, x: f64) -> GearParameters {
    GearParameters {
        module: length(3.),
        teeth: scalar(z, "count"),
        profile_shift: scalar(x, "ratio"),
        bore_diameter: length(2.),
        ..Default::default()
    }
}
fn pair(mut recipes: [GearParameters; 2]) -> GearPairRequest {
    couple_shifted_spur_pair_recipes(&mut recipes).unwrap();
    GearPairRequest::new(
        "Matched shifted spur",
        recipes.map(|p| GearRequest::new("Member", p)),
    )
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}
#[test]
fn typed_reciprocal_pitch_and_both_module_planes_preserve_the_selected_source() {
    let mut doc = Document::new("Pitch").unwrap();
    let count = Entity::new(
        "tooth_count",
        EntityData::Parameter {
            expression: "24".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let dp = Entity::new(
        "pitch",
        EntityData::Parameter {
            expression: "tooth_count / (2 in)".into(),
            unit: "teeth_per_in".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    doc.insert(count).unwrap();
    doc.insert(dp.clone()).unwrap();
    let evaluated = parameters::evaluate(&doc).unwrap();
    near(evaluated.value(dp.id, Unit::TeethPerInch).unwrap(), 12.);
    assert!(evaluated.value(dp.id, Unit::Millimeter).is_err());
    for system in [GearModuleSystem::Normal, GearModuleSystem::Transverse] {
        let mut module = GearParameters {
            family: GearFamily::Helical,
            helix_angle: angle(20.),
            hand: GearHand::Right,
            module_system: system,
            module: length(25.4 / 12.),
            ..Default::default()
        };
        let a = module.resolve(&evaluated).unwrap();
        module.pitch_input = GearPitchInput::DiametralPitch;
        module.module = Dimension::Expression {
            expression: BoundExpression::bind(&evaluated.document, "=$pitch", "teeth_per_in", None)
                .unwrap(),
        };
        let b = module.resolve(&evaluated).unwrap();
        assert_eq!(a, b);
        let source = module.module.clone();
        let request = GearRequest::new("DP", module);
        let inputs = GearDefinition {
            output: BodyOutput { body: request.body },
            parameters: request.parameters.clone(),
        }
        .inputs()
        .unwrap();
        assert!(inputs.iter().any(|link| link.target == dp.id));
        assert_eq!(request.parameters.module, source);
    }
    assert_eq!(Unit::parse("teeth_per_in"), Some(Unit::TeethPerInch));
}
#[test]
fn khk_shifted_pair_and_both_signed_examples_match_independent_source_rows() {
    let s = session();
    for (z1, z2, x1, x2, expected_center) in [
        (12., 24., 0.6, 0.36, 56.499869720304),
        (24., 48., -0.2, 0.2, 108.),
        (36., 48., -0.15, -0.1, 125.2319357212574),
    ] {
        let req = pair([shifted(z1, x1), shifted(z2, x2)]);
        let resolved = req
            .parameters
            .resolve_recipes(req.gears.each_ref().map(|g| &g.parameters), s.parameters())
            .unwrap();
        near(resolved.center_distance_mm, expected_center);
        let a = resolved.gears[0].rotary().unwrap();
        let b = resolved.gears[1].rotary().unwrap();
        let context = shifted_spur_pair_geometry(a, b).unwrap();
        assert!(context.contact_ratio > 1.);
        near(a.root_radius_mm, 3. * (z1 / 2. - 1.25 + x1));
        near(
            a.tip_radius_mm,
            3. * (z1 / 2. + 1. + x1 - context.tip_shortening_coefficient),
        );
        near(
            a.nominal_tooth_thickness_mm,
            3. * (std::f64::consts::PI / 2. + 2. * x1 * 20_f64.to_radians().tan()),
        );
        if z1 == 12. {
            near(
                context.working_pressure_angle_rad.to_degrees(),
                26.0885634421,
            );
            near(context.contact_ratio, 1.20210157005);
        }
    }
}
#[test]
fn hubs_preserve_signed_extent_and_bore_wall_without_erasing_the_tooth_gap() {
    let s = session();
    for family in [
        GearFamily::Spur,
        GearFamily::Helical,
        GearFamily::Herringbone,
        GearFamily::DoubleHelical,
    ] {
        let mut p = GearParameters {
            family,
            hub: Some(GearHub {
                diameter: length(20.),
                axial_start: length(-4.),
                axial_end: length(14.),
            }),
            ..Default::default()
        };
        if family != GearFamily::Spur {
            p.helix_angle = angle(15.);
            p.hand = GearHand::Right;
        }
        if family == GearFamily::DoubleHelical {
            p.center_gap = length(2.);
        }
        let r = p.resolve(s.parameters()).unwrap();
        assert_eq!(
            r.hub,
            Some(ResolvedGearHub {
                radius_mm: 10.,
                axial_start_mm: -4.,
                axial_end_mm: 14.
            })
        );
        for bad in [
            GearHub {
                diameter: length(47.),
                axial_start: length(-4.),
                axial_end: length(14.),
            },
            GearHub {
                diameter: length(9.),
                axial_start: length(-4.),
                axial_end: length(14.),
            },
            GearHub {
                diameter: length(20.),
                axial_start: length(10.),
                axial_end: length(14.),
            },
        ] {
            let mut invalid = p.clone();
            invalid.hub = Some(bad);
            assert!(invalid.resolve(s.parameters()).is_err());
        }
    }
}
#[test]
fn unsupported_and_incompatible_shift_inputs_refuse_without_a_transaction() {
    let s = session();
    let before = s.document().clone();
    for p in [
        shifted(24., -0.5),
        shifted(12., 1.),
        GearParameters {
            family: GearFamily::Helical,
            helix_angle: angle(20.),
            hand: GearHand::Right,
            ..shifted(24., 0.2)
        },
        GearParameters {
            pitch_input: GearPitchInput::DiametralPitch,
            module: length(12.),
            ..Default::default()
        },
    ] {
        assert!(prepare_gear(&s, 0, actor(), &GearRequest::new("Invalid", p)).is_err());
    }
    let bad = pair([shifted(12., 1.), shifted(12., 1.)]);
    assert!(prepare_gear_pair(&s, 0, actor(), &bad).is_err());
    let mut conflict = [shifted(12., 0.6), shifted(24., 0.36)];
    conflict[0].shift_mate = Some(ShiftedSpurMate {
        teeth: scalar(24., "count"),
        profile_shift: scalar(0.3600001, "ratio"),
    });
    assert!(couple_shifted_spur_pair_recipes(&mut conflict).is_err());
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), 0);
}
#[test]
fn atomic_pair_geometry_edits_preserve_ids_history_and_independent_reopen() {
    let mut s = session();
    let req = pair([shifted(12., 0.6), shifted(24., 0.36)]);
    let prepared = prepare_gear_pair(&s, 0, actor(), &req).unwrap();
    assert_eq!(s.revision(), 0);
    drop(prepared);
    s.commit(prepare_gear_pair(&s, 0, actor(), &req).unwrap().prepared)
        .unwrap();
    let original = s.document().clone();
    let mut recipes = req.gears.each_ref().map(|g| g.parameters.clone());
    recipes[0].profile_shift = scalar(0.65, "ratio");
    assert!(prepare_gear_edit(&s, 1, actor(), req.gears[0].feature, &recipes[0]).is_err());
    // Contexts from the previous pair must be explicitly re-derived from the new authored sources.
    for p in &mut recipes {
        p.shift_mate = None;
    }
    couple_shifted_spur_pair_recipes(&mut recipes).unwrap();
    s.commit(
        prepare_gear_pair_geometry_edit(&s, 1, actor(), req.pair, &req.parameters, &recipes)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert_eq!(
        read_gear_pair(s.document(), req.pair).unwrap().outputs,
        req.outputs
    );
    let saved = s.document().clone();
    let project = ProjectFile::new(saved.clone()).unwrap();
    assert!(project.capabilities().contains(GEAR_COVERAGE_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, saved);
    let independent = Session::new(reopened.project.document).unwrap();
    resolve_gear_pair_parameters(
        independent.document(),
        independent.parameters(),
        &req.parameters,
    )
    .unwrap();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
}
#[test]
fn legacy_json_is_unchanged_and_new_unit_requires_an_explicit_reader() {
    let source = serde_json::to_value(GearParameters::default()).unwrap();
    for key in ["pitch_input", "hub", "shift_mate"] {
        assert!(source.get(key).is_none());
    }
    assert_eq!(
        serde_json::from_value::<GearParameters>(source).unwrap(),
        GearParameters::default()
    );
    let mut s = session();
    let parameter = Entity::new(
        "DP",
        EntityData::Parameter {
            expression: "12 teeth_per_in".into(),
            unit: "teeth_per_in".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Pitch source".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(TEETH_PER_INCH_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(TEETH_PER_INCH_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(TEETH_PER_INCH_CAPABILITY)
    );
}
#[test]
fn new_expression_dimensions_keep_all_dependencies_and_update_both_matched_sources() {
    let mut s = session();
    let inputs = [
        ("za", "12", "count"),
        ("zb", "24", "count"),
        ("xa", "0.6", "ratio"),
        ("xb", "0.36", "ratio"),
        ("dp", "8.466666666666667", "teeth_per_in"),
        ("hub_d", "16", "mm"),
        ("hub_start", "-2", "mm"),
        ("hub_end", "12", "mm"),
    ];
    let entities: Vec<_> = inputs
        .into_iter()
        .map(|(name, text, unit)| {
            Entity::new(
                name,
                EntityData::Parameter {
                    expression: text.into(),
                    unit: unit.into(),
                    exposed: true,
                    inputs: vec![],
                },
            )
        })
        .collect();
    let ids: Vec<_> = entities.iter().map(|e| e.id).collect();
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Retained sources".into(),
        edits: entities.into_iter().map(Mutation::Insert).collect(),
    })
    .unwrap();
    let bound = |name: &str, unit: &str| Dimension::Expression {
        expression: BoundExpression::bind(s.document(), &format!("=${name}"), unit, None).unwrap(),
    };
    let mut recipes = [shifted(12., 0.6), shifted(24., 0.36)];
    for (i, p) in recipes.iter_mut().enumerate() {
        p.teeth = bound(if i == 0 { "za" } else { "zb" }, "count");
        p.profile_shift = bound(if i == 0 { "xa" } else { "xb" }, "ratio");
        p.pitch_input = GearPitchInput::DiametralPitch;
        p.module = bound("dp", "teeth_per_in");
        p.hub = Some(GearHub {
            diameter: bound("hub_d", "mm"),
            axial_start: bound("hub_start", "mm"),
            axial_end: bound("hub_end", "mm"),
        });
    }
    let originals = recipes.clone();
    couple_shifted_spur_pair_recipes(&mut recipes).unwrap();
    assert_eq!(
        recipes[0].shift_mate.as_ref().unwrap().teeth,
        originals[1].teeth
    );
    assert_eq!(
        recipes[1].shift_mate.as_ref().unwrap().profile_shift,
        originals[0].profile_shift
    );
    let req = GearPairRequest::new("Bound pair", recipes.map(|p| GearRequest::new("Member", p)));
    s.commit(prepare_gear_pair(&s, 1, actor(), &req).unwrap().prepared)
        .unwrap();
    for gear in &req.gears {
        let EntityData::Feature { inputs, .. } = &s.document().get(gear.feature).unwrap().data
        else {
            panic!()
        };
        for id in &ids {
            assert!(inputs.iter().any(|l| l.target == *id), "missing {id}");
        }
    }
    let before =
        resolve_gear_pair_parameters(s.document(), s.parameters(), &req.parameters).unwrap();
    let mut input = s.document().get(ids[3]).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut input.data else {
        panic!()
    };
    *expression = "0.4".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Shift source".into(),
        edits: vec![Mutation::Replace(input)],
    })
    .unwrap();
    let after =
        resolve_gear_pair_parameters(s.document(), s.parameters(), &req.parameters).unwrap();
    assert_ne!(before.center_distance_mm, after.center_distance_mm);
    for i in 0..2 {
        assert_ne!(
            before.gears[i].rotary().unwrap().tip_radius_mm,
            after.gears[i].rotary().unwrap().tip_radius_mm
        );
        assert_eq!(
            read_gear(s.document(), req.gears[i].feature)
                .unwrap()
                .parameters,
            req.gears[i].parameters
        );
    }
}
#[test]
fn coverage_reader_gates_suppressed_sources_and_template_only_library_definitions() {
    use multicad_core::symbols::{self, SymbolAuthorRequest, SymbolKind};
    let mut s = session();
    let req = GearRequest::new(
        "Library hub",
        GearParameters {
            pitch_input: GearPitchInput::DiametralPitch,
            module: scalar(12.7, "teeth_per_in"),
            hub: Some(GearHub {
                diameter: length(20.),
                axial_start: length(-2.),
                axial_end: length(12.),
            }),
            ..Default::default()
        },
    );
    s.commit(prepare_gear(&s, 0, actor(), &req).unwrap().prepared)
        .unwrap();
    let check = |s: &Session| {
        let p = ProjectFile::new(s.document().clone()).unwrap();
        for cap in [GEAR_COVERAGE_CAPABILITY, TEETH_PER_INCH_CAPABILITY] {
            assert!(p.capabilities().contains(cap));
            let mut bytes = vec![];
            project_file::write(&p, &mut bytes, Default::default()).unwrap();
            let mut old = project_file::supported_capabilities();
            old.remove(cap);
            assert!(
                project_file::read(&mut bytes.as_slice(), Default::default(), &old)
                    .unwrap_err()
                    .contains(cap)
            );
        }
    };
    check(&s);
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
    check(&s);
    s.undo(s.revision(), actor()).unwrap();
    let author =
        SymbolAuthorRequest::new("Hub definition", req.body, SymbolKind::Static, vec![], None);
    s.commit(
        symbols::prepare_author(&s, s.revision(), actor(), &author)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let content = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut receiver = session();
    receiver
        .commit(
            symbols::prepare_import(&receiver, 0, actor(), &content)
                .unwrap()
                .prepared,
        )
        .unwrap();
    assert!(
        !receiver
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Feature { .. }))
    );
    check(&receiver);
}
