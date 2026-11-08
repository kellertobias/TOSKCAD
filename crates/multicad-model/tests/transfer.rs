use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind},
    symbols::{self, SymbolPlaceRequest},
};
use multicad_model::{Compiler, Model};
use std::f64::consts::PI;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-4, "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn bound(s: &Session, text: &str, unit: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), text, unit, None).unwrap(),
    }
}
fn commit(s: &mut Session, p: ModelPrepared) {
    Compiler::default()
        .compile(p.prepared.document(), s.revision())
        .unwrap();
    s.commit(p.prepared).unwrap();
}
fn top(m: &Model, producer: EntityId, owner: EntityId, z: f64) -> AlignmentInput {
    let mut snapshot = m.snapshot().shapes[&producer].topology.clone();
    snapshot.owner = owner;
    let selected=snapshot.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[2]>0.999&&(frame.origin_mm[2]-z).abs()<1e-7)).unwrap();
    AlignmentInput {
        source: owner,
        intent: GeometryIntent::capture(&snapshot, selected.key).unwrap(),
    }
}
struct Fixture {
    s: Session,
    request: TransferRequest,
    rail: PrimitiveRequest,
    rail_head: EntityId,
    source_member: ComponentPlacementRequest,
    outer: ComponentPlacementRequest,
}
fn fixture() -> Fixture {
    let mut doc = Document::new("Associative native rail transfer").unwrap();
    for (name, value, unit) in [
        ("pitch", "32", "mm"),
        ("count", "2", "count"),
        ("offset", "10", "mm"),
    ] {
        doc.insert(Entity::new(
            name,
            EntityData::Parameter {
                expression: value.into(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    }
    let mut s = Session::new(doc).unwrap();
    let rail = PrimitiveRequest::new(
        "Independent hardware",
        PrimitiveParameters::Box {
            size: [130., 20., 4.].map(length),
            frame: Default::default(),
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &rail).unwrap();
    commit(&mut s, p);
    let tool = PrimitiveRequest::new(
        "Hole seed",
        PrimitiveParameters::Cylinder {
            radius: length(3.),
            height: length(6.),
            frame: PrimitiveFrame {
                origin: [20., 10., -1.].map(length),
                ..Default::default()
            },
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &tool).unwrap();
    commit(&mut s, p);
    let pattern = AdvancedRequest::on_body(
        BodyInput {
            body: tool.body,
            producer: tool.feature,
        },
        AdvancedParameters::LinearPattern(LinearPatternParameters {
            count: bound(&s, "=$count", "count"),
            step: [bound(&s, "=$pitch", "mm"), length(0.), length(0.)],
            merge: false,
        }),
        vec![],
    );
    let p = prepare_advanced(&s, s.revision(), actor(), &pattern).unwrap();
    commit(&mut s, p);
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: rail.body,
            producer: rail.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: pattern.feature,
        }],
    );
    let p = prepare_boolean(&s, s.revision(), actor(), &cut).unwrap();
    commit(&mut s, p);
    let mut receivers = vec![];
    for (name, height, density) in [
        ("Wood", 18., 700.),
        ("Printed", 8., 1240.),
        ("Sheet", 2., 7850.),
    ] {
        let part = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [130., 30., height].map(length),
                frame: Default::default(),
            },
        );
        let p = prepare_primitive(&s, s.revision(), actor(), &part).unwrap();
        commit(&mut s, p);
        let mat = Entity::new(
            format!("{name} material"),
            EntityData::Material {
                density_kg_m3: Some(density),
                color_rgba: [120, 150, 180, 255],
                section_hatch: None,
            },
        );
        let mut body = s.document().get(part.body).unwrap().clone();
        let EntityData::Part { material, .. } = &mut body.data else {
            panic!()
        };
        *material = Some(mat.id);
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Explicit receiving material".into(),
            edits: vec![Mutation::Insert(mat), Mutation::Replace(body)],
        })
        .unwrap();
        receivers.push(part);
    }
    let mut members = vec![];
    for (body, z) in [
        (rail.body, 80.),
        (receivers[0].body, 0.),
        (receivers[1].body, 40.),
        (receivers[2].body, 60.),
    ] {
        let mut parameters = NumericTransform {
            translation: [length(0.), length(0.), length(z)],
            ..Default::default()
        };
        if body == rail.body {
            parameters.translation[0] = bound(&s, "=$offset", "mm");
        }
        let member = ComponentPlacementRequest::new(body, parameters);
        s.commit(
            prepare_place_component(&s, s.revision(), actor(), &member)
                .unwrap()
                .prepared,
        )
        .unwrap();
        members.push(member);
    }
    let assembly = AssemblyRequest::new(
        "Reusable source and receiving materials",
        members.iter().map(|m| m.instance).collect(),
        std::iter::once(rail.body)
            .chain(receivers.iter().map(|p| p.body))
            .collect(),
    );
    s.commit(
        prepare_assembly(&s, s.revision(), actor(), &assembly)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let outer = ComponentPlacementRequest::new(
        assembly.assembly,
        NumericTransform {
            translation: [300., 500., 100.].map(length),
            angle: angle(30.),
            ..Default::default()
        },
    );
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &outer)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let m = compile(&s);
    let source = top(&m, cut.feature, rail.body, 4.);
    let mut request = TransferRequest::new(
        "Rail drilling",
        TransferSource {
            component: rail.body,
            path: OccurrencePath {
                root: outer.instance,
                members: vec![members[0].instance],
            },
            selection: TransferSelection::FaceHoles {
                face: source.intent.id,
            },
        },
    );
    request.references.push(source);
    for (index, (part, height)) in receivers.iter().zip([18., 8., 2.]).enumerate() {
        let face = top(&m, part.feature, part.feature, height);
        let parameters = match index {
            0 => TransferTargetParameters {
                diameter: Some(length(5.)),
                depth: TransferDepth::Blind { depth: length(12.) },
                ..Default::default()
            },
            1 => TransferTargetParameters {
                clearance: length(0.2),
                ..Default::default()
            },
            _ => TransferTargetParameters {
                diameter: Some(length(6.5)),
                ..Default::default()
            },
        };
        request.targets.push(TransferTarget::new(
            BodyInput {
                body: part.body,
                producer: part.feature,
            },
            OccurrencePath {
                root: outer.instance,
                members: vec![members[index + 1].instance],
            },
            face.intent.id,
            parameters,
        ));
        request.references.push(face);
    }
    Fixture {
        s,
        request,
        rail,
        rail_head: cut.feature,
        source_member: members.remove(0),
        outer,
    }
}
fn parameter(s: &mut Session, name: &str, text: &str) {
    let mut value = s
        .document()
        .entities()
        .find(|e| e.name == name)
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut value.data else {
        panic!()
    };
    *expression = text.into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: format!("Change {name}"),
        edits: vec![Mutation::Replace(value)],
    })
    .unwrap();
}
fn verify(f: &Fixture, count: usize, centers: &[f64]) -> Model {
    let m = compile(&f.s);
    for (row, (height, radius, depth)) in
        f.request
            .targets
            .iter()
            .zip([(18., 2.5, 12.), (8., 3.2, 8.), (2., 3.25, 2.)])
    {
        assert_eq!(m.snapshot().transfer_counts.get(&row.feature), Some(&count));
        near(
            m.solid(row.input.body).unwrap().volume_mm3().unwrap(),
            130. * 30. * height - count as f64 * PI * radius * radius * depth,
        );
        let topology = &m.snapshot().shapes[&row.feature].topology;
        let mut x = topology
            .elements
            .iter()
            .filter_map(|e| match e.geometry {
                Some(AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    sweep_rad,
                    ..
                }) if e.key.kind == TopologyKind::Edge
                    && (radius_mm - radius).abs() < 1e-6
                    && (sweep_rad.abs() - 2. * PI).abs() < 1e-6
                    && (frame.origin_mm[2] - height).abs() < 1e-6 =>
                {
                    Some(frame.origin_mm[0])
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        x.sort_by(f64::total_cmp);
        x.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        assert_eq!(x.len(), centers.len());
        for (a, b) in x.into_iter().zip(centers) {
            near(a, *b);
        }
    }
    m.snapshot().transfer_evaluation().validate().unwrap();
    m
}
fn reopen(s: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    Session::new(opened.project.document).unwrap()
}
#[test]
fn pinned_library_washer_bore_transfers_and_follows_its_placement() {
    let mut bytes = include_bytes!("../../../library/washer-normal-m6.tcd").as_slice();
    let library = project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let [(_, definition)] = symbols::definitions(&library.project.document)
        .unwrap()
        .try_into()
        .expect("one shipped washer definition");
    let mut session = Session::new(Document::new("Library washer transfer").unwrap()).unwrap();
    let placement = SymbolPlaceRequest::from_definition(
        EntityId::new(),
        &definition,
        "M6 washer source",
        Default::default(),
        Placement {
            translation_mm: [15., 12., 15.],
            ..Default::default()
        },
    )
    .unwrap();
    let placed = symbols::prepare_library_place(
        &session,
        session.revision(),
        actor(),
        &definition,
        &placement,
        None,
    )
    .unwrap();
    session.commit(placed.prepared).unwrap();
    let pin = definition.pin().unwrap();
    assert_eq!(
        symbols::read_instance(session.document(), placement.instance)
            .unwrap()
            .pin,
        pin
    );

    let receiver = PrimitiveRequest::new(
        "Receiving plate",
        PrimitiveParameters::Box {
            size: [45., 30., 5.].map(length),
            frame: Default::default(),
        },
    );
    let plate = prepare_primitive(&session, session.revision(), actor(), &receiver).unwrap();
    session.commit(plate.prepared).unwrap();
    let receiver_placement = ComponentPlacementRequest::new(
        receiver.body,
        NumericTransform {
            translation: [5., 2., 0.].map(length),
            ..Default::default()
        },
    );
    let placed =
        prepare_place_component(&session, session.revision(), actor(), &receiver_placement)
            .unwrap();
    session.commit(placed.prepared).unwrap();

    let path = OccurrencePath {
        root: placement.instance,
        members: vec![placement.member],
    };
    let source = transfer_occurrence_catalog(session.document())
        .unwrap()
        .into_iter()
        .find(|source| source.path == path)
        .expect("the placed Library leaf is listed as a transfer source");
    assert_eq!(source.component, placement.ids[&definition.root]);
    let washer = read_mechanical_part(session.document(), source.producer).unwrap();
    let ResolvedMechanicalPart::Washer {
        bore_radius_mm,
        thickness_mm,
        frame,
        ..
    } = washer.parameters.resolve(session.parameters()).unwrap()
    else {
        panic!("the shipped M6 source must resolve as a washer");
    };
    near(bore_radius_mm, 3.2);
    assert_eq!(frame.n, [0., 0., 1.]);
    let initial = compile(&session);
    let source_face = top(
        &initial,
        source.producer,
        source.component,
        frame.origin[2] + thickness_mm,
    );
    let receiver_face = top(&initial, receiver.feature, receiver.feature, 5.);
    let mut request = TransferRequest::new(
        "Transfer the Library washer bore",
        TransferSource {
            component: source.component,
            path,
            selection: TransferSelection::FaceHoles {
                face: source_face.intent.id,
            },
        },
    );
    let target = TransferTarget::new(
        BodyInput {
            body: receiver.body,
            producer: receiver.feature,
        },
        OccurrencePath {
            root: receiver_placement.instance,
            members: vec![],
        },
        receiver_face.intent.id,
        Default::default(),
    );
    request.references = vec![source_face, receiver_face];
    request.targets.push(target.clone());
    let transfer = prepare_transfer(&session, session.revision(), actor(), &request).unwrap();
    session.commit(transfer.prepared).unwrap();

    let verify = |session: &Session, x: f64, y: f64| {
        let model = compile(session);
        let snapshot = model.snapshot();
        assert_eq!(snapshot.transfer_counts[&target.feature], 1);
        near(
            model.solid(receiver.body).unwrap().volume_mm3().unwrap(),
            (45. * 30. - PI * bore_radius_mm.powi(2)) * 5.,
        );
        let center = snapshot.shapes[&target.feature]
            .topology
            .elements
            .iter()
            .find_map(|edge| match edge.geometry {
                Some(AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    sweep_rad,
                    ..
                }) if edge.key.kind == TopologyKind::Edge
                    && (radius_mm - bore_radius_mm).abs() < 1e-6
                    && (sweep_rad.abs() - 2. * PI).abs() < 1e-6
                    && (frame.origin_mm[2] - 5.).abs() < 1e-6 =>
                {
                    Some(frame.origin_mm)
                }
                _ => None,
            })
            .expect("the receiving plate contains the exact transferred circle");
        near(center[0], frame.origin[0] + x);
        near(center[1], frame.origin[1] + y);
    };
    verify(&session, 10., 10.);
    let move_washer = InstanceMoveRequest::new(
        placement.instance,
        NumericTransform {
            translation: [8., 2., 0.].map(length),
            ..Default::default()
        },
    );
    let moved =
        prepare_instance_moves(&session, session.revision(), actor(), &[move_washer]).unwrap();
    session.commit(moved.prepared).unwrap();
    verify(&session, 18., 12.);
    assert_eq!(
        symbols::read_instance(session.document(), placement.instance)
            .unwrap()
            .pin,
        pin
    );
}

#[test]
fn native_rail_pitch_pattern_and_member_pose_update_three_explicit_material_cuts() {
    let mut f = fixture();
    let source_part = f.s.document().get(f.rail.body).unwrap().clone();
    let original = f.s.document().clone();
    let revision = f.s.revision();
    let p = prepare_transfer(&f.s, revision, actor(), &f.request).unwrap();
    let preview = Compiler::default()
        .compile(p.prepared.document(), revision)
        .unwrap();
    assert_eq!(preview.snapshot().transfer_counts.len(), 3);
    assert_eq!(f.s.document(), &original);
    f.s.commit(p.prepared).unwrap();
    assert_eq!(f.s.revision(), revision + 1);
    verify(&f, 2, &[30., 62.]);
    assert_eq!(f.s.document().get(f.rail.body), Some(&source_part));
    assert!(
        matches!(f.s.document().get(f.source_member.instance).map(|e|&e.data),Some(EntityData::Instance{component,..})if *component==f.rail.body)
    );
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &original);
    f.s.redo(f.s.revision(), actor()).unwrap();
    let m = verify(&f, 2, &[30., 62.]);
    let out = f.request.targets[0].feature;
    let topo = &m.snapshot().shapes[&out].topology;
    let wall = topo
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.geometry.is_none()
                && e.origins.iter().any(|o| o.role.contains("/hole/"))
        })
        .unwrap();
    let old_wall = GeometryIntent::capture(topo, wall.key).unwrap();
    parameter(&mut f.s, "pitch", "40");
    parameter(&mut f.s, "count", "3");
    let m = verify(&f, 3, &[30., 70., 110.]);
    assert!(matches!(
        references::resolve(
            &old_wall,
            &m.snapshot().shapes[&out].topology,
            f.s.revision()
        ),
        Resolution::Resolved { .. }
    ));
    near(
        m.solid(f.rail.body).unwrap().volume_mm3().unwrap(),
        10400. - 3. * PI * 9. * 4.,
    );
    parameter(&mut f.s, "offset", "15");
    verify(&f, 3, &[35., 75., 115.]);
    let root_move = InstanceMoveRequest::new(
        f.outer.instance,
        NumericTransform {
            translation: [1e6, -2e6, 300.].map(length),
            angle: angle(35.),
            ..Default::default()
        },
    );
    let p = prepare_instance_moves(&f.s, f.s.revision(), actor(), &[root_move]).unwrap();
    f.s.commit(p.prepared).unwrap();
    let m = verify(&f, 3, &[35., 75., 115.]);
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("wood.step");
    m.solid(f.request.targets[0].input.body)
        .unwrap()
        .write_step(&step)
        .unwrap();
    let loaded = multicad_kernel::Solid::read_step(&step, Default::default()).unwrap();
    near(
        loaded.volume_mm3().unwrap(),
        m.solid(f.request.targets[0].input.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
    );
    f.s = reopen(&f.s);
    verify(&f, 3, &[35., 75., 115.]);
    assert_eq!(f.s.revision(), 0);
    assert_eq!(
        TransferRequest::existing(f.s.document(), out).unwrap(),
        f.request
    );
}
#[test]
fn missing_explicit_hole_is_repairable_using_source_only_native_geometry() {
    let mut f = fixture();
    let m = compile(&f.s);
    let mut topology = m.snapshot().shapes[&f.rail_head].topology.clone();
    topology.owner = f.rail.body;
    let edge=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{frame,..})if e.key.kind==TopologyKind::Edge&&(frame.origin_mm[0]-52.).abs()<1e-6&&(frame.origin_mm[2]-4.).abs()<1e-6)).unwrap();
    let selected = GeometryIntent::capture(&topology, edge.key).unwrap();
    let old_face = f.request.source.selection.captures()[0];
    f.request.references.retain(|r| r.intent.id != old_face);
    f.request.source.selection = TransferSelection::Circles {
        references: vec![selected.id],
    };
    f.request.references.insert(
        0,
        AlignmentInput {
            source: f.rail.body,
            intent: selected,
        },
    );
    let p = prepare_transfer(&f.s, f.s.revision(), actor(), &f.request).unwrap();
    commit(&mut f.s, p);
    verify(&f, 1, &[62.]);
    parameter(&mut f.s, "count", "1");
    let original = f.s.document().clone();
    let revision = f.s.revision();
    let error = Compiler::default()
        .compile(f.s.document(), revision)
        .err()
        .unwrap();
    assert!(error.message.contains("review/repair"));
    let repaired_source = Compiler::default()
        .compile_sources(f.s.document(), revision, &[f.rail_head])
        .unwrap();
    assert!(matches!(
        repaired_source.snapshot().scope,
        multicad_model::EvaluationScope::Sources(_)
    ));
    assert!(repaired_source.snapshot().transfer_counts.is_empty());
    let catalog = transfer_occurrence_catalog(f.s.document()).unwrap();
    assert!(
        catalog
            .iter()
            .any(|entry| entry.path == f.request.source.path && entry.component == f.rail.body)
    );
    for target in &f.request.targets {
        assert!(
            catalog
                .iter()
                .any(|entry| entry.path == target.path && entry.component == target.input.body)
        );
    }
    let mut current = repaired_source.snapshot().shapes[&f.rail_head]
        .topology
        .clone();
    current.owner = f.rail.body;
    let new_edge=current.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{frame,..})if e.key.kind==TopologyKind::Edge&&(frame.origin_mm[0]-20.).abs()<1e-6&&(frame.origin_mm[2]-4.).abs()<1e-6)).unwrap();
    let mut intent = GeometryIntent::capture(&current, new_edge.key).unwrap();
    intent.id = f.request.references[0].intent.id;
    let mut repair = TransferRequest::existing(f.s.document(), f.request.group).unwrap();
    repair.references[0].intent = intent;
    let p = prepare_transfer_edit(&f.s, revision, actor(), &repair).unwrap();
    let native = Compiler::default()
        .compile(p.prepared.document(), revision)
        .unwrap();
    assert_eq!(
        native.snapshot().transfer_counts[&repair.targets[0].feature],
        1
    );
    assert_eq!(f.s.document(), &original);
    assert_eq!(f.s.revision(), revision);
    f.s.commit(p.prepared).unwrap();
    f.request = repair;
    verify(&f, 1, &[30.]);
    f.s = reopen(&f.s);
    verify(&f, 1, &[30.]);
}
#[test]
fn valid_empty_face_pattern_removes_no_material_and_never_rebinds_old_hole_walls() {
    let mut f = fixture();
    let p = prepare_transfer(&f.s, f.s.revision(), actor(), &f.request).unwrap();
    commit(&mut f.s, p);
    let m = verify(&f, 2, &[30., 62.]);
    let out = f.request.targets[0].feature;
    let topology = &m.snapshot().shapes[&out].topology;
    let wall = topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.geometry.is_none()
                && e.origins.iter().any(|o| o.role.contains("/hole/"))
        })
        .unwrap();
    let old = GeometryIntent::capture(topology, wall.key).unwrap();
    let mut cut = f.s.document().get(f.rail_head).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut cut.data else {
        panic!()
    };
    *suppressed = true;
    f.s.apply(Request {
        expected_revision: f.s.revision(),
        actor: actor(),
        label: "Suppress source holes".into(),
        edits: vec![Mutation::Replace(cut)],
    })
    .unwrap();
    let m = verify(&f, 0, &[]);
    assert!(!matches!(
        references::resolve(&old, &m.snapshot().shapes[&out].topology, f.s.revision()),
        Resolution::Resolved { .. }
    ));
    assert_eq!(f.s.document().get(f.rail.body).unwrap().id, f.rail.body);
}
#[test]
fn source_outer_boundary_is_an_exact_editable_cutout_with_per_material_clearance() {
    let mut f = fixture();
    let face = f.request.source.selection.captures()[0];
    f.request.source.selection = TransferSelection::FaceBoundary { face };
    for target in &mut f.request.targets {
        target.parameters.diameter = None;
    }
    let source = f.s.document().get(f.rail.body).unwrap().clone();
    let p = prepare_transfer(&f.s, f.s.revision(), actor(), &f.request).unwrap();
    commit(&mut f.s, p);
    let m = compile(&f.s);
    for (target, (height, depth, area)) in f.request.targets.iter().zip([
        (18., 12., 120. * 20.),
        (8., 8., 120.2 * 20.2),
        (2., 2., 120. * 20.),
    ]) {
        assert_eq!(m.snapshot().transfer_counts[&target.feature], 1);
        near(
            m.solid(target.input.body).unwrap().volume_mm3().unwrap(),
            130. * 30. * height - area * depth,
        );
        assert_eq!(
            m.solid(target.input.body).unwrap().solid_count().unwrap(),
            1
        );
    }
    assert_eq!(f.s.document().get(f.rail.body), Some(&source));
    let loaded = reopen(&f.s);
    assert_eq!(
        compile(&loaded).snapshot().transfer_counts,
        m.snapshot().transfer_counts
    );
    let mut edit = TransferRequest::existing(f.s.document(), f.request.group).unwrap();
    edit.targets[0].parameters.clearance = length(0.5);
    let p = prepare_transfer_edit(&f.s, f.s.revision(), actor(), &edit).unwrap();
    let preview = Compiler::default()
        .compile(p.prepared.document(), f.s.revision())
        .unwrap();
    near(
        preview
            .solid(edit.targets[0].input.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        130. * 30. * 18. - 120.5 * 20.5 * 12.,
    );
}

#[test]
fn threaded_mouth_outside_axial_span_is_refused_but_same_part_plain_hole_transfers() {
    for representation in [
        ThreadRepresentation::Modeled,
        ThreadRepresentation::CosmeticEnvelope,
    ] {
        let mut f = fixture();
        let thread = ThreadRequest::receiving(
            "Internal M8 transfer refusal source",
            ThreadMode::CutInternal,
            BodyInput {
                body: f.rail.body,
                producer: f.rail_head,
            },
            ThreadParameters {
                major_diameter: length(8.),
                pitch: length(1.25),
                length: length(3.),
                frame: PrimitiveFrame {
                    origin: [20., 10., 0.5].map(length),
                    ..Default::default()
                },
                representation,
                ..Default::default()
            },
        )
        .unwrap();
        let prepared = prepare_thread(&f.s, f.s.revision(), actor(), &thread).unwrap();
        let compiler = Compiler {
            omit_display: true,
            ..Default::default()
        };
        // Prove that the authored threaded source itself is valid native geometry.
        compiler
            .compile(prepared.prepared.document(), f.s.revision())
            .unwrap();
        f.s.commit(prepared.prepared).unwrap();
        let baseline = compiler.compile(f.s.document(), f.s.revision()).unwrap();
        let original = f.s.document().clone();
        let revision = f.s.revision();
        let source_face = top(&baseline, thread.feature, f.rail.body, 4.);
        let mut source_topology = baseline.snapshot().shapes[&thread.feature].topology.clone();
        source_topology.owner = f.rail.body;
        let mouth = |x: f64| {
            let edge = source_topology
                .elements
                .iter()
                .find(|e| {
                    matches!(e.geometry, Some(AnalyticGeometry::CircularArc {
                        frame, radius_mm, sweep_rad, ..
                    }) if e.key.kind == TopologyKind::Edge
                        && (frame.origin_mm[0] - x).abs() < 1e-7
                        && (frame.origin_mm[1] - 10.).abs() < 1e-7
                        && (frame.origin_mm[2] - 4.).abs() < 1e-7
                        && (radius_mm - 3.).abs() < 1e-7
                        && (sweep_rad.abs() - 2. * PI).abs() < 1e-7)
                })
                .expect("unthreaded collar retains its original complete circular mouth");
            AlignmentInput {
                source: f.rail.body,
                intent: GeometryIntent::capture(&source_topology, edge.key).unwrap(),
            }
        };
        // Both circles are above the thread's z=0.5..3.5 span. The first is
        // coaxial with its M8 thread; the second is an unrelated ordinary hole.
        let threaded_mouth = mouth(20.);
        let ordinary_mouth = mouth(52.);
        let target_captures = f.request.references[1..].to_vec();
        for (selection, capture) in [
            (
                TransferSelection::FaceHoles {
                    face: source_face.intent.id,
                },
                source_face,
            ),
            (
                TransferSelection::Circles {
                    references: vec![threaded_mouth.intent.id],
                },
                threaded_mouth,
            ),
        ] {
            let mut request = f.request.clone();
            request.source.selection = selection;
            request.references = std::iter::once(capture)
                .chain(target_captures.clone())
                .collect();
            let candidate = prepare_transfer(&f.s, revision, actor(), &request).unwrap();
            let error = compiler
                .compile(candidate.prepared.document(), revision)
                .err()
                .expect("threaded transfer must fail before any transaction is committed");
            assert!(
                error
                    .message
                    .contains("Threaded-hole transfer is unsupported"),
                "{representation:?}: {error}"
            );
            assert!(
                error
                    .message
                    .contains("Internal M8 transfer refusal source")
            );
            assert_eq!(f.s.document(), &original);
            assert_eq!(f.s.revision(), revision);
            assert!(baseline.snapshot().transfer_counts.is_empty());
        }
        let mut request = f.request.clone();
        request.source.selection = TransferSelection::Circles {
            references: vec![ordinary_mouth.intent.id],
        };
        request.references = std::iter::once(ordinary_mouth)
            .chain(target_captures)
            .collect();
        let candidate = prepare_transfer(&f.s, revision, actor(), &request).unwrap();
        let transferred = compiler
            .compile(candidate.prepared.document(), revision)
            .expect("an ordinary noncoaxial hole on the same threaded Part remains transferable");
        for (target, (height, radius, depth)) in
            request
                .targets
                .iter()
                .zip([(18., 2.5, 12.), (8., 3.2, 8.), (2., 3.25, 2.)])
        {
            assert_eq!(
                transferred.snapshot().transfer_counts.get(&target.feature),
                Some(&1)
            );
            near(
                transferred
                    .solid(target.input.body)
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
                130. * 30. * height - PI * radius * radius * depth,
            );
            let circles = transferred.snapshot().shapes[&target.feature]
                .topology
                .elements
                .iter()
                .filter_map(|e| match e.geometry {
                    Some(AnalyticGeometry::CircularArc {
                        frame,
                        radius_mm,
                        sweep_rad,
                        ..
                    }) if (frame.origin_mm[2] - height).abs() < 1e-7
                        && (radius_mm - radius).abs() < 1e-7
                        && (sweep_rad.abs() - 2. * PI).abs() < 1e-7 =>
                    {
                        Some(frame.origin_mm)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(!circles.is_empty());
            assert!(circles.iter().all(|center| {
                (center[0] - 62.).abs() < 1e-7 && (center[1] - 10.).abs() < 1e-7
            }));
        }
        assert_eq!(f.s.document(), &original);
        assert_eq!(f.s.revision(), revision);
    }
}
