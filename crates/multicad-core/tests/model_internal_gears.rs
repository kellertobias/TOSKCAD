use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use std::f64::consts::PI;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Internal ring source").unwrap()).unwrap()
}
fn count(v: f64) -> Dimension {
    Dimension::Literal {
        value: v,
        unit: "count".into(),
    }
}
fn ring(teeth: f64) -> GearParameters {
    GearParameters {
        family: GearFamily::InternalRing,
        module: length(1.),
        teeth: count(teeth),
        bore_diameter: length(0.),
        outer_diameter: Some(length(teeth + 8.)),
        face_width: length(2.),
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
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
#[test]
fn sourced_inward_dimensions_and_independent_thinning_keep_the_outer_rim() {
    let s = session();
    for teeth in [48., 60.] {
        let p = ring(teeth);
        let r = p.resolve(s.parameters()).unwrap();
        close(r.pitch_radius_mm, teeth / 2.);
        close(r.tip_radius_mm, teeth / 2. - 1.);
        close(r.root_radius_mm, teeth / 2. + 1.25);
        close(r.outer_radius_mm.unwrap(), teeth / 2. + 4.);
        let q = GearParameters {
            backlash: length(0.1),
            fabrication_allowance: length(0.03),
            ..p
        }
        .resolve(s.parameters())
        .unwrap();
        close(q.physical_tooth_thickness_mm, PI / 2. - 0.13);
        assert!(q.half_base_angle_rad < r.half_base_angle_rad);
        close(q.outer_radius_mm.unwrap(), r.outer_radius_mm.unwrap());
    }
    assert!(
        serde_json::to_value(GearParameters::default())
            .unwrap()
            .get("outer_diameter")
            .is_none()
    );
}
#[test]
fn unsupported_ring_and_pair_parameters_leave_no_partial_source() {
    let s = session();
    let before = s.document().clone();
    for p in [
        GearParameters {
            outer_diameter: None,
            ..ring(48.)
        },
        GearParameters {
            outer_diameter: Some(length(51.)),
            ..ring(48.)
        },
        ring(40.),
        GearParameters {
            pressure_angle: angle(25.),
            ..ring(48.)
        },
        GearParameters {
            bore_diameter: length(2.),
            ..ring(48.)
        },
        GearParameters {
            helix_angle: angle(5.),
            hand: GearHand::Left,
            ..ring(48.)
        },
        GearParameters {
            profile_shift: Dimension::Literal {
                value: 0.1,
                unit: "ratio".into(),
            },
            ..ring(48.)
        },
        GearParameters {
            backlash: length(10.),
            ..ring(48.)
        },
    ] {
        assert!(prepare_gear(&s, 0, actor(), &GearRequest::new("Invalid ring", p)).is_err());
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
    for p in [
        GearParameters {
            teeth: count(23.),
            ..pinion()
        },
        GearParameters {
            module: length(2.),
            ..pinion()
        },
        GearParameters {
            pressure_angle: angle(25.),
            ..pinion()
        },
    ] {
        let req = GearPairRequest::new(
            "Bad pair",
            [
                GearRequest::new("Pinion", p),
                GearRequest::new("Ring", ring(48.)),
            ],
        );
        assert!(prepare_gear_pair(&s, 0, actor(), &req).is_err());
        assert_eq!(s.document(), &before);
    }
}
#[test]
fn both_pair_orders_derive_difference_centers_and_same_direction_phase() {
    let s = session();
    for teeth in [48., 60.] {
        for reverse in [false, true] {
            let mut gears = [
                GearRequest::new("Pinion", pinion()),
                GearRequest::new("Ring", ring(teeth)),
            ];
            if reverse {
                gears.swap(0, 1);
            }
            let mut req = GearPairRequest::new("Pair", gears);
            let base = req
                .parameters
                .resolve_recipes(
                    [&req.gears[0].parameters, &req.gears[1].parameters],
                    s.parameters(),
                )
                .unwrap();
            close(base.center_distance_mm, (teeth - 24.) / 2.);
            req.parameters.drive_angle = angle(13.);
            let driven = req
                .parameters
                .resolve_recipes(
                    [&req.gears[0].parameters, &req.gears[1].parameters],
                    s.parameters(),
                )
                .unwrap();
            let ratio = f64::from(base.gears[0].teeth) / f64::from(base.gears[1].teeth);
            close(
                driven.phases_rad[1] - base.phases_rad[1],
                13_f64.to_radians() * ratio,
            );
            assert!(driven.phases_rad[1] > base.phases_rad[1]);
            prepare_gear_pair(&s, 0, actor(), &req).unwrap();
            assert_eq!(s.revision(), 0);
        }
    }
}
fn check_reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(INTERNAL_GEAR_CAPABILITY));
    assert!(project.capabilities().contains(GEAR_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(INTERNAL_GEAR_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(INTERNAL_GEAR_CAPABILITY)
    );
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, *s.document());
}
#[test]
fn outer_expressions_history_and_hidden_templates_require_the_internal_reader() {
    let mut s = session();
    let param = Entity::new(
        "rimOutside",
        EntityData::Parameter {
            expression: "56".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = param.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Outside parameter".into(),
        edits: vec![Mutation::Insert(param)],
    })
    .unwrap();
    let p = GearParameters {
        outer_diameter: Some(Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=$rimOutside", "mm", None).unwrap(),
        }),
        ..ring(48.)
    };
    let req = GearRequest::new("Ring", p);
    let prepared = prepare_gear(&s, s.revision(), actor(), &req).unwrap();
    s.commit(prepared.prepared).unwrap();
    let EntityData::Feature { inputs, .. } = &s.document().get(req.feature).unwrap().data else {
        panic!()
    };
    assert!(inputs.iter().any(|l| l.target == id));
    check_reader(&s);
    let mut entity = s.document().get(req.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut entity.data else {
        panic!()
    };
    *suppressed = true;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Suppress".into(),
        edits: vec![Mutation::Replace(entity)],
    })
    .unwrap();
    check_reader(&s);
    s.undo(s.revision(), actor()).unwrap();
    let author =
        SymbolAuthorRequest::new("Reusable ring", req.body, SymbolKind::Static, vec![], None);
    let prepared = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(prepared.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut imported = session();
    let prepared = symbols::prepare_import(&imported, 0, actor(), &definition).unwrap();
    imported.commit(prepared.prepared).unwrap();
    assert!(
        !imported
            .document()
            .entities()
            .any(|e| matches!(e.data, EntityData::Feature { .. }))
    );
    check_reader(&imported);
}
