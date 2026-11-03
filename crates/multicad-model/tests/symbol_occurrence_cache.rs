use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::{self, InstanceMoveRequest, NumericTransform, length},
    project_file,
    symbols::{self, SymbolEditRequest, SymbolPlaceRequest, SymbolUpdateRequest, SymbolValue},
};
use multicad_model::{Compiler, EvaluationCache, Model};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn fixture(material: bool) -> (Session, EntityId) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/screw-socket-m3-l30.tcd");
    let bytes = std::fs::read(path).unwrap();
    let project = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let mut definition = symbols::definitions(&project.document)
        .unwrap()
        .pop()
        .unwrap()
        .1;
    if material {
        let mut template = Session::new(definition.template.clone()).unwrap();
        let value = Entity::new(
            "Fixture material",
            EntityData::Material {
                density_kg_m3: Some(7800.),
                color_rgba: [100, 110, 120, 255],
                section_hatch: None,
            },
        );
        let mut root = template.document().get(definition.root).unwrap().clone();
        let EntityData::Part { material, .. } = &mut root.data else {
            unreachable!()
        };
        *material = Some(value.id);
        replace(
            &mut template,
            vec![Mutation::Insert(value), Mutation::Replace(root)],
        );
        definition.template = template.document().clone();
    }
    let mut session = Session::new(Document::new("Actual pinned screw cache").unwrap()).unwrap();
    let prepared = symbols::prepare_import(&session, 0, actor(), &definition).unwrap();
    let id = prepared.definition;
    session.commit(prepared.prepared).unwrap();
    (session, id)
}
fn replace(session: &mut Session, edits: Vec<Mutation>) {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Cache boundary fixture change".into(),
            edits,
        })
        .unwrap();
}
fn place(session: &mut Session, definition: EntityId, x: f64) -> SymbolPlaceRequest {
    let request = SymbolPlaceRequest::new(
        session.document(),
        definition,
        "Pinned screw",
        BTreeMap::new(),
        Placement {
            translation_mm: [x, 0., 0.],
            ..Default::default()
        },
    )
    .unwrap();
    let prepared = symbols::prepare_place(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    request
}
fn nodes(request: &SymbolPlaceRequest) -> BTreeSet<EntityId> {
    BTreeSet::from([request.member, request.wrapper, request.instance])
}
fn compare(a: &Model, b: &Model, revision: u64) {
    let (a, b) = (a.snapshot(), b.snapshot());
    assert_eq!(a.stamp, b.stamp);
    assert_eq!(a.stamp.revision, revision);
    assert_eq!(a.references, b.references);
    assert_eq!(a.bodies, b.bodies);
    assert_eq!(a.body_sources, b.body_sources);
    assert_eq!(a.consumed_bodies, b.consumed_bodies);
    assert_eq!(
        a.shapes.keys().collect::<Vec<_>>(),
        b.shapes.keys().collect::<Vec<_>>()
    );
    for (id, x) in &a.shapes {
        let y = &b.shapes[id];
        assert_eq!(x.topology, y.topology);
        assert_eq!(x.solid_count, y.solid_count);
        assert!(
            (x.volume_mm3 - y.volume_mm3).abs() < 1e-7 * (x.volume_mm3.abs() + y.volume_mm3.abs())
        );
        assert_eq!(x.bounds, y.bounds);
        assert_eq!(x.topology.revision, revision);
    }
    assert_eq!(
        a.occurrences.keys().collect::<Vec<_>>(),
        b.occurrences.keys().collect::<Vec<_>>()
    );
    for (id, x) in &a.occurrences {
        let y = &b.occurrences[id];
        assert_eq!(x.instance, y.instance);
        assert_eq!(x.component, y.component);
        assert_eq!(x.producer, y.producer);
        assert_eq!(x.pose, y.pose);
        assert_eq!(x.bounds, y.bounds);
        assert_eq!(x.topology, y.topology);
        assert_eq!(x.topology.revision, revision);
    }
    assert_eq!(
        a.leaf_occurrences.keys().collect::<Vec<_>>(),
        b.leaf_occurrences.keys().collect::<Vec<_>>()
    );
    for (path, x) in &a.leaf_occurrences {
        let y = &b.leaf_occurrences[path];
        assert_eq!(x.path, y.path);
        assert_eq!(x.component, y.component);
        assert_eq!(x.producer, y.producer);
        assert_eq!(x.pose, y.pose);
        assert_eq!(x.bounds, y.bounds);
        assert_eq!(x.topology, y.topology);
        assert_eq!(x.topology.revision, revision);
    }
}
fn run(session: &Session, cache: &mut EvaluationCache) -> Model {
    compiler()
        .compile_cached(cache, session.document(), session.revision(), None)
        .unwrap()
}
#[test]
fn second_actual_catalog_placement_reuses_only_old_owned_nodes_with_complete_current_metadata() {
    let (mut session, definition) = fixture(false);
    let first = place(&mut session, definition, 0.);
    let mut cache = EvaluationCache::default();
    run(&session, &mut cache);
    let second = place(&mut session, definition, 20.);
    let cached = run(&session, &mut cache);
    assert!(nodes(&first).is_subset(&cache.last_compile().symbol_reused));
    assert!(nodes(&second).is_subset(&cache.last_compile().symbol_evaluated));
    assert!(nodes(&second).is_disjoint(&cache.last_compile().symbol_reused));
    let fresh = compiler()
        .compile(session.document(), session.revision())
        .unwrap();
    compare(&cached, &fresh, session.revision());
    for id in [
        first.member,
        first.wrapper,
        first.instance,
        second.member,
        second.wrapper,
        second.instance,
    ] {
        let (x, y) = (cached.solid(id).unwrap(), fresh.solid(id).unwrap());
        assert_eq!(x.bounds().unwrap(), y.bounds().unwrap());
        assert!((x.volume_mm3().unwrap() - y.volume_mm3().unwrap()).abs() < 1e-5);
    }
}
#[test]
fn source_override_pose_pin_policy_mesh_and_material_changes_cannot_reuse_old_node_proofs() {
    let (mut session, definition) = fixture(true);
    let first = place(&mut session, definition, 0.);
    let mut cache = EvaluationCache::default();
    run(&session, &mut cache);
    let pin = symbols::read_definition(session.document(), definition).unwrap();
    let parameter = pin
        .exposed
        .iter()
        .find(|id| pin.template.get(**id).unwrap().name == "underhead_length")
        .copied()
        .unwrap();
    let edit = SymbolEditRequest {
        record: first.record,
        overrides: BTreeMap::from([(
            parameter,
            SymbolValue {
                value: 31.,
                unit: "mm".into(),
            },
        )]),
    };
    session
        .commit(
            symbols::prepare_edit(&session, session.revision(), actor(), &edit)
                .unwrap()
                .prepared,
        )
        .unwrap();
    run(&session, &mut cache);
    assert!(nodes(&first).is_disjoint(&cache.last_compile().symbol_reused));
    let movement = InstanceMoveRequest::new(
        first.instance,
        NumericTransform {
            translation: [length(12.), length(3.), length(0.)],
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_instance_moves(&session, session.revision(), actor(), &[movement])
                .unwrap()
                .prepared,
        )
        .unwrap();
    run(&session, &mut cache);
    assert!(!cache.last_compile().symbol_reused.contains(&first.instance));
    // Return through canonical history to the prior literal placement; do not
    // independently mutate a symbol-owned source or generated parameter.
    session.undo(session.revision(), actor()).unwrap();
    run(&session, &mut cache);
    run(&session, &mut cache);
    assert!(nodes(&first).is_subset(&cache.last_compile().symbol_reused));
    let mut changed = compiler();
    changed.policy.minimum_feature_mm *= 2.;
    changed
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert!(nodes(&first).is_disjoint(&cache.last_compile().symbol_reused));
    changed.mesh.linear_deflection_mm *= 0.5;
    changed
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    assert!(nodes(&first).is_disjoint(&cache.last_compile().symbol_reused));
    run(&session, &mut cache);
    run(&session, &mut cache);
    assert!(nodes(&first).is_subset(&cache.last_compile().symbol_reused));
    let mut replacement = symbols::read_definition(session.document(), definition).unwrap();
    replacement.revision.push_str("-cache-test");
    let update = SymbolUpdateRequest::new(session.document(), definition, replacement).unwrap();
    session
        .commit(
            symbols::prepare_update(&session, session.revision(), actor(), &update)
                .unwrap()
                .prepared,
        )
        .unwrap();
    run(&session, &mut cache);
    assert!(nodes(&first).is_disjoint(&cache.last_compile().symbol_reused));
    run(&session, &mut cache);
    assert!(nodes(&first).is_subset(&cache.last_compile().symbol_reused));
    let mut replacement = symbols::read_definition(session.document(), definition).unwrap();
    let mut template = Session::new(replacement.template.clone()).unwrap();
    let mut material = template
        .document()
        .entities()
        .find(|entity| matches!(entity.data, EntityData::Material { .. }))
        .unwrap()
        .clone();
    let EntityData::Material { color_rgba, .. } = &mut material.data else {
        unreachable!()
    };
    *color_rgba = [200, 50, 20, 255];
    replace(&mut template, vec![Mutation::Replace(material)]);
    replacement.template = template.document().clone();
    replacement.revision.push_str("-material");
    let update = SymbolUpdateRequest::new(session.document(), definition, replacement).unwrap();
    session
        .commit(
            symbols::prepare_update(&session, session.revision(), actor(), &update)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let cached = run(&session, &mut cache);
    assert!(nodes(&first).is_disjoint(&cache.last_compile().symbol_reused));
    compare(
        &cached,
        &compiler()
            .compile(session.document(), session.revision())
            .unwrap(),
        session.revision(),
    );
}
#[test]
fn superseded_and_failed_candidates_publish_no_new_symbol_nodes_and_preserve_old_receipts() {
    let (mut session, definition) = fixture(false);
    let first = place(&mut session, definition, 0.);
    let original = session.document().clone();
    let revision = session.revision();
    let mut cache = EvaluationCache::default();
    run(&session, &mut cache);
    let committed = cache.symbol_node_entries();
    place(&mut session, definition, 20.);
    let mut dry_cache = EvaluationCache::default();
    compiler()
        .compile_cached(&mut dry_cache, &original, revision, None)
        .unwrap();
    let count = Cell::new(0usize);
    compiler()
        .compile_cached(
            &mut dry_cache,
            session.document(),
            session.revision(),
            Some(&|| {
                count.set(count.get() + 1);
                false
            }),
        )
        .unwrap();
    let last = count.get();
    assert!(last > 1);
    let observed = Cell::new(0usize);
    assert!(
        compiler()
            .compile_cached(
                &mut cache,
                session.document(),
                session.revision(),
                Some(&|| {
                    observed.set(observed.get() + 1);
                    observed.get() >= last
                })
            )
            .is_err()
    );
    assert_eq!(cache.symbol_node_entries(), committed);
    let mut invalid = session.document().clone();
    let mut rejected = Session::new(invalid.clone()).unwrap();
    let stock = model::PrimitiveRequest::new(
        "Bad ordinary geometry",
        model::PrimitiveParameters::Box {
            size: [length(2.), length(2.), length(2.)],
            frame: Default::default(),
        },
    );
    rejected
        .commit(
            model::prepare_primitive(&rejected, 0, actor(), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut entity = rejected.document().get(stock.feature).unwrap().clone();
    let EntityData::Feature { definition, .. } = &mut entity.data else {
        unreachable!()
    };
    let mut primitive = model::PrimitiveDefinition::from_definition(definition).unwrap();
    let model::PrimitiveParameters::Box { size, .. } = &mut primitive.parameters else {
        unreachable!()
    };
    size[0] = length(-1.);
    *definition = primitive.definition().unwrap();
    replace(&mut rejected, vec![Mutation::Replace(entity)]);
    invalid = rejected.document().clone();
    assert!(
        compiler()
            .compile_cached(&mut cache, &invalid, rejected.revision(), None)
            .is_err()
    );
    assert_eq!(cache.symbol_node_entries(), committed);
    compiler()
        .compile_cached(&mut cache, &original, revision, None)
        .unwrap();
    assert!(nodes(&first).is_subset(&cache.last_compile().symbol_reused));
}
#[test]
fn zero_capacity_disables_symbol_node_retention() {
    let (mut session, definition) = fixture(false);
    let first = place(&mut session, definition, 0.);
    let mut cache = EvaluationCache::new(0, 0);
    for _ in 0..2 {
        run(&session, &mut cache);
        assert_eq!(cache.symbol_node_entries(), 0);
        assert!(nodes(&first).is_disjoint(&cache.last_compile().symbol_reused));
    }
}
