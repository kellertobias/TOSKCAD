use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Canonical gears").unwrap()).unwrap()
}
fn scalar(value: f64, unit: &str) -> Dimension {
    Dimension::Literal {
        value,
        unit: unit.into(),
    }
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn helical() -> GearParameters {
    GearParameters {
        family: GearFamily::Helical,
        helix_angle: angle(15.),
        hand: GearHand::Right,
        teeth: scalar(30., "count"),
        ..Default::default()
    }
}
#[test]
fn normal_and_transverse_conventions_resolve_distinct_reference_geometry() {
    let s = session();
    let normal = helical().resolve(s.parameters()).unwrap();
    let transverse = GearParameters {
        module_system: GearModuleSystem::Transverse,
        ..helical()
    }
    .resolve(s.parameters())
    .unwrap();
    let beta = 15_f64.to_radians();
    close(normal.pitch_radius_mm * 2., 60. / beta.cos());
    close(transverse.pitch_radius_mm * 2., 60.);
    close(normal.normal_module_mm, 2.);
    close(transverse.normal_module_mm, 2. * beta.cos());
    close(
        normal.transverse_pressure_angle_rad,
        (20_f64.to_radians().tan() / beta.cos()).atan(),
    );
    close(
        transverse.normal_pressure_angle_rad,
        (20_f64.to_radians().tan() * beta.cos()).atan(),
    );
    for r in [&normal, &transverse] {
        close(r.tip_radius_mm - r.pitch_radius_mm, 2.);
        close(r.pitch_radius_mm - r.root_radius_mm, 2.5);
    }
    let left = GearParameters {
        hand: GearHand::Left,
        ..helical()
    }
    .resolve(s.parameters())
    .unwrap();
    close(left.twist_rad_per_mm, -normal.twist_rad_per_mm);
}
#[test]
fn design_backlash_and_fabrication_thinning_preserve_reference_circles() {
    let s = session();
    let base = GearParameters::default().resolve(s.parameters()).unwrap();
    let altered = GearParameters {
        backlash: length(0.2),
        fabrication_allowance: length(0.1),
        ..Default::default()
    }
    .resolve(s.parameters())
    .unwrap();
    close(base.pitch_radius_mm, altered.pitch_radius_mm);
    close(base.base_radius_mm, altered.base_radius_mm);
    close(base.tip_radius_mm, altered.tip_radius_mm);
    close(
        altered.nominal_tooth_thickness_mm - altered.design_tooth_thickness_mm,
        0.2,
    );
    close(
        altered.design_tooth_thickness_mm - altered.physical_tooth_thickness_mm,
        0.1,
    );
    assert!(altered.half_base_angle_rad < base.half_base_angle_rad);
}
#[test]
fn unsupported_families_and_unsafe_or_ambiguous_profiles_fail_closed() {
    let s = session();
    for family in GearFamily::ALL.into_iter().filter(|f| !f.supported()) {
        assert!(
            GearParameters {
                family,
                ..Default::default()
            }
            .resolve(s.parameters())
            .is_err()
        );
    }
    let invalid = [
        GearParameters {
            profile_shift: scalar(0.1, "ratio"),
            ..Default::default()
        },
        GearParameters {
            teeth: scalar(17., "count"),
            ..Default::default()
        },
        GearParameters {
            teeth: scalar(24.5, "count"),
            ..Default::default()
        },
        GearParameters {
            bore_diameter: length(43.),
            ..Default::default()
        },
        GearParameters {
            backlash: length(100.),
            ..Default::default()
        },
        GearParameters {
            center_gap: length(1.),
            ..Default::default()
        },
        GearParameters {
            family: GearFamily::Helical,
            ..Default::default()
        },
        GearParameters {
            hand: GearHand::None,
            ..helical()
        },
        GearParameters {
            family: GearFamily::DoubleHelical,
            ..helical()
        },
        GearParameters {
            family: GearFamily::DoubleHelical,
            center_gap: length(10.),
            ..helical()
        },
        GearParameters {
            family: GearFamily::Herringbone,
            center_gap: length(1.),
            ..helical()
        },
    ];
    for p in invalid {
        assert!(p.resolve(s.parameters()).is_err(), "accepted {p:?}");
    }
    for family in [
        GearFamily::Helical,
        GearFamily::Herringbone,
        GearFamily::DoubleHelical,
    ] {
        let p = GearParameters {
            family,
            center_gap: length(if family == GearFamily::DoubleHelical {
                2.
            } else {
                0.
            }),
            ..helical()
        };
        assert!(p.resolve(s.parameters()).is_ok());
    }
    let mut json = serde_json::to_value(GearParameters::default()).unwrap();
    json["accuracy_grade"] = "certified".into();
    assert!(serde_json::from_value::<GearParameters>(json).is_err());
}
#[test]
fn preparation_edit_history_and_reopen_preserve_formula_links_and_body_identity() {
    let mut s = session();
    let parameter = Entity::new(
        "module",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Parameter".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let module = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$module", "mm", None).unwrap(),
    };
    let request = GearRequest::new(
        "Gear",
        GearParameters {
            module,
            ..Default::default()
        },
    );
    let before = s.document().clone();
    let candidate = prepare_gear(&s, s.revision(), actor(), &request).unwrap();
    assert_eq!(&before, s.document());
    assert!(
        read_gear(candidate.prepared.document(), request.feature)
            .unwrap()
            .inputs()
            .unwrap()
            .iter()
            .any(|l| l.target == parameter_id)
    );
    s.commit(candidate.prepared).unwrap();
    let committed = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(&before, s.document());
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(&committed, s.document());
    let mut parameters = read_gear(s.document(), request.feature).unwrap().parameters;
    parameters.teeth = scalar(30., "count");
    let edited =
        prepare_gear_edit(&s, s.revision(), actor(), request.feature, &parameters).unwrap();
    s.commit(edited.prepared).unwrap();
    assert_eq!(
        read_gear(s.document(), request.feature)
            .unwrap()
            .output
            .body,
        request.body
    );
    let mut changed = s.document().get(parameter_id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        unreachable!()
    };
    *expression = "3".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Change module".into(),
        edits: vec![Mutation::Replace(changed)],
    })
    .unwrap();
    close(
        read_gear(s.document(), request.feature)
            .unwrap()
            .parameters
            .resolve(s.parameters())
            .unwrap()
            .pitch_radius_mm,
        45.,
    );
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(GEAR_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(&reopened, s.document());
    let mut feature = reopened.get(request.feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut feature.data else {
        unreachable!()
    };
    inputs.clear();
    assert!(reopened.edited(&[Mutation::Replace(feature)]).is_err());
}
