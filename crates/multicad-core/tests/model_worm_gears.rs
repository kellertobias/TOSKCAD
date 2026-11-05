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
    Session::new(Document::new("Canonical ZA pair").unwrap()).unwrap()
}
fn count(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "count".into(),
    }
}
fn ratio(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "ratio".into(),
    }
}
fn recipe(role: WormMember, z: f64, m: f64) -> GearParameters {
    let worm = role == WormMember::Worm;
    GearParameters {
        family: GearFamily::Worm,
        module_system: GearModuleSystem::Axial,
        module: length(m),
        teeth: count(if worm { 1. } else { z }),
        face_width: length(if worm { 16. * m } else { 2. * m }),
        bore_diameter: length(0.),
        backlash: length(0.05 * m),
        minimum_wall: length(0.1 * m),
        root_fillet_ratio: ratio(0.),
        hand: GearHand::Right,
        worm: Some(WormDimensions {
            role,
            tooth_system: WormToothSystem::ZaAxialStraight,
            mate_teeth: count(if worm { z } else { 1. }),
            diameter_quotient: ratio(10.),
        }),
        ..Default::default()
    }
}
fn pair() -> GearPairRequest {
    GearPairRequest::new(
        "ZA pair",
        [
            GearRequest::new("Worm", recipe(WormMember::Worm, 32., 1.)),
            GearRequest::new("Wheel", recipe(WormMember::Wheel, 32., 1.)),
        ],
    )
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}
#[test]
fn bounded_roles_resolve_real_dimensions_without_rotary_aliases() {
    let s = session();
    for m in [0.5, 2.] {
        for z in [32., 64.] {
            for role in [WormMember::Worm, WormMember::Wheel] {
                let mut p = recipe(role, z, m);
                for extent in if role == WormMember::Worm {
                    [16. * m, 32. * m]
                } else {
                    [0.5 * m, 2. * m]
                } {
                    p.face_width = length(extent);
                    for thin in [0., 0.1 * m] {
                        p.backlash = length(thin);
                        assert!(p.resolve(s.parameters()).is_err());
                        let g = p.resolve_geometry(s.parameters()).unwrap();
                        let r = g.worm().unwrap();
                        assert_eq!(
                            r.teeth,
                            if role == WormMember::Worm {
                                1
                            } else {
                                z as u32
                            }
                        );
                        assert_eq!(r.wheel_teeth, z as u32);
                        near(r.pitch_mm, PI * m);
                        near(r.lead_mm, PI * m);
                        near(r.lead_angle_rad, 0.1_f64.atan());
                        near(
                            r.axial_pressure_angle_rad,
                            (20_f64.to_radians().tan() * 1.01_f64.sqrt()).atan(),
                        );
                        near(
                            r.minimum_root_radius_mm,
                            if role == WormMember::Worm {
                                3.75 * m
                            } else {
                                (z / 2. - 1.25) * m
                            },
                        );
                        near(r.physical_tooth_thickness_mm, PI * m / 2. - thin);
                        assert_eq!(
                            r.hob_pitch_space_thickness_mm,
                            if role == WormMember::Wheel {
                                Some(PI * m / 2. + thin)
                            } else {
                                None
                            }
                        );
                        assert!(g.rotary().is_none() && g.rack().is_none() && g.bevel().is_none());
                    }
                }
            }
        }
    }
    let legacy = serde_json::to_value(GearParameters::default()).unwrap();
    assert!(legacy.get("worm").is_none());
    assert_eq!(
        serde_json::from_value::<GearParameters>(legacy).unwrap(),
        GearParameters::default()
    );
}
#[test]
fn unsupported_systems_units_and_impossible_geometry_remain_source_neutral() {
    let s = session();
    let before = s.document().clone();
    let good = recipe(WormMember::Worm, 32., 1.);
    let mut cases = vec![];
    for role in [WormMember::Worm, WormMember::Wheel] {
        for z in [31., 65., 32.5] {
            cases.push(recipe(role, z, 1.));
        }
        let p = recipe(role, 32., 1.);
        for m in [0.49, 2.01] {
            let mut q = p.clone();
            q.module = length(m);
            cases.push(q);
        }
        for width in [0.1, 40.] {
            let mut q = p.clone();
            q.face_width = length(width);
            cases.push(q);
        }
        for thin in [-0.001, 0.101] {
            let mut q = p.clone();
            q.backlash = length(thin);
            cases.push(q);
        }
    }
    for system in [
        WormToothSystem::Zi,
        WormToothSystem::Zn,
        WormToothSystem::Zk,
    ] {
        let mut q = good.clone();
        q.worm.as_mut().unwrap().tooth_system = system;
        cases.push(q);
    }
    for hand in [GearHand::None, GearHand::Left] {
        let mut q = good.clone();
        q.hand = hand;
        cases.push(q);
    }
    for system in [GearModuleSystem::Normal, GearModuleSystem::Transverse] {
        let mut q = good.clone();
        q.module_system = system;
        cases.push(q);
    }
    let mut q = good.clone();
    q.worm.as_mut().unwrap().diameter_quotient = ratio(11.);
    cases.push(q);
    let mut q = good.clone();
    q.worm.as_mut().unwrap().mate_teeth = length(32.);
    cases.push(q);
    let mut q = good.clone();
    q.pressure_angle = angle(21.);
    cases.push(q);
    let mut q = good.clone();
    q.helix_angle = angle(1.);
    cases.push(q);
    let mut q = good.clone();
    q.root_fillet_ratio = ratio(0.1);
    cases.push(q);
    let mut q = good.clone();
    q.bore_diameter = length(8.);
    cases.push(q);
    let mut q = good.clone();
    q.fabrication_allowance = length(0.06);
    cases.push(q);
    let mut q = good.clone();
    q.worm = None;
    cases.push(q);
    cases.push(GearParameters {
        module_system: GearModuleSystem::Axial,
        ..Default::default()
    });
    for q in cases {
        assert!(prepare_gear(&s, 0, actor(), &GearRequest::new("Invalid", q)).is_err());
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
}
#[test]
fn pair_drive_is_worm_owned_in_both_orders_and_preserves_skew_axis_datum() {
    let s = session();
    for reverse in [false, true] {
        let mut p = pair();
        if reverse {
            p.gears.swap(0, 1);
            p.parameters.gears = p.gears.each_ref().map(|g| g.feature);
        }
        p.parameters.frame = PrimitiveFrame {
            origin: [length(10.), length(20.), length(30.)],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        };
        p.parameters.drive_angle = angle(360.);
        let g = p
            .parameters
            .resolve_recipes(p.gears.each_ref().map(|g| &g.parameters), s.parameters())
            .unwrap();
        let c = g.worm.as_ref().unwrap();
        let wi = c.worm_member.index();
        let gi = c.wheel_member.index();
        assert_eq!(wi, if reverse { 1 } else { 0 });
        near(c.worm_period_rad, 2. * PI);
        near(g.phases_rad[wi], 2. * PI);
        near(g.phases_rad[gi], 2. * PI / 32.);
        assert_eq!(g.mating[wi].origin_mm, [10., 20., 30.]);
        assert_eq!(g.mating[gi].origin_mm, [10., -1., 30.]);
        near(g.center_distance_mm, 21.);
        let wheel_normal = [
            g.mating[gi].u[1] * g.mating[gi].v[2] - g.mating[gi].u[2] * g.mating[gi].v[1],
            g.mating[gi].u[2] * g.mating[gi].v[0] - g.mating[gi].u[0] * g.mating[gi].v[2],
            g.mating[gi].u[0] * g.mating[gi].v[1] - g.mating[gi].u[1] * g.mating[gi].v[0],
        ];
        for (actual, expected) in wheel_normal.into_iter().zip([0., 0., 1.]) {
            near(actual, expected);
        }
        let mut conflict = p.clone();
        conflict.gears[gi]
            .parameters
            .worm
            .as_mut()
            .unwrap()
            .mate_teeth = count(2.);
        assert!(prepare_gear_pair(&s, 0, actor(), &conflict).is_err());
        p.parameters.drive_angle = angle(360001.);
        assert!(prepare_gear_pair(&s, 0, actor(), &p).is_err());
    }
}
#[test]
fn typed_coupling_grouped_history_and_independent_reopen_preserve_identity() {
    let mut s = session();
    let z = Entity::new(
        "wheel_count",
        EntityData::Parameter {
            expression: "32".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let q = Entity::new(
        "quotient",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "ratio".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Inputs".into(),
        edits: vec![Mutation::Insert(z.clone()), Mutation::Insert(q.clone())],
    })
    .unwrap();
    let mut recipes = [
        recipe(WormMember::Worm, 32., 1.),
        recipe(WormMember::Wheel, 32., 1.),
    ];
    recipes[1].teeth = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$wheel_count", "count", None).unwrap(),
    };
    recipes[0].worm.as_mut().unwrap().diameter_quotient = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$quotient", "ratio", None).unwrap(),
    };
    let recipes = couple_worm_pair_recipes(&recipes).unwrap();
    assert_eq!(
        recipes[0].worm.as_ref().unwrap().mate_teeth,
        recipes[1].teeth
    );
    assert_eq!(
        recipes[1].worm.as_ref().unwrap().diameter_quotient,
        recipes[0].worm.as_ref().unwrap().diameter_quotient
    );
    let req = GearPairRequest::new("Typed ZA", recipes.map(|p| GearRequest::new("Member", p)));
    let before = s.document().clone();
    let candidate = prepare_gear_pair(&s, 1, actor(), &req).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(candidate.prepared).unwrap();
    let original = s.document().clone();
    for g in &req.gears {
        let EntityData::Feature { inputs, .. } = &s.document().get(g.feature).unwrap().data else {
            panic!()
        };
        assert!(inputs.iter().any(|l| l.target == q.id));
        assert!(inputs.iter().any(|l| l.target == z.id));
    }
    let mut edited = req.gears.each_ref().map(|g| g.parameters.clone());
    edited[1].teeth = count(40.);
    assert!(prepare_gear_edit(&s, 2, actor(), req.gears[1].feature, &edited[1]).is_err());
    let edited = couple_worm_pair_recipes(&edited).unwrap();
    s.commit(
        prepare_gear_pair_geometry_edit(&s, 2, actor(), req.pair, &req.parameters, &edited)
            .unwrap()
            .prepared,
    )
    .unwrap();
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
    let independent = Session::new(reopened.project.document).unwrap();
    assert_eq!(independent.document(), &saved);
    let resolved = resolve_gear_pair_parameters(
        independent.document(),
        independent.parameters(),
        &req.parameters,
    )
    .unwrap();
    assert_eq!(resolved.worm.unwrap().wheel_teeth, 40);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
}

