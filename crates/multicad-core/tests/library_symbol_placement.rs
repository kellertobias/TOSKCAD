//! Library placement is one source transaction, including optional containment.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    hierarchy::{self, Action, ContainerKind},
    model::{self, PrimitiveParameters, PrimitiveRequest},
    project_file::{self, ProjectFile},
    symbols::*,
};
use std::collections::BTreeMap;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn content() -> SymbolDefinition {
    let mut source = Session::new(Document::new("Library author").unwrap()).unwrap();
    let primitive = PrimitiveRequest::new(
        "Block",
        PrimitiveParameters::Box {
            size: [model::length(20.), model::length(30.), model::length(10.)],
            frame: Default::default(),
        },
    );
    source
        .commit(
            model::prepare_primitive(&source, 0, actor(), &primitive)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let author = SymbolAuthorRequest::new(
        "Pinned block",
        primitive.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let prepared = prepare_author(&source, source.revision(), actor(), &author).unwrap();
    read_definition(prepared.prepared.document(), prepared.definition).unwrap()
}
fn host() -> (Session, EntityId) {
    let mut session = Session::new(Document::new("Receiving project").unwrap()).unwrap();
    let create = hierarchy::prepare(
        &session,
        0,
        actor(),
        Action::Create {
            kind: ContainerKind::Component,
            name: "Receiving assembly".into(),
            parent: None,
        },
    )
    .unwrap();
    let parent = create.created.unwrap();
    session.commit(create.prepared).unwrap();
    (session, parent)
}
fn request(content: &SymbolDefinition, definition: EntityId) -> SymbolPlaceRequest {
    SymbolPlaceRequest::from_definition(
        definition,
        content,
        "Placed block",
        BTreeMap::new(),
        Placement::default(),
    )
    .unwrap()
}
#[test]
fn atomic_library_pin_instance_and_only_visible_parent_membership_survive_history_and_reopen() {
    let content = content();
    let (mut session, parent) = host();
    let before = session.document().clone();
    let revision = session.revision();
    let request = request(&content, EntityId::new());
    for _ in 0..2 {
        let preview = prepare_library_place(
            &session,
            revision,
            actor(),
            &content,
            &request,
            Some(parent),
        )
        .unwrap();
        assert_eq!(preview.definition, request.definition);
        assert_eq!(preview.instances, [request.instance]);
        assert_eq!(
            preview.prepared.document().hierarchy().members[&parent],
            [request.instance]
        );
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
    }
    let prepared = prepare_library_place(
        &session,
        revision,
        actor(),
        &content,
        &request,
        Some(parent),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    assert_eq!(session.revision(), revision + 1);
    let after = session.document().clone();
    let private = owned_entities(&after).unwrap();
    assert!(private.contains(&request.wrapper));
    assert!(private.contains(&request.member));
    assert!(request.ids.values().all(|id| private.contains(id)));
    assert!(!private.contains(&request.instance));
    assert!(
        after.hierarchy().members[&parent]
            .iter()
            .all(|id| !private.contains(id))
    );
    assert_eq!(
        read_instance(&after, request.record).unwrap().pin,
        content.pin().unwrap()
    );
    validate(&after).unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(after.clone()).unwrap(),
        &mut bytes,
        project_file::FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        project_file::FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(reopened, after);
    Session::new(reopened).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &after);
}
#[test]
fn exact_pin_reuses_definition_and_another_pin_requires_explicit_update() {
    let content = content();
    let (mut session, parent) = host();
    let first = request(&content, EntityId::new());
    session
        .commit(
            prepare_library_place(
                &session,
                session.revision(),
                actor(),
                &content,
                &first,
                None,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    assert_eq!(
        library_placement_definition(session.document(), &content).unwrap(),
        Some(first.definition)
    );
    let second = request(&content, first.definition);
    let prepared = prepare_library_place(
        &session,
        session.revision(),
        actor(),
        &content,
        &second,
        Some(parent),
    )
    .unwrap();
    assert_eq!(definitions(prepared.prepared.document()).unwrap().len(), 1);
    assert_eq!(
        prepared.prepared.document().get(first.wrapper),
        session.document().get(first.wrapper)
    );
    let wrong_id = request(&content, EntityId::new());
    assert!(
        prepare_library_place(
            &session,
            session.revision(),
            actor(),
            &content,
            &wrong_id,
            None
        )
        .is_err()
    );
    let mut newer = content.clone();
    newer.revision = "2".into();
    assert!(library_placement_definition(session.document(), &newer).is_err());
    let newer_request = request(&newer, EntityId::new());
    assert!(
        prepare_library_place(
            &session,
            session.revision(),
            actor(),
            &newer,
            &newer_request,
            None
        )
        .is_err()
    );
}
#[test]
fn invalid_private_or_stale_parent_and_stale_revision_never_publish_partial_content() {
    let content = content();
    let (mut session, parent) = host();
    let request = request(&content, EntityId::new());
    let before = session.document().clone();
    assert!(
        prepare_library_place(
            &session,
            session.revision(),
            actor(),
            &content,
            &request,
            Some(EntityId::new())
        )
        .is_err()
    );
    let invalid_parent = Entity::new(
        "Material",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [0, 0, 0, 255],
            section_hatch: None,
        },
    );
    let material = invalid_parent.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Fixture material".into(),
            edits: vec![Mutation::Insert(invalid_parent)],
        })
        .unwrap();
    let base = session.document().clone();
    assert!(
        prepare_library_place(
            &session,
            session.revision(),
            actor(),
            &content,
            &request,
            Some(material)
        )
        .is_err()
    );
    assert!(
        prepare_library_place(
            &session,
            session.revision() - 1,
            actor(),
            &content,
            &request,
            Some(parent)
        )
        .is_err()
    );
    let prepared = prepare_library_place(
        &session,
        session.revision(),
        actor(),
        &content,
        &request,
        Some(parent),
    )
    .unwrap();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Owner change".into(),
            edits: vec![Mutation::Insert(Entity::new("Group", EntityData::Group {}))],
        })
        .unwrap();
    assert!(session.commit(prepared.prepared).is_err());
    assert_eq!(definitions(session.document()).unwrap().len(), 0);
    assert_ne!(session.document(), &before);
    assert_ne!(session.document(), &base);
    session
        .commit(
            prepare_library_place(
                &session,
                session.revision(),
                actor(),
                &content,
                &request,
                Some(parent),
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let extra = SymbolPlaceRequest::new(
        session.document(),
        request.definition,
        "Second",
        BTreeMap::new(),
        Placement::default(),
    )
    .unwrap();
    let before = session.document().clone();
    assert!(
        prepare_place_in(
            &session,
            session.revision(),
            actor(),
            &extra,
            Some(request.wrapper)
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
    let embedded =
        prepare_place_in(&session, session.revision(), actor(), &extra, Some(parent)).unwrap();
    assert_eq!(
        embedded.prepared.document().hierarchy().members[&parent],
        [request.instance, extra.instance]
    );
}
