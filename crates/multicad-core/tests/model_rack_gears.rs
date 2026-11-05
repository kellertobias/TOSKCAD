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
    Session::new(Document::new("Rack source").unwrap()).unwrap()
}
fn count(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "count".into(),
    }
}
fn rack(cells: f64) -> GearParameters {
    GearParameters {
        family: GearFamily::Rack,
        rack: Some(RackDimensions {
            backing_thickness: length(3.),
        }),
        module: length(1.),
        teeth: count(cells),
        bore_diameter: length(0.),
        face_width: length(4.),
        ..Default::default()
    }
}
fn pinion() -> GearParameters {
    GearParameters {
        module: length(1.),
        face_width: length(4.),
        ..Default::default()
    }
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn pair(reverse: bool, cells: f64) -> GearPairRequest {
    let mut members = [
        GearRequest::new("Rack", rack(cells)),
        GearRequest::new("Pinion", pinion()),
    ];
    if reverse {
        members.swap(0, 1);
    }
    GearPairRequest::new("Rack and pinion", members)
}
#[test]
fn straight_rack_dimensions_use_independent_thinning_and_tangent_custom_root_circles() {
    let s = session();
    let parameters = GearParameters {
        backlash: length(0.1),
        fabrication_allowance: length(0.03),
        ..rack(20.)
    };
    assert!(
        parameters
            .resolve(s.parameters())
            .unwrap_err()
            .to_string()
            .contains("no rotary radii")
    );
    let geometry = parameters.resolve_geometry(s.parameters()).unwrap();
    assert_eq!(geometry.family(), GearFamily::Rack);
    assert_eq!(geometry.teeth(), 20);
    assert!(geometry.rotary().is_none());
    let r = geometry.rack().unwrap();
    close(r.pitch_mm, PI);
    close(r.length_mm, 20. * PI);
    close(r.addendum_mm, 1.);
    close(r.dedendum_mm, 1.25);
    close(r.nominal_tooth_thickness_mm, PI / 2.);
    close(r.design_tooth_thickness_mm, PI / 2. - 0.1);
    close(r.physical_tooth_thickness_mm, PI / 2. - 0.13);
    close(
        r.tip_width_mm,
        PI / 2. - 0.13 - 2. * 20_f64.to_radians().tan(),
    );
    let a = r.pressure_angle_rad;
    let f = r.root_fillet_radius_mm;
    let x = r.fillet_center_x_mm - f * a.cos();
    let y = r.fillet_center_y_mm - f * a.sin();
    // Independent line and circle equations at the shared tangent point.
    close(x + y * a.tan(), r.physical_tooth_thickness_mm / 2.);
    close(
        (x - r.fillet_center_x_mm).hypot(y - r.fillet_center_y_mm),
        f,
    );
    close(r.fillet_center_y_mm - f, -r.dedendum_mm);
    close(r.root_flat_width_mm, r.pitch_mm - 2. * r.fillet_center_x_mm);
    assert!(r.root_flat_width_mm > 0. && r.tip_width_mm > 0.);
    for module_system in [GearModuleSystem::Normal, GearModuleSystem::Transverse] {
        let p = GearParameters {
            module_system,
            ..parameters.clone()
        };
        assert_eq!(p.resolve_geometry(s.parameters()).unwrap(), geometry);
    }
    // Both ends of the bounded native ancestry envelope remain supported.
    for cells in [4., 64.] {
        let geometry = rack(cells).resolve_geometry(s.parameters()).unwrap();
        assert_eq!(geometry.teeth(), cells as u32);
        close(geometry.rack().unwrap().length_mm, cells * PI);
    }
    let legacy = serde_json::to_value(GearParameters::default()).unwrap();
    assert!(legacy.get("rack").is_none());
    assert_eq!(
        serde_json::from_value::<GearParameters>(legacy).unwrap(),
        GearParameters::default()
    );
}
#[test]
fn unsupported_or_impossible_racks_and_pairs_are_atomic() {
    let s = session();
    let before = s.document().clone();
    let mut cases = vec![
        GearParameters {
            rack: None,
            ..rack(20.)
        },
        GearParameters {
            family: GearFamily::Spur,
            ..rack(20.)
        },
        rack(3.),
        rack(20.5),
        rack(65.),
        GearParameters {
            bore_diameter: length(1.),
            ..rack(20.)
        },
        GearParameters {
            outer_diameter: Some(length(10.)),
            ..rack(20.)
        },
        GearParameters {
            helix_angle: angle(5.),
            ..rack(20.)
        },
        GearParameters {
            hand: GearHand::Left,
            ..rack(20.)
        },
        GearParameters {
            center_gap: length(1.),
            ..rack(20.)
        },
        GearParameters {
            backlash: length(1.),
            ..rack(20.)
        },
        GearParameters {
            minimum_wall: length(4.),
            ..rack(20.)
        },
        GearParameters {
            rack: Some(RackDimensions {
                backing_thickness: angle(3.),
            }),
            ..rack(20.)
        },
        GearParameters {
            root_fillet_ratio: Dimension::Literal {
                value: 0.21,
                unit: "ratio".into(),
            },
            ..rack(20.)
        },
    ];
    cases.push(GearParameters {
        profile_shift: Dimension::Literal {
            value: 0.1,
            unit: "ratio".into(),
        },
        ..rack(20.)
    });
    for p in cases {
        assert!(prepare_gear(&s, 0, actor(), &GearRequest::new("Invalid", p)).is_err());
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
    let short = pair(false, 4.);
    assert!(prepare_gear_pair(&s, 0, actor(), &short).is_err());
    for index in 0..4 {
        let mut bad = pair(false, 20.);
        match index {
            0 => bad.gears[1].parameters.module = length(2.),
            1 => bad.gears[1].parameters.face_width = length(2.),
            2 => bad.gears[1].parameters.pressure_angle = angle(25.),
            _ => {
                bad.gears[1].parameters.family = GearFamily::Helical;
                bad.gears[1].parameters.helix_angle = angle(10.);
                bad.gears[1].parameters.hand = GearHand::Right;
            }
        }
        assert!(prepare_gear_pair(&s, 0, actor(), &bad).is_err());
        assert_eq!(s.document(), &before);
    }
}
#[test]
fn either_pair_role_derives_centered_phase_slide_and_both_finite_end_limits() {
    let s = session();
    for reverse in [false, true] {
        for cells in [20., 21.] {
            let mut request = pair(reverse, cells);
            request.parameters.frame = PrimitiveFrame {
                origin: [length(100.), length(200.), length(300.)],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            };
            let recipes = request.gears.each_ref().map(|g| &g.parameters);
            let base = request
                .parameters
                .resolve_recipes(recipes, s.parameters())
                .unwrap();
            let travel = base.rack_travel.as_ref().unwrap();
            let ri = usize::from(reverse);
            let pi = 1 - ri;
            assert_eq!(travel.rack_member.index(), ri);
            assert_eq!(travel.pinion_member.index(), pi);
            close(base.center_distance_mm, 12.);
            close(travel.max_translation_mm, cells * PI / 2. - 13.);
            close(travel.min_translation_mm, -travel.max_translation_mm);
            close(travel.pinion_period_rad, 2. * PI / 24.);
            close(
                base.phases_rad[pi],
                -PI / 2. - if cells == 21. { PI / 24. } else { 0. },
            );
            for drive in [-travel.max_drive_angle_rad, 0.1, travel.max_drive_angle_rad] {
                let mut parameters = request.parameters.clone();
                parameters.drive_angle = angle(drive.to_degrees());
                let driven = parameters.resolve_recipes(recipes, s.parameters()).unwrap();
                close(driven.rack_travel.unwrap().translation_mm, 12. * drive);
                close(driven.phases_rad[ri], 0.);
                close(driven.phases_rad[pi] - base.phases_rad[pi], drive);
                close(driven.mating[ri].origin_mm[1], 200. + 12. * drive);
                close(driven.mating[pi].origin_mm[2], 312.);
                close(driven.mating[pi].origin_mm[1], 200.);
            }
            for drive in [
                travel.min_drive_angle_rad - 1e-6,
                travel.max_drive_angle_rad + 1e-6,
            ] {
                let mut parameters = request.parameters.clone();
                parameters.drive_angle = angle(drive.to_degrees());
                assert!(parameters.resolve_recipes(recipes, s.parameters()).is_err());
            }
        }
    }
}
fn check_reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(RACK_GEAR_CAPABILITY));
    assert!(project.capabilities().contains(GEAR_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(RACK_GEAR_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(RACK_GEAR_CAPABILITY)
    );
    let read = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(read.project.document, *s.document());
    Session::new(read.project.document).unwrap();
}
#[test]
fn rack_backing_dependencies_and_source_reader_survive_suppression_and_template_only_import() {
    let mut s = session();
    let parameter = Entity::new(
        "backing",
        EntityData::Parameter {
            expression: "3 mm".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Backing parameter".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let p = GearParameters {
        rack: Some(RackDimensions {
            backing_thickness: Dimension::Expression {
                expression: BoundExpression::bind(s.document(), "=$backing", "mm", None).unwrap(),
            },
        }),
        ..rack(20.)
    };
    let request = GearRequest::new("Rack", p);
    s.commit(
        prepare_gear(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let EntityData::Feature { inputs, .. } = &s.document().get(request.feature).unwrap().data
    else {
        panic!()
    };
    assert!(inputs.iter().any(|input| input.target == id));
    check_reader(&s);
    let mut feature = s.document().get(request.feature).unwrap().clone();
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
    check_reader(&s);
    s.undo(s.revision(), actor()).unwrap();
    let author = SymbolAuthorRequest::new(
        "Reusable rack",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
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
    check_reader(&receiver);
    assert!(
        !ProjectFile::new(session().document().clone())
            .unwrap()
            .capabilities()
            .contains(RACK_GEAR_CAPABILITY)
    );
}
#[test]
fn pair_edit_keeps_all_outputs_and_updates_shared_joint_poses_with_one_history_step() {
    let mut s = session();
    let request = pair(false, 20.);
    let before = s.document().clone();
    let draft = prepare_gear_pair(&s, 0, actor(), &request).unwrap();
    assert_eq!(s.document(), &before);
    s.commit(draft.prepared).unwrap();
    let original = s.document().clone();
    let mut driven = request.parameters.clone();
    driven.drive_angle = angle(10.);
    let edited = prepare_gear_pair_edit(&s, 1, actor(), request.pair, &driven).unwrap();
    s.commit(edited.prepared).unwrap();
    assert_eq!(
        read_gear_pair(s.document(), request.pair).unwrap().outputs,
        request.outputs
    );
    let pose =
        resolve_instance_pose(s.document(), request.outputs.instances[0], s.parameters()).unwrap();
    close(pose.translation_mm[0], 12. * 10_f64.to_radians());
    check_reader(&s);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    let good = s.document().clone();
    let mut invalid = driven;
    invalid.drive_angle = angle(1000.);
    assert!(prepare_gear_pair_edit(&s, s.revision(), actor(), request.pair, &invalid).is_err());
    assert_eq!(s.document(), &good);
}
