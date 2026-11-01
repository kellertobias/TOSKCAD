use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyOutput, LayeredWallTypeDefinition, WallChange, WallDefinition, WallEnd,
        WallJunction, WallJunctionStyle, WallLayer, WallLayerKind, WallLayerPlacement,
        WallLayerReference, WallLayerStack, WallRequest, length,
    },
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn value(material: EntityId) -> LayeredWallTypeDefinition {
    LayeredWallTypeDefinition {
        layers: WallLayerStack {
            layers: vec![
                WallLayer {
                    id: EntityId::new(),
                    name: "Outer brick".into(),
                    role: None,
                    construction: WallLayerKind::Solid {
                        thickness: length(100.),
                        material,
                    },
                },
                WallLayer {
                    id: EntityId::new(),
                    name: "Air".into(),
                    role: None,
                    construction: WallLayerKind::Cavity {
                        thickness: length(30.),
                    },
                },
                WallLayer {
                    id: EntityId::new(),
                    name: "Lining".into(),
                    role: None,
                    construction: WallLayerKind::Solid {
                        thickness: length(20.),
                        material,
                    },
                },
            ],
        },
        height: length(2700.),
        reference: WallLayerReference::Center,
    }
}
fn request(ty: EntityId, points: &[[f64; 2]]) -> WallRequest {
    WallRequest::new(
        "Wall",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty,
            layer_placement: Some(WallLayerPlacement::default()),
            path: points.iter().map(|p| p.map(length)).collect(),
            closed: false,
            level: None,
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    )
}
fn fixture() -> (Session, WallRequest, WallRequest) {
    let material = Entity::new(
        "Brick",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 80, 60, 255],
            section_hatch: None,
        },
    );
    let mut session = Session::new(
        Document::new("T joins")
            .unwrap()
            .edited(&[Mutation::Insert(material.clone())])
            .unwrap(),
    )
    .unwrap();
    let (host_type, p) =
        model::prepare_layered_wall_type(&session, 0, actor(), "Host", &value(material.id))
            .unwrap();
    session.commit(p).unwrap();
    let (branch_type, p) = model::prepare_layered_wall_type(
        &session,
        session.revision(),
        actor(),
        "Branch",
        &value(material.id),
    )
    .unwrap();
    session.commit(p).unwrap();
    let host = request(host_type, &[[0., 0.], [4000., 0.]]);
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&host),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut branch = request(branch_type, &[[2000., 3000.], [2000., 0.]]);
    branch
        .wall
        .layer_placement
        .as_mut()
        .unwrap()
        .junctions
        .push(WallJunction {
            end: WallEnd::End,
            host_wall: host.feature,
            host_segment: 0,
            host_point_count: host.wall.path.len(),
            host_closed: host.wall.closed,
            style: WallJunctionStyle::FlushButt,
        });
    (session, host, branch)
}
fn cap(plan: &model::WallPlan, end: WallEnd) -> [[f64; 2]; 2] {
    let i = plan
        .outer
        .roles
        .iter()
        .position(|r| r == &format!("wall.{}", end.label()))
        .unwrap();
    [
        plan.outer.points[i],
        plan.outer.points[(i + 1) % plan.outer.points.len()],
    ]
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
#[test]
fn both_ends_trim_to_the_physical_host_face_keep_roles_and_persist_with_a_capability_gate() {
    for end in [WallEnd::Start, WallEnd::End] {
        let (mut session, host, mut branch) = fixture();
        if end == WallEnd::Start {
            branch.wall.path.reverse();
            branch.wall.layer_placement.as_mut().unwrap().flipped = true;
            branch.wall.layer_placement.as_mut().unwrap().junctions[0].end = end;
        }
        let before = session.document().clone();
        let p = model::prepare_wall_change(
            &session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Create(&branch),
        )
        .unwrap();
        assert_eq!(session.document(), &before);
        let resolved =
            model::resolve_wall(p.prepared.document(), p.prepared.parameters(), &branch.wall)
                .unwrap();
        assert_eq!(resolved.junctions.len(), 1);
        near(resolved.junctions[0].end_extension_mm, -75.);
        for point in cap(&resolved.plan().unwrap(), end) {
            near(point[1], 75.);
        }
        let (start, finish) = resolved.usable_segment_range(0).unwrap();
        if end == WallEnd::Start {
            near(start, 75.);
            near(finish, 3000.);
        } else {
            near(start, 0.);
            near(finish, 2925.);
        }
        assert!(
            branch
                .wall
                .inputs()
                .iter()
                .any(|link| link.target == host.feature
                    && link.role == format!("junction/{}/host", end.label()))
        );
        session.commit(p.prepared).unwrap();
        let file = ProjectFile::new(session.document().clone()).unwrap();
        assert!(
            file.capabilities()
                .contains(model::WALL_JUNCTION_CAPABILITY)
        );
        let mut bytes = vec![];
        project_file::write(&file, &mut bytes, Default::default()).unwrap();
        let mut old = project_file::supported_capabilities();
        old.remove(model::WALL_JUNCTION_CAPABILITY);
        assert!(
            project_file::read(&mut bytes.as_slice(), Default::default(), &old)
                .unwrap_err()
                .contains(model::WALL_JUNCTION_CAPABILITY)
        );
        let opened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(&opened.document, session.document());
        let mut forged = session.document().get(branch.feature).unwrap().clone();
        if let EntityData::Feature { inputs, .. } = &mut forged.data {
            inputs.retain(|l| !l.role.starts_with("junction/"));
        }
        assert!(
            session
                .document()
                .edited(&[Mutation::Replace(forged)])
                .is_err()
        );
    }
}
#[test]
fn host_thickness_updates_move_the_termination_and_exterior_air_extends_the_branch_without_solid_inflation()
 {
    let (mut session, host, branch) = fixture();
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let model::WallTypeConstruction::Layered(mut host_type) =
        model::read_wall_construction(session.document(), host.wall.wall_type).unwrap()
    else {
        unreachable!()
    };
    host_type.layers.layers[0].construction = WallLayerKind::Solid {
        thickness: length(140.),
        material: host_type.layers.layers[0].construction.material().unwrap(),
    };
    let p = model::prepare_layered_wall_type_edit(
        &session,
        session.revision(),
        actor(),
        host.wall.wall_type,
        &host_type,
    )
    .unwrap();
    let resolved = model::resolve_wall(p.document(), p.parameters(), &branch.wall).unwrap();
    for point in cap(&resolved.plan().unwrap(), WallEnd::End) {
        near(point[1], 95.);
    }
    host_type.reference = WallLayerReference::Outside;
    host_type.layers.layers.insert(
        0,
        WallLayer {
            id: EntityId::new(),
            name: "Exterior air".into(),
            role: None,
            construction: WallLayerKind::Cavity {
                thickness: length(30.),
            },
        },
    );
    let p = model::prepare_layered_wall_type_edit(
        &session,
        session.revision(),
        actor(),
        host.wall.wall_type,
        &host_type,
    )
    .unwrap();
    let resolved = model::resolve_wall(p.document(), p.parameters(), &branch.wall).unwrap();
    near(resolved.junctions[0].end_extension_mm, 30.);
    for point in cap(&resolved.plan().unwrap(), WallEnd::End) {
        near(point[1], -30.);
    }
    let material_width = resolved
        .layers
        .as_ref()
        .unwrap()
        .stack
        .layers
        .iter()
        .filter(|l| l.solid)
        .map(|l| l.thickness_mm)
        .sum::<f64>();
    near(material_width, 120.);
}
#[test]
fn unsupported_angle_datum_corner_height_and_duplicate_end_fail_without_committing() {
    let (session, host, branch) = fixture();
    let before = session.document().clone();
    let mut cases = vec![];
    let mut wrong = branch.wall.clone();
    wrong.path[0][0] = length(1900.);
    cases.push(wrong);
    let mut wrong = branch.wall.clone();
    wrong.path[1][1] = length(1.);
    cases.push(wrong);
    let mut wrong = branch.wall.clone();
    wrong.path[0][0] = length(20.);
    wrong.path[1][0] = length(20.);
    cases.push(wrong);
    let mut wrong = branch.wall.clone();
    wrong.height = Some(length(3000.));
    cases.push(wrong);
    let mut wrong = branch.wall.clone();
    wrong
        .layer_placement
        .as_mut()
        .unwrap()
        .junctions
        .push(WallJunction {
            end: WallEnd::End,
            host_wall: host.feature,
            host_segment: 0,
            host_point_count: host.wall.path.len(),
            host_closed: host.wall.closed,
            style: WallJunctionStyle::FlushButt,
        });
    cases.push(wrong);
    for wall in cases {
        let request = WallRequest {
            feature: branch.feature,
            body: branch.body,
            name: branch.name.clone(),
            wall,
        };
        assert!(
            model::prepare_wall_change(
                &session,
                session.revision(),
                actor(),
                &[],
                &WallChange::Create(&request)
            )
            .is_err()
        );
        assert_eq!(session.document(), &before);
    }
}

#[test]
fn impact_lists_include_joined_branches_and_explicit_alignment_preserves_other_points() {
    let (mut session, host, mut branch) = fixture();
    branch.wall.path[1] = [length(2050.), length(80.)];
    let before = branch.wall.path.clone();
    let junction = &branch.wall.layer_placement.as_ref().unwrap().junctions[0];
    let aligned = junction
        .aligned_endpoint(
            session.document(),
            session.parameters(),
            &branch.wall.path,
            false,
        )
        .unwrap();
    assert_eq!(aligned, [2000., 0.]);
    assert_eq!(branch.wall.path, before);
    branch.wall.path[1] = aligned.map(length);
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let affected = model::wall_construction_users(session.document(), host.wall.wall_type).unwrap();
    assert_eq!(affected.len(), 2);
    assert!(affected.iter().any(|u| u.body == branch.body));
    let mut changed_host = host.wall.clone();
    changed_host.path.push([length(4000.), length(-2000.)]);
    let p = model::prepare_wall_edit(
        &session,
        session.revision(),
        actor(),
        host.feature,
        &changed_host,
    )
    .unwrap();
    let error = model::resolve_wall(p.prepared.document(), p.prepared.parameters(), &branch.wall)
        .unwrap_err();
    assert!(error.to_string().contains("topology changed"));
}

#[test]
fn equipment_in_the_trimmed_end_needs_repair_while_intact_mounting_points_remain_valid() {
    use multicad_core::equipment::{Attachment, HostPlacement, WallSide};
    let (mut session, _, branch) = fixture();
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let (floor, p) = model::prepare_level(
        &session,
        session.revision(),
        actor(),
        "Ground",
        &model::LevelDefinition {
            elevation: length(0.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    session.commit(p).unwrap();
    let anchor = |along| Attachment {
        owner: EntityId::new(),
        floor,
        placement: HostPlacement::Wall {
            wall: branch.feature,
            segment: 0,
            point_count: 2,
            closed: false,
            side: WallSide::Left,
            along: length(along),
            standoff: length(0.),
            height: length(300.),
            rotation: model::angle(0.),
        },
    };
    let error = anchor(2990.)
        .resolve(session.document(), session.parameters())
        .unwrap_err();
    assert!(error.to_string().contains("junction"));
    assert!(
        anchor(2800.)
            .resolve(session.document(), session.parameters())
            .is_ok()
    );
}

#[test]
fn host_voids_and_branch_end_openings_require_explicit_junction_repair() {
    let (mut session, host, branch) = fixture();
    let door = model::NewType::new(
        "Door",
        model::NewTypeKind::Opening(model::OpeningTypeDefinition {
            kind: model::OpeningKind::Door {
                hinge: model::Hinge::Start,
                swing: model::Swing::Left,
            },
            width: length(900.),
            height: length(2100.),
            sill: length(0.),
        }),
    );
    let mut opened = host.wall.clone();
    opened.openings.push(model::Opening {
        id: EntityId::new(),
        opening_type: door.id,
        segment: 0,
        offset: length(1600.),
        width: None,
        height: None,
        sill: None,
    });
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&door),
        &WallChange::Edit {
            feature: host.feature,
            wall: &opened,
        },
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let before = session.document().clone();
    let error = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap_err();
    assert!(error.message.contains("door/window"));
    assert_eq!(session.document(), &before);
    let p = model::prepare_wall_edit(
        &session,
        session.revision(),
        actor(),
        host.feature,
        &host.wall,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut chased = host.wall.clone();
    chased.cutouts.push(model::VerticalCutout {
        id: EntityId::new(),
        segment: 0,
        offset: length(1900.),
        width: length(200.),
        depth: Some(length(20.)),
        face: model::WallFace::Left,
    });
    let p = model::prepare_wall_edit(&session, session.revision(), actor(), host.feature, &chased)
        .unwrap();
    session.commit(p.prepared).unwrap();
    let before = session.document().clone();
    let error = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap_err();
    assert!(error.message.contains("chase/cutout"));
    assert_eq!(session.document(), &before);
    let p = model::prepare_wall_edit(
        &session,
        session.revision(),
        actor(),
        host.feature,
        &host.wall,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut opening_at_end = branch.wall.clone();
    opening_at_end.openings.push(model::Opening {
        id: EntityId::new(),
        opening_type: door.id,
        segment: 0,
        offset: length(2700.),
        width: Some(length(200.)),
        height: None,
        sill: None,
    });
    let request = WallRequest {
        feature: branch.feature,
        body: branch.body,
        name: branch.name,
        wall: opening_at_end,
    };
    let before = session.document().clone();
    assert!(
        model::prepare_wall_change(
            &session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Create(&request)
        )
        .unwrap_err()
        .message
        .contains("does not fit")
    );
    assert_eq!(session.document(), &before);
}
