use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::{LibraryDefinition, ProfileShape, Stock, StockKind},
    model::{
        self, BodyInput, BodyOutput, MemberDefinition, MemberEnd, MemberPath, MemberRequest,
        OccurrencePath, ProfileAttachment, ProfileConnectionKind, ProfileConnectionNut,
        ProfileConnectionRequest, ProfileConnectorAnchor, ProfileGrooveSide, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, Model, schedules::BomOptions};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn prepare_members(paths: [([f64; 3], [f64; 3]); 2]) -> (Session, [MemberRequest; 2]) {
    let mut session = Session::new(Document::new("Line 8 connection").unwrap()).unwrap();
    let material = Entity::new(
        "Aluminum",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 165, 170, 255],
            section_hatch: None,
        },
    );
    let stock = EntityId::new();
    let definition = LibraryDefinition::Stock(Stock {
        material: material.id,
        specification: StockKind::Profile {
            designation: "item 0.0.026.03".into(),
            length_mm: 6000.0,
            width_mm: 40.0,
            height_mm: 40.0,
            wall_mm: None,
            shape: ProfileShape::ItemLine8_40x40,
        },
    })
    .entity(stock, "Line 8 section")
    .unwrap();
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Profile stock".into(),
            edits: vec![Mutation::Insert(material), Mutation::Insert(definition)],
        })
        .unwrap();
    let requests = paths.map(|(start, end)| {
        MemberRequest::new(
            "Profile member",
            MemberDefinition {
                output: BodyOutput {
                    body: EntityId::nil(),
                },
                stock,
                path: MemberPath::Points {
                    start: start.map(length),
                    end: end.map(length),
                },
                anchor: Default::default(),
                rotation: None,
                extend_start: None,
                extend_end: None,
            },
        )
    });
    let members = model::prepare_members(&session, 1, actor(), &requests, None).unwrap();
    session.commit(members.prepared).unwrap();
    (session, requests)
}

fn attach(member: &MemberRequest, anchor: ProfileConnectorAnchor) -> ProfileAttachment {
    let body = member.member.output.body;
    ProfileAttachment {
        member: BodyInput {
            body,
            producer: member.feature,
        },
        occurrence: OccurrencePath {
            root: body,
            members: vec![],
        },
        anchor,
    }
}

fn groove(member: &MemberRequest, side: ProfileGrooveSide, station: f64) -> ProfileAttachment {
    attach(
        member,
        ProfileConnectorAnchor::Groove {
            side,
            station: length(station),
        },
    )
}

fn compile(document: &Document, revision: u64) -> Result<Model, multicad_model::ModelError> {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(document, revision)
}

#[test]
fn angle_and_flat_connections_create_five_parts_and_never_machine_the_profiles() {
    use ProfileConnectionKind::{AngleBracket, JoiningPlate};
    use ProfileGrooveSide::{MaxX, MaxY};
    for (kind, paths, second_side, station) in [
        (
            AngleBracket,
            [
                ([0., 0., 0.], [100., 0., 0.]),
                ([-20., 0., 20.], [-20., 0., 120.]),
            ],
            MaxX,
            20.,
        ),
        (
            JoiningPlate,
            [
                ([0., 0., 0.], [100., 0., 0.]),
                ([0., 40., 0.], [100., 40., 0.]),
            ],
            MaxY,
            40.,
        ),
    ] {
        let (mut session, members) = prepare_members(paths);
        let request = ProfileConnectionRequest::new(
            kind.label(),
            kind,
            groove(&members[0], MaxY, station),
            groove(&members[1], second_side, station),
        );
        let original = session.document().clone();
        let draft =
            model::prepare_profile_connection(&session, session.revision(), actor(), &request)
                .unwrap();
        assert_eq!(session.document(), &original);
        let native = compile(draft.prepared.document(), session.revision() + 1).unwrap();
        assert_eq!(native.snapshot().leaf_occurrences.len(), 7);
        assert_eq!(draft.outputs.len(), 5);
        for member in &members {
            let id = member.member.output.body;
            assert_eq!(original.get(id), draft.prepared.document().get(id));
        }
        let ids = draft.outputs;
        session.commit(draft.prepared).unwrap();
        for part in &ids {
            assert_eq!(
                model::profile_connection_for(session.document(), part.body),
                Some(request.connection)
            );
        }
        let mut edit = request.clone();
        edit.name = "Retained connector".into();
        let prepared =
            model::prepare_profile_connection(&session, session.revision(), actor(), &edit)
                .unwrap();
        assert_eq!(ids, prepared.outputs);
        if kind == JoiningPlate {
            edit.second = groove(&members[1], MaxY, 41.);
            let invalid =
                model::prepare_profile_connection(&session, session.revision(), actor(), &edit)
                    .unwrap();
            let problem = compile(invalid.prepared.document(), session.revision() + 1)
                .err()
                .unwrap();
            assert!(
                problem.to_string().contains("coplanar parallel grooves"),
                "{problem}"
            );
        }
    }
}

