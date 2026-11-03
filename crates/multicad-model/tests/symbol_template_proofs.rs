use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model,
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use multicad_model::{Compiler, EvaluationCache};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> Session {
    let mut source = Session::new(Document::new("Pure template").unwrap()).unwrap();
    let stock = model::PrimitiveRequest::new(
        "Box",
        model::PrimitiveParameters::Box {
            size: [model::length(10.), model::length(20.), model::length(5.)],
            frame: Default::default(),
        },
    );
    source
        .commit(
            model::prepare_primitive(&source, 0, actor(), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let author =
        SymbolAuthorRequest::new("Pinned box", stock.body, SymbolKind::Static, vec![], None);
    source
        .commit(
            symbols::prepare_author(&source, source.revision(), actor(), &author)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let definition = symbols::read_definition(source.document(), author.definition).unwrap();
    let mut target = Session::new(Document::new("Unplaced pin").unwrap()).unwrap();
    target
        .commit(
            symbols::prepare_import(&target, 0, actor(), &definition)
                .unwrap()
                .prepared,
        )
        .unwrap();
    target
}
#[test]
fn completed_template_proofs_reuse_exact_pins_and_invalidate_policy_and_source() {
    let mut session = fixture();
    let mut cache = EvaluationCache::default();
    let compiler = Compiler::default();
    compiler
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert_eq!(cache.symbol_template_native_checks(), 1);
    assert_eq!(cache.symbol_template_proofs(), 1);
    compiler
        .compile_cached(&mut cache, session.document(), session.revision() + 1, None)
        .unwrap();
    assert_eq!(cache.symbol_template_proof_hits(), 1);
    assert_eq!(cache.symbol_template_native_checks(), 1);
    let changed = Compiler {
        omit_display: true,
        ..compiler
    };
    changed
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert_eq!(cache.symbol_template_native_checks(), 2);
    let (id, mut definition) = symbols::definitions(session.document())
        .unwrap()
        .pop()
        .unwrap();
    definition.name = "Different exact pin".into();
    let mut entity = session.document().get(id).unwrap().clone();
    let EntityData::Extension {
        definition: stored, ..
    } = &mut entity.data
    else {
        panic!("definition extension")
    };
    *stored = definition.definition().unwrap();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Change pin".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
    compiler
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert_eq!(cache.symbol_template_native_checks(), 3);
    cache.clear();
    assert_eq!(cache.symbol_template_proofs(), 0);
    compiler
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert_eq!(cache.symbol_template_native_checks(), 4);
}
#[test]
fn failed_or_cancelled_compiles_publish_no_template_proof_and_warm_proofs_do_not_skip_geometry() {
    let mut session = fixture();
    let mut cache = EvaluationCache::default();
    let compiler = Compiler::default();
    let stock = model::PrimitiveRequest::new(
        "Invalid ordinary box",
        model::PrimitiveParameters::Box {
            size: [model::length(10.), model::length(20.), model::length(5.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, session.revision(), actor(), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let valid = session.document().get(stock.feature).unwrap().clone();
    let mut invalid = valid.clone();
    let EntityData::Feature { definition, .. } = &mut invalid.data else {
        unreachable!()
    };
    let mut source = model::PrimitiveDefinition::from_definition(definition).unwrap();
    let model::PrimitiveParameters::Box { size, .. } = &mut source.parameters else {
        unreachable!()
    };
    size[0] = model::length(-1.);
    *definition = source.definition().unwrap();
    let edit = |session: &mut Session, entity| {
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Change ordinary geometry".into(),
                edits: vec![Mutation::Replace(entity)],
            })
            .unwrap();
    };
    edit(&mut session, invalid.clone());
    assert!(
        compiler
            .compile_cached(&mut cache, session.document(), session.revision(), None)
            .is_err()
    );
    assert_eq!(cache.symbol_template_native_checks(), 1);
    assert_eq!(cache.symbol_template_proofs(), 0);
    edit(&mut session, valid);
    assert!(
        compiler
            .compile_cached(
                &mut cache,
                session.document(),
                session.revision(),
                Some(&|| true)
            )
            .is_err()
    );
    assert_eq!(cache.symbol_template_proofs(), 0);
    compiler
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert_eq!(cache.symbol_template_native_checks(), 2);
    assert_eq!(cache.symbol_template_proofs(), 1);
    edit(&mut session, invalid);
    assert!(
        compiler
            .compile_cached(&mut cache, session.document(), session.revision(), None)
            .is_err()
    );
    assert_eq!(cache.symbol_template_proof_hits(), 1);
}
#[test]
fn zero_capacity_disables_template_proof_retention() {
    let session = fixture();
    let mut cache = EvaluationCache::new(0, 0);
    for _ in 0..2 {
        Compiler::default()
            .compile_cached(&mut cache, session.document(), session.revision(), None)
            .unwrap();
    }
    assert_eq!(cache.symbol_template_proofs(), 0);
    assert_eq!(cache.symbol_template_native_checks(), 2);
}
#[test]
fn interruption_inside_template_validation_cannot_seed_a_later_hit() {
    let session = fixture();
    let compiler = Compiler::default();
    let mut cache = EvaluationCache::default();
    let checkpoints = std::cell::Cell::new(0);
    let interrupt = || {
        checkpoints.set(checkpoints.get() + 1);
        checkpoints.get() >= 3
    };
    assert!(
        compiler
            .compile_cached(
                &mut cache,
                session.document(),
                session.revision(),
                Some(&interrupt)
            )
            .is_err()
    );
    assert_eq!(cache.symbol_template_native_checks(), 1);
    assert_eq!(cache.symbol_template_proofs(), 0);
    compiler
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert_eq!(cache.symbol_template_native_checks(), 2);
    assert_eq!(cache.symbol_template_proof_hits(), 0);
}
#[test]
fn receipts_obey_owner_capacity_and_policy_changes_require_new_native_proof() {
    let session = fixture();
    let mut cache = EvaluationCache::new(1024, 1);
    let original = Compiler::default();
    let mut changed = original;
    changed.policy.minimum_feature_mm *= 2.;
    for compiler in [original, changed, original] {
        compiler
            .compile_cached(&mut cache, session.document(), session.revision(), None)
            .unwrap();
        assert_eq!(cache.symbol_template_proofs(), 1);
    }
    assert_eq!(cache.symbol_template_native_checks(), 3);
    assert_eq!(cache.symbol_template_proof_hits(), 0);
}
