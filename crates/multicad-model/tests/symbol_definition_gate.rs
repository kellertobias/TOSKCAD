use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, Mutation},
    model,
    symbols::*,
};
use multicad_model::Compiler;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
#[test]
fn an_unplaced_native_invalid_definition_cannot_pass_the_complete_model_gate() {
    let mut source = Session::new(Document::new("Bad library source").unwrap()).unwrap();
    let stock = model::PrimitiveRequest::new(
        "Bad box",
        model::PrimitiveParameters::Box {
            size: [model::length(10.), model::length(20.), model::length(5.)],
            frame: Default::default(),
        },
    );
    source
        .commit(
            model::prepare_primitive(&source, source.revision(), actor(), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut feature = source.document().get(stock.feature).unwrap().clone();
    let EntityData::Feature { definition, .. } = &mut feature.data else {
        unreachable!()
    };
    let mut primitive = model::PrimitiveDefinition::from_definition(definition).unwrap();
    let model::PrimitiveParameters::Box { size, .. } = &mut primitive.parameters else {
        unreachable!()
    };
    size[0] = model::length(-10.);
    *definition = primitive.definition().unwrap();
    source
        .apply(multicad_core::commands::Request {
            expected_revision: source.revision(),
            actor: actor(),
            label: "Invalid source geometry".into(),
            edits: vec![Mutation::Replace(feature)],
        })
        .unwrap();
    let author = SymbolAuthorRequest::new(
        "Bad library box",
        stock.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    source
        .commit(
            prepare_author(&source, source.revision(), actor(), &author)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let definition = read_definition(source.document(), author.definition).unwrap();
    let mut target = Session::new(Document::new("Unplaced library").unwrap()).unwrap();
    target
        .commit(
            prepare_import(&target, 0, actor(), &definition)
                .unwrap()
                .prepared,
        )
        .unwrap();
    assert_eq!(target.document().entities().count(), 1);
    let error = Compiler::default()
        .compile(target.document(), target.revision())
        .err()
        .expect("Bad template must fail even with no placements");
    assert!(error.message.contains("Pinned symbol"), "{error}");
    assert!(error.message.to_lowercase().contains("positive"), "{error}");
    // Pure source remains serializable and repairable; the geometry gate is the
    // authority for Apply/export, rather than source-container parsing alone.
    assert!(multicad_core::project_file::ProjectFile::new(target.document().clone()).is_ok());
}
#[test]
fn empty_symbol_root_is_not_mistaken_for_valid_native_geometry() {
    let mut source = Session::new(Document::new("Empty library source").unwrap()).unwrap();
    let part = Entity::new(
        "Empty",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    source
        .apply(multicad_core::commands::Request {
            expected_revision: 0,
            actor: actor(),
            label: "Empty source".into(),
            edits: vec![Mutation::Insert(part.clone())],
        })
        .unwrap();
    let author = SymbolAuthorRequest::new("Empty", part.id, SymbolKind::Static, vec![], None);
    source
        .commit(
            prepare_author(&source, source.revision(), actor(), &author)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let error = Compiler::default()
        .compile(source.document(), source.revision())
        .err()
        .expect("Empty symbol root must fail");
    assert!(error.message.contains("nonempty"), "{error}");
}