#[test]
fn paired_end_connection_machines_only_the_host_and_survives_reopen_and_undo() {
    let (mut session, members) = prepare_members([
        ([-100., 0., 0.], [0., 0., 0.]),
        ([20., 0., -50.], [20., 0., 50.]),
    ]);
    let request = ProfileConnectionRequest::new(
        "Paired universal end fasteners",
        ProfileConnectionKind::UniversalEnd,
        attach(
            &members[0],
            ProfileConnectorAnchor::End {
                end: MemberEnd::End,
            },
        ),
        groove(&members[1], ProfileGrooveSide::MinX, 50.0),
    );
    let original = session.document().clone();
    let mut unsupported = request.clone();
    unsupported.nut = ProfileConnectionNut::Stainless;
    let error =
        model::prepare_profile_connection(&session, session.revision(), actor(), &unsupported)
            .unwrap_err();
    assert!(error.message.contains("original fastener set"));
    assert_eq!(session.document(), &original);
    let draft =
        model::prepare_profile_connection(&session, session.revision(), actor(), &request).unwrap();
    assert_eq!(draft.outputs.len(), 6);
    let native = compile(draft.prepared.document(), session.revision() + 1).unwrap();
    assert_eq!(native.snapshot().leaf_occurrences.len(), 8);
    let host = members[0].member.output.body;
    let cut = native.snapshot().bodies[&host];
    assert_ne!(cut, members[0].feature);
    assert_eq!(native.solid(cut).unwrap().solid_count().unwrap(), 1);
    let volume = |feature| native.solid(feature).unwrap().volume_mm3().unwrap();
    assert!(volume(members[0].feature) - volume(cut) > 100.);
    let other = members[1].member.output.body;
    assert_eq!(native.snapshot().bodies[&other], members[1].feature);
    use multicad_model::schedules::{BomOptions, Supply};
    let bom = native
        .bom(draft.prepared.document(), &BomOptions::default())
        .unwrap();
    for row in &bom.rows {
        let expected = if row.part == host || row.part == other {
            Supply::Manufactured
        } else {
            Supply::Purchased
        };
        assert_eq!(row.supply, expected, "{}", row.name);
    }
    let made_parts = native
        .bom(
            draft.prepared.document(),
            &BomOptions {
                include_purchased: false,
                ..BomOptions::default()
            },
        )
        .unwrap();
    assert_eq!(made_parts.rows.len(), 2);
    assert_eq!(made_parts.excluded.len(), draft.outputs.len());
    session.commit(draft.prepared).unwrap();
    let mut bytes = vec![];
    let file = ProjectFile::new(session.document().clone()).unwrap();
    assert!(
        file.capabilities()
            .contains(model::PROFILE_CONNECTION_CAPABILITY)
    );
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(&reopened.document, session.document());
    assert_eq!(
        model::read_profile_connection_request(&reopened.document, request.connection).unwrap(),
        request
    );
    let removed = model::prepare_remove_profile_connection(
        &session,
        session.revision(),
        actor(),
        request.connection,
    )
    .unwrap();
    assert_eq!(removed.document(), &original);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &reopened.document);
}