fn reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(WORM_GEAR_CAPABILITY));
    assert!(project.capabilities().contains(GEAR_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(WORM_GEAR_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(WORM_GEAR_CAPABILITY)
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
fn worm_reader_is_required_for_direct_suppressed_and_template_only_retained_sources() {
    let mut s = session();
    let req = GearRequest::new("Worm", recipe(WormMember::Worm, 32., 1.));
    s.commit(prepare_gear(&s, 0, actor(), &req).unwrap().prepared)
        .unwrap();
    reader(&s);
    let mut feature = s.document().get(req.feature).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut feature.data else {
        panic!()
    };
    *suppressed = true;
    s.apply(Request {
        expected_revision: 1,
        actor: actor(),
        label: "Suppress".into(),
        edits: vec![Mutation::Replace(feature)],
    })
    .unwrap();
    reader(&s);
    s.undo(s.revision(), actor()).unwrap();
    let author =
        SymbolAuthorRequest::new("Reusable worm", req.body, SymbolKind::Static, vec![], None);
    s.commit(
        symbols::prepare_author(&s, s.revision(), actor(), &author)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut imported = session();
    imported
        .commit(
            symbols::prepare_import(&imported, 0, actor(), &definition)
                .unwrap()
                .prepared,
        )
        .unwrap();
    assert!(
        !imported
            .document()
            .entities()
            .any(|entity| matches!(entity.data, EntityData::Feature { .. }))
    );
    reader(&imported);
    assert!(
        !ProjectFile::new(session().document().clone())
            .unwrap()
            .capabilities()
            .contains(WORM_GEAR_CAPABILITY)
    );
}
