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
    Session::new(Document::new("Canonical bevel").unwrap()).unwrap()
}
fn count(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "count".into(),
    }
}
fn bevel(teeth: f64, mate: f64) -> GearParameters {
    GearParameters {
        family: GearFamily::Bevel,
        module: length(1.),
        teeth: count(teeth),
        face_width: length(3.),
        bore_diameter: length(2.),
        bevel: Some(BevelDimensions {
            tooth_system: BevelToothSystem::SphericalInvoluteStraight,
            mate_teeth: count(mate),
            shaft_angle: angle(90.),
        }),
        ..Default::default()
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn pair() -> GearPairRequest {
    GearPairRequest::new(
        "Spherical-involute pair",
        [
            GearRequest::new("A", bevel(24., 48.)),
            GearRequest::new("B", bevel(48., 24.)),
        ],
    )
}
#[test]
fn declared_cones_and_both_custom_root_branches_have_independent_spherical_and_backcone_oracles() {
    let s = session();
    let mut branches = vec![];
    for (teeth, mate) in [(24., 24.), (24., 64.), (64., 24.), (64., 64.)] {
        let p = bevel(teeth, mate);
        assert!(p.resolve(s.parameters()).is_err());
        let geometry = p.resolve_geometry(s.parameters()).unwrap();
        assert_eq!(geometry.family(), GearFamily::Bevel);
        assert_eq!(geometry.teeth(), teeth as u32);
        assert!(geometry.rotary().is_none() && geometry.rack().is_none());
        let r = geometry.bevel().unwrap();
        branches.push(r.root_branch);
        let delta = (teeth / mate).atan();
        let distance = (teeth * teeth + mate * mate).sqrt() / 2.;
        near(r.pitch_cone_angle_rad, delta);
        near(r.outer_cone_distance_mm, distance);
        near(r.inner_cone_distance_mm, distance - 3.);
        near(
            r.base_cone_angle_rad,
            (delta.sin() * 20_f64.to_radians().cos()).asin(),
        );
        near(r.tip_cone_angle_rad, delta + (1. / distance).atan());
        near(r.root_cone_angle_rad, delta - (1.25 / distance).atan());
        near(r.nominal_tooth_thickness_mm, PI / 2.);
        near(r.physical_tooth_thickness_mm, PI / 2. - 0.05);
        for (radius, axial, station) in [
            (r.heel_root_radius_mm, r.heel_root_axial_mm, distance),
            (r.heel_tip_radius_mm, r.heel_tip_axial_mm, distance),
            (r.toe_root_radius_mm, r.toe_root_axial_mm, distance - 3.),
            (r.toe_tip_radius_mm, r.toe_tip_axial_mm, distance - 3.),
        ] {
            near(radius * delta.sin() + axial * delta.cos(), station);
        }
        near(r.heel_tip_radius_mm, teeth / 2. + delta.cos());
        near(r.heel_root_radius_mm, teeth / 2. - 1.25 * delta.cos());
        for direction in [
            r.root_center_direction,
            r.flank_tangent_direction,
            r.root_tangent_direction,
        ] {
            near(dot(direction, direction), 1.);
        }
        near(
            dot(r.root_center_direction, r.flank_tangent_direction),
            r.root_rounding_angle_rad.cos(),
        );
        near(
            dot(r.root_center_direction, r.root_tangent_direction),
            r.root_rounding_angle_rad.cos(),
        );
        near(r.root_tangent_direction[2].acos(), r.root_cone_angle_rad);
        near(
            r.root_center_direction[2].acos(),
            r.root_cone_angle_rad + r.root_rounding_angle_rad,
        );
        near(r.root_fillet_radius_mm, 0.2);
        near(r.root_rounding_angle_rad.sin() * distance, 0.2);
        assert!(r.toe_root_radius_mm - r.bore_radius_mm >= r.minimum_wall_mm);
        assert!(r.flank_start_gamma_rad < delta);
    }
    assert!(
        branches.contains(&BevelRootBranch::Meridian)
            && branches.contains(&BevelRootBranch::Involute)
    );
    let legacy = serde_json::to_value(GearParameters::default()).unwrap();
    assert!(legacy.get("bevel").is_none());
    assert_eq!(
        serde_json::from_value::<GearParameters>(legacy).unwrap(),
        GearParameters::default()
    );
    assert!(
        serde_json::to_value(GearParameters {
            family: GearFamily::Rack,
            rack: Some(RackDimensions {
                backing_thickness: length(3.)
            }),
            ..Default::default()
        })
        .unwrap()
        .get("bevel")
        .is_none()
    );
}
#[test]
fn unsupported_systems_impossible_cones_and_wrong_units_are_refused_atomically() {
    let s = session();
    let before = s.document().clone();
    let mut cases = vec![
        bevel(23., 24.),
        bevel(24., 65.),
        bevel(24.5, 24.),
        GearParameters {
            bevel: None,
            ..bevel(24., 24.)
        },
        GearParameters {
            family: GearFamily::Spur,
            ..bevel(24., 24.)
        },
        GearParameters {
            face_width: length(6.),
            ..bevel(24., 24.)
        },
        GearParameters {
            pressure_angle: angle(21.),
            ..bevel(24., 24.)
        },
        GearParameters {
            bore_diameter: length(30.),
            ..bevel(24., 24.)
        },
        GearParameters {
            minimum_wall: length(15.),
            ..bevel(24., 24.)
        },
        GearParameters {
            helix_angle: angle(1.),
            ..bevel(24., 24.)
        },
        GearParameters {
            hand: GearHand::Right,
            ..bevel(24., 24.)
        },
        GearParameters {
            center_gap: length(1.),
            ..bevel(24., 24.)
        },
        GearParameters {
            outer_diameter: Some(length(40.)),
            ..bevel(24., 24.)
        },
        GearParameters {
            rack: Some(RackDimensions {
                backing_thickness: length(3.),
            }),
            ..bevel(24., 24.)
        },
        GearParameters {
            backlash: length(2.),
            ..bevel(24., 24.)
        },
        GearParameters {
            profile_shift: Dimension::Literal {
                value: 0.1,
                unit: "ratio".into(),
            },
            ..bevel(24., 24.)
        },
    ];
    for system in BevelToothSystem::ALL.into_iter().filter(|s| !s.supported()) {
        let mut p = bevel(24., 24.);
        p.bevel.as_mut().unwrap().tooth_system = system;
        cases.push(p);
    }
    for bad in [angle(80.), length(90.)] {
        let mut p = bevel(24., 24.);
        p.bevel.as_mut().unwrap().shaft_angle = bad;
        cases.push(p);
    }
    let mut p = bevel(24., 24.);
    p.bevel.as_mut().unwrap().mate_teeth = length(24.);
    cases.push(p);
    for p in cases {
        assert!(prepare_gear(&s, 0, actor(), &GearRequest::new("Invalid", p)).is_err());
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
}
#[test]
fn complete_pair_sources_refuse_conflicting_inputs_and_recompute_intersecting_shaft_poses() {
    let s = session();
    let mut req = pair();
    req.parameters.frame = PrimitiveFrame {
        origin: [length(10.), length(20.), length(30.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let base = req
        .parameters
        .resolve_recipes(req.gears.each_ref().map(|g| &g.parameters), s.parameters())
        .unwrap();
    assert!(base.rack_travel.is_none());
    let context = base.bevel.as_ref().unwrap();
    near(context.shaft_angle_rad, PI / 2.);
    near(context.pitch_cone_angles_rad.iter().sum(), PI / 2.);
    near(base.center_distance_mm, 0.);
    for mating in base.mating {
        assert_eq!(mating.origin_mm, [10., 20., 30.]);
    }
    let normal = |frame: multicad_core::references::ReferenceFrame| {
        [
            frame.u[1] * frame.v[2] - frame.u[2] * frame.v[1],
            frame.u[2] * frame.v[0] - frame.u[0] * frame.v[2],
            frame.u[0] * frame.v[1] - frame.u[1] * frame.v[0],
        ]
    };
    near(dot(normal(base.mating[0]), normal(base.mating[1])), 0.);
    near(normal(base.mating[0])[0], 1.);
    near(normal(base.mating[1])[1], 1.);
    req.parameters.drive_angle = angle(11.);
    let driven = req
        .parameters
        .resolve_recipes(req.gears.each_ref().map(|g| &g.parameters), s.parameters())
        .unwrap();
    near(driven.phases_rad[0], 11_f64.to_radians());
    near(
        driven.phases_rad[1],
        PI - PI / 48. - 11_f64.to_radians() / 2.,
    );
    let mut conflict = req.clone();
    conflict.gears[0]
        .parameters
        .bevel
        .as_mut()
        .unwrap()
        .mate_teeth = count(24.);
    assert!(prepare_gear_pair(&s, 0, actor(), &conflict).is_err());
    let mut same_value_different_source = req.clone();
    same_value_different_source.gears[0]
        .parameters
        .bevel
        .as_mut()
        .unwrap()
        .shaft_angle = Dimension::Literal {
        value: PI / 2.,
        unit: "rad".into(),
    };
    assert!(prepare_gear_pair(&s, 0, actor(), &same_value_different_source).is_err());
    let mut mismatch = req.clone();
    mismatch.gears[1].parameters.module = length(2.);
    assert!(prepare_gear_pair(&s, 0, actor(), &mismatch).is_err());
    let mut mixed = req.clone();
    mixed.gears[1].parameters = GearParameters::default();
    assert!(prepare_gear_pair(&s, 0, actor(), &mixed).is_err());
    prepare_gear_pair(&s, 0, actor(), &req).unwrap();
    assert_eq!(s.revision(), 0);
}
#[test]
fn typed_mate_count_and_shaft_dependencies_survive_parameter_updates_without_copied_numeric_values()
{
    let mut s = session();
    let a = Entity::new(
        "zA",
        EntityData::Parameter {
            expression: "24".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let b = Entity::new(
        "zB",
        EntityData::Parameter {
            expression: "48".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let shaft = Entity::new(
        "shaft",
        EntityData::Parameter {
            expression: "90 deg".into(),
            unit: "deg".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let ids = [a.id, b.id, shaft.id];
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Inputs".into(),
        edits: vec![
            Mutation::Insert(a),
            Mutation::Insert(b),
            Mutation::Insert(shaft),
        ],
    })
    .unwrap();
    let bound = |name: &str, unit: &str| Dimension::Expression {
        expression: BoundExpression::bind(s.document(), name, unit, None).unwrap(),
    };
    let mut recipes = [bevel(24., 48.), bevel(48., 24.)];
    recipes[0].teeth = bound("=$zA", "count");
    recipes[1].teeth = bound("=$zB", "count");
    let shaft_source = bound("=$shaft", "deg");
    let coupled = couple_bevel_pair_recipes(&recipes, &shaft_source).unwrap();
    assert_eq!(
        coupled[0].bevel.as_ref().unwrap().mate_teeth,
        recipes[1].teeth
    );
    assert_eq!(
        coupled[1].bevel.as_ref().unwrap().mate_teeth,
        recipes[0].teeth
    );
    assert_eq!(coupled[0].bevel.as_ref().unwrap().shaft_angle, shaft_source);
    let req = GearPairRequest::new(
        "Expression bevel pair",
        coupled.map(|p| GearRequest::new("Member", p)),
    );
    s.commit(prepare_gear_pair(&s, 1, actor(), &req).unwrap().prepared)
        .unwrap();
    for gear in &req.gears {
        let EntityData::Feature { inputs, .. } = &s.document().get(gear.feature).unwrap().data
        else {
            panic!()
        };
        for id in ids {
            assert!(inputs.iter().any(|input| input.target == id));
        }
    }
    let before =
        resolve_gear_pair_parameters(s.document(), s.parameters(), &req.parameters).unwrap();
    let mut parameter = s.document().get(ids[1]).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "40".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Mate count".into(),
        edits: vec![Mutation::Replace(parameter)],
    })
    .unwrap();
    let after =
        resolve_gear_pair_parameters(s.document(), s.parameters(), &req.parameters).unwrap();
    assert_ne!(
        before.gears[0].bevel().unwrap().pitch_cone_angle_rad,
        after.gears[0].bevel().unwrap().pitch_cone_angle_rad
    );
    assert_ne!(
        before.gears[1].bevel().unwrap().pitch_cone_angle_rad,
        after.gears[1].bevel().unwrap().pitch_cone_angle_rad
    );
    assert_ne!(before.mating[1].u, after.mating[1].u);
    assert_eq!(after.gears[0].bevel().unwrap().mate_teeth, 40);
    assert_eq!(
        read_gear(s.document(), req.gears[0].feature)
            .unwrap()
            .parameters
            .bevel
            .as_ref()
            .unwrap()
            .mate_teeth,
        recipes[1].teeth
    );
}
#[test]
fn grouped_edits_keep_pair_identity_and_history_while_individual_coupled_edits_refuse() {
    let mut s = session();
    let req = pair();
    let before = s.document().clone();
    let first = prepare_gear_pair(&s, 0, actor(), &req).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(first.prepared).unwrap();
    let original = s.document().clone();
    let mut recipes = req.gears.each_ref().map(|g| g.parameters.clone());
    recipes[0].teeth = count(32.);
    assert!(prepare_gear_edit(&s, 1, actor(), req.gears[0].feature, &recipes[0]).is_err());
    assert_eq!(s.document(), &original);
    let recipes = couple_bevel_pair_recipes(&recipes, &angle(90.)).unwrap();
    let edited =
        prepare_gear_pair_geometry_edit(&s, 1, actor(), req.pair, &req.parameters, &recipes)
            .unwrap();
    s.commit(edited.prepared).unwrap();
    let source = read_gear_pair(s.document(), req.pair).unwrap();
    assert_eq!(source.outputs, req.outputs);
    let resolved =
        resolve_gear_pair_parameters(s.document(), s.parameters(), &source.parameters).unwrap();
    assert_eq!(resolved.gears[1].bevel().unwrap().mate_teeth, 32);
    let saved = s.document().clone();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopen = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopen.project.document, saved);
    let independent = Session::new(reopen.project.document).unwrap();
    assert_eq!(
        read_gear_pair(independent.document(), req.pair)
            .unwrap()
            .outputs,
        req.outputs
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
}
fn reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(BEVEL_GEAR_CAPABILITY));
    assert!(project.capabilities().contains(GEAR_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(BEVEL_GEAR_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(BEVEL_GEAR_CAPABILITY)
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
fn bevel_reader_is_required_for_direct_suppressed_and_template_only_retained_sources() {
    let mut s = session();
    let req = GearRequest::new("Bevel", bevel(24., 24.));
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
        SymbolAuthorRequest::new("Reusable bevel", req.body, SymbolKind::Static, vec![], None);
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
            .contains(BEVEL_GEAR_CAPABILITY)
    );
}