#[test]
fn stainless_line_8_nut_replaces_stock_without_reallocating_connection_children() {
    use ProfileConnectionKind::JoiningPlate;
    use ProfileConnectionNut::{Stainless, ZincPlated};
    use ProfileGrooveSide::MaxY;
    let (mut session, members) = prepare_members([
        ([0., 0., 0.], [100., 0., 0.]),
        ([0., 40., 0.], [100., 40., 0.]),
    ]);
    let mut request = ProfileConnectionRequest::new(
        "Joining plate",
        JoiningPlate,
        groove(&members[0], MaxY, 40.),
        groove(&members[1], MaxY, 40.),
    );
    assert_eq!(request.nut, ZincPlated);
    let request_wire = serde_json::to_value(&request).unwrap();
    assert!(request_wire.get("nut").is_none());
    let restored: ProfileConnectionRequest = serde_json::from_value(request_wire).unwrap();
    assert_eq!(restored, request);

    let prepared =
        model::prepare_profile_connection(&session, session.revision(), actor(), &request).unwrap();
    let outputs = prepared.outputs.clone();
    session.commit(prepared.prepared).unwrap();
    let initial = session.document().clone();
    let baseline = compile(&initial, session.revision()).unwrap();
    assert!(
        !ProjectFile::new(initial.clone())
            .unwrap()
            .capabilities()
            .contains(model::PROFILE_CONNECTION_NUT_CAPABILITY)
    );

    request.nut = Stainless;
    let prepared =
        model::prepare_profile_connection(&session, session.revision(), actor(), &request).unwrap();
    assert_eq!(session.document(), &initial);
    assert_eq!(prepared.outputs, outputs);
    let after = prepared.prepared.document();
    let checked = compile(after, session.revision() + 1).unwrap();
    assert_eq!(checked.snapshot().leaf_occurrences.len(), 7);
    let bom = checked.bom(after, &BomOptions::default()).unwrap();
    for (role, part) in JoiningPlate.roles().iter().zip(&outputs) {
        if matches!(role, model::ProfilePartRole::Nut(_)) {
            let before = baseline.solid(part.feature).unwrap();
            let replacement = checked.solid(part.feature).unwrap();
            assert_eq!(replacement.solid_count().unwrap(), 1);
            assert_eq!(replacement.bounds().unwrap(), before.bounds().unwrap());
            assert!(
                (replacement.volume_mm3().unwrap() - before.volume_mm3().unwrap()).abs() < 1e-7
            );
            let row = bom.rows.iter().find(|row| row.part == part.body).unwrap();
            assert_eq!(row.quantity, 1);
            assert!(row.name.contains("stainless T-slot nut") && row.name.contains("0.0.388.49"));
        } else {
            assert_eq!(after.get(part.body), initial.get(part.body));
            assert_eq!(after.get(part.feature), initial.get(part.feature));
        }
    }
    session.commit(prepared.prepared).unwrap();

    let project = ProjectFile::new(session.document().clone()).unwrap();
    let capability = model::PROFILE_CONNECTION_NUT_CAPABILITY;
    assert!(project.capabilities().contains(capability));
    let mut bytes = Vec::new();
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut supported = project_file::supported_capabilities();
    assert!(supported.remove(capability));
    let error =
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported).unwrap_err();
    assert!(error.contains(capability), "{error}");
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, project.document);
    assert_eq!(
        model::read_profile_connection_request(&reopened.document, request.connection).unwrap(),
        request
    );
    let revised = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &initial);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &revised);

    request.nut = ZincPlated;
    let restored =
        model::prepare_profile_connection(&session, session.revision(), actor(), &request).unwrap();
    assert_eq!(restored.outputs, outputs);
    assert_eq!(restored.prepared.document(), &initial);
}
