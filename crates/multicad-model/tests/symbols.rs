#[path = "../../multicad-app/examples/symbol_review.rs"]
mod review;
use multicad_core::{
    commands::{Request, Session},
    document::{Document, EntityData, EntityId, Mutation, Placement},
    model::{self, InstanceMoveRequest, NumericTransform, length},
    project_file::ContentHash,
    references::{self, AnalyticGeometry, GeometryIntent, Resolution},
    symbols::{
        self, SymbolEditRequest, SymbolKind, SymbolPlaceRequest, SymbolUpdateRequest, SymbolValue,
    },
};
use multicad_model::Compiler;
use std::collections::{BTreeMap, BTreeSet};
const ORIGINAL: [[f64; 3]; 2] = [[150., 90., 8.], [200., 100., 12.]];
const UPGRADED: [[f64; 3]; 2] = [[140., 95., 8.], [190., 95., 12.]];
#[test]
fn independent_native_variants_preserve_internal_constraints_plan_and_topology_identity()
-> Result<(), review::Error> {
    let mut f = review::seed(false)?;
    review::verify(&f, ORIGINAL)?;
    let pinned = symbols::read_definition(f.session.document(), f.definition)?;
    assert_eq!(symbols::definitions(f.session.document())?.len(), 1);
    let first = symbols::read_instance(f.session.document(), f.placed[0].instance)?;
    let second = symbols::read_instance(f.session.document(), f.placed[1].instance)?;
    assert_eq!(first.pin, second.pin);
    assert_eq!(first.pin, pinned.pin()?);
    assert!(
        first
            .ids
            .values()
            .collect::<BTreeSet<_>>()
            .is_disjoint(&second.ids.values().collect::<BTreeSet<_>>())
    );
    assert_ne!(f.plans[&first.instance], f.plans[&second.instance]);
    let model = Compiler::default().compile(f.session.document(), f.session.revision())?;
    let producer = first.ids[&f.library.panel.output.feature];
    let topology = &model.snapshot().shapes[&producer].topology;
    let face=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if(frame.origin_mm[2]-8.).abs()<1e-7)).unwrap();
    let intent = GeometryIntent::capture(topology, face.key)?;
    let before = f.session.document().clone();
    let before_hash = ContentHash::document(&before)?;
    let mut overrides = first.overrides.clone();
    overrides.insert(
        f.library.panel.width,
        SymbolValue {
            value: 175.,
            unit: "mm".into(),
        },
    );
    let prepared = symbols::prepare_edit(
        &f.session,
        f.session.revision(),
        review::actor(),
        &SymbolEditRequest {
            record: f.placed[0].record,
            overrides,
        },
    )?;
    assert_eq!(f.session.document(), &before);
    review::checked(&mut f.session, prepared.prepared)?;
    review::verify(&f, [[175., 90., 8.], ORIGINAL[1]])?;
    assert_eq!(
        symbols::read_definition(f.session.document(), f.definition)?,
        pinned
    );
    assert_eq!(
        symbols::read_instance(f.session.document(), f.placed[0].record)?.ids,
        first.ids
    );
    let changed = Compiler::default().compile(f.session.document(), f.session.revision())?;
    assert!(matches!(
        references::resolve(
            &intent,
            &changed.snapshot().shapes[&producer].topology,
            f.session.revision()
        ),
        Resolution::Resolved { .. }
    ));
    assert_ne!(changed.snapshot().stamp.source_hash, before_hash);
    f.session.undo(f.session.revision(), review::actor())?;
    assert_eq!(f.session.document(), &before);
    review::verify(&f, ORIGINAL)?;
    f.session.redo(f.session.revision(), review::actor())?;
    review::verify(&f, [[175., 90., 8.], ORIGINAL[1]])?;
    // The current native occurrence pose also controls the linked plan after a
    // normal placement driver is added; the original literal is insufficient.
    let moved = InstanceMoveRequest::new(
        first.instance,
        NumericTransform {
            translation: [length(25.), length(10.), length(0.)],
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_instance_moves(&f.session, f.session.revision(), review::actor(), &[moved])?;
    review::checked(&mut f.session, prepared.prepared)?;
    let current = Compiler::default().compile(f.session.document(), f.session.revision())?;
    let (outer, plan) = symbols::symbol_plan(f.session.document(), first.instance)?.unwrap();
    assert_eq!(outer, first.instance);
    assert_eq!(plan, f.plans[&first.instance]);
    let evaluated = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        plan,
        Some(&current.snapshot().references),
    )?;
    let world = current.snapshot().occurrences[&outer]
        .pose
        .point(evaluated.frame.point([0., 0.]));
    assert_eq!(world, [275., 60., 0.]);
    let leaf = current
        .snapshot()
        .leaf_occurrences
        .values()
        .find(|l| l.path.root == outer)
        .unwrap();
    for (i, v) in world.into_iter().enumerate() {
        review::near(v, leaf.bounds.min[i])?;
    }
    let bytes = review::archive(f.session.document())?;
    let reopened = review::reopen(&bytes)?;
    assert_eq!(reopened.document(), f.session.document());
    let native = Compiler::default().compile(reopened.document(), reopened.revision())?;
    review::near(
        native.solid(first.instance).unwrap().volume_mm3()?,
        175. * 90. * 8.,
    )?;
    Ok(())
}
#[test]
fn native_assembly_upgrade_is_explicit_atomic_compatible_and_offline_pinned()
-> Result<(), review::Error> {
    let mut f = review::seed(true)?;
    review::verify(&f, ORIGINAL)?;
    let old = symbols::read_definition(f.session.document(), f.definition)?;
    let first_source = symbols::read_instance(f.session.document(), f.placed[0].record)?;
    let producer = first_source.ids[&f.library.panel.output.feature];
    let baseline = Compiler::default().compile(f.session.document(), f.session.revision())?;
    let source_topology = &baseline.snapshot().shapes[&producer].topology;
    let face=source_topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if(frame.origin_mm[2]-8.).abs()<1e-7)).unwrap();
    let reference = GeometryIntent::capture(source_topology, face.key)?;
    let records = symbols::instances(f.session.document(), f.definition)?;
    let before = f.session.document().clone();
    let replacement = review::publish_revision_two(&mut f.library)?;
    assert_ne!(old.pin()?, replacement.pin()?);
    assert_eq!(f.session.document(), &before);
    assert_eq!(
        symbols::read_definition(f.session.document(), f.definition)?,
        old
    );
    review::verify(&f, ORIGINAL)?;
    let report = symbols::compare_update(f.session.document(), f.definition, &replacement)?;
    assert_eq!(
        report
            .affected_instances
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        f.placed.iter().map(|p| p.instance).collect()
    );
    assert_eq!(report.removed_parameters, vec![f.library.panel.height]);
    assert_eq!(report.added_parameters, vec![f.library.panel.depth]);
    assert_eq!(report.conflicts.len(), 2);
    assert!(report.changed_entities.contains(&f.library.panel.sketch));
    let request =
        SymbolUpdateRequest::new(f.session.document(), f.definition, replacement.clone())?;
    assert!(
        symbols::prepare_update(&f.session, f.session.revision(), review::actor(), &request)
            .is_err()
    );
    assert_eq!(f.session.document(), &before);
    review::update(&mut f, replacement.clone())?;
    review::verify(&f, UPGRADED)?;
    let updated_native = Compiler::default().compile(f.session.document(), f.session.revision())?;
    assert!(
        matches!(
            references::resolve(
                &reference,
                &updated_native.snapshot().shapes[&producer].topology,
                f.session.revision()
            ),
            Resolution::Resolved { .. }
        ),
        "Compatible symbol upgrade must retain the actual named native face reference"
    );
    for (record, prior) in &records {
        let now = symbols::read_instance(f.session.document(), *record)?;
        assert_eq!(now.ids, prior.ids);
        assert_eq!(now.instance, prior.instance);
        assert_eq!(now.pin, replacement.pin()?);
        assert!(!now.overrides.contains_key(&f.library.panel.height));
        assert_eq!(
            now.overrides[&f.library.panel.width],
            prior.overrides[&f.library.panel.width]
        );
    }
    let updated = f.session.document().clone();
    f.session.undo(f.session.revision(), review::actor())?;
    assert_eq!(f.session.document(), &before);
    review::verify(&f, ORIGINAL)?;
    f.session.redo(f.session.revision(), review::actor())?;
    assert_eq!(f.session.document(), &updated);
    review::verify(&f, UPGRADED)?;
    f.session = review::reopen(&review::archive(f.session.document())?)?;
    review::verify(&f, UPGRADED)?;
    let parts = multicad_core::exchange::resolve_parts(f.session.document(), &Default::default())?;
    assert_eq!(
        parts.len(),
        4,
        "Only physical placed members appear in manufacturing scopes"
    );
    let native = Compiler::default().compile(f.session.document(), f.session.revision())?;
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("real-symbol-assembly.step");
    native
        .solid(f.placed[1].instance)
        .unwrap()
        .write_step(&path)?;
    let exact = multicad_kernel::Solid::read_step(&path, Default::default())?;
    assert_eq!(exact.solid_count()?, 2);
    review::near(exact.volume_mm3()?, 190. * 95. * 12. * 2.)?;
    // A newly exposed derived parameter can be explicitly overridden for one
    // placement, while the pinned expression and the other placement stay intact.
    let before_expression_override = f.session.document().clone();
    let mut expression_values =
        symbols::read_instance(f.session.document(), f.placed[0].record)?.overrides;
    expression_values.insert(
        f.library.panel.depth,
        SymbolValue {
            value: 11.,
            unit: "mm".into(),
        },
    );
    let depth_edit = symbols::prepare_edit(
        &f.session,
        f.session.revision(),
        review::actor(),
        &SymbolEditRequest {
            record: f.placed[0].record,
            overrides: expression_values,
        },
    )?;
    review::checked(&mut f.session, depth_edit.prepared)?;
    review::verify(&f, [[140., 95., 11.], UPGRADED[1]])?;
    let overridden = symbols::read_instance(f.session.document(), f.placed[0].record)?;
    let EntityData::Parameter { inputs, .. } = &f
        .session
        .document()
        .get(overridden.ids[&f.library.panel.depth])
        .unwrap()
        .data
    else {
        panic!()
    };
    assert!(
        inputs.is_empty(),
        "Explicit expression override must clear the derived private dependency"
    );
    assert_eq!(
        symbols::read_definition(f.session.document(), f.definition)?
            .template
            .get(f.library.panel.depth),
        replacement.template.get(f.library.panel.depth)
    );
    f.session.undo(f.session.revision(), review::actor())?;
    assert_eq!(f.session.document(), &before_expression_override);
    review::verify(&f, UPGRADED)?;
    // Valid source values can still make a dependent sketch unsatisfiable. This
    // is an explicit native candidate preflight, not a core-source rejection.
    let before = f.session.document().clone();
    let mut values = symbols::read_instance(&before, f.placed[0].record)?.overrides;
    values.insert(
        f.library.panel.width,
        SymbolValue {
            value: 5.,
            unit: "mm".into(),
        },
    );
    let invalid = symbols::prepare_edit(
        &f.session,
        f.session.revision(),
        review::actor(),
        &SymbolEditRequest {
            record: f.placed[0].record,
            overrides: values,
        },
    )?;
    assert!(
        Compiler::default()
            .compile(invalid.prepared.document(), f.session.revision())
            .is_err()
    );
    assert_eq!(f.session.document(), &before);
    review::verify(&f, UPGRADED)?;
    Ok(())
}
#[test]
fn static_overrides_invalid_units_and_owned_edits_are_rejected_without_mutation()
-> Result<(), review::Error> {
    let library = review::library(false, SymbolKind::Static)?;
    let def =
        symbols::read_definition(library.panel.session.document(), library.author.definition)?;
    let mut session = Session::new(Document::new("No external library resources")?)?;
    let imported = symbols::prepare_import(&session, 0, review::actor(), &def)?;
    let definition = imported.definition;
    review::checked(&mut session, imported.prepared)?;
    let placed = SymbolPlaceRequest::new(
        session.document(),
        definition,
        "Fixed static panel",
        BTreeMap::new(),
        Placement::default(),
    )?;
    let prepared = symbols::prepare_place(&session, session.revision(), review::actor(), &placed)?;
    review::checked(&mut session, prepared.prepared)?;
    let native = Compiler::default().compile(session.document(), session.revision())?;
    review::near(
        native.solid(placed.instance).unwrap().volume_mm3()?,
        120. * 80. * 8.,
    )?;
    let before = session.document().clone();
    let bad = SymbolPlaceRequest::new(
        &before,
        definition,
        "Illegal dynamic value",
        BTreeMap::from([(
            library.panel.width,
            SymbolValue {
                value: 1.,
                unit: "mm".into(),
            },
        )]),
        Default::default(),
    )?;
    assert!(symbols::prepare_place(&session, session.revision(), review::actor(), &bad).is_err());
    assert_eq!(session.document(), &before);
    let mut f = review::seed(false)?;
    for (id, value) in [
        (
            f.library.panel.width,
            SymbolValue {
                value: 1.,
                unit: "deg".into(),
            },
        ),
        (
            f.library.panel.width,
            SymbolValue {
                value: f64::NAN,
                unit: "mm".into(),
            },
        ),
        (
            EntityId::new(),
            SymbolValue {
                value: 1.,
                unit: "mm".into(),
            },
        ),
    ] {
        let before = f.session.document().clone();
        let mut values = symbols::read_instance(&before, f.placed[0].record)?.overrides;
        values.insert(id, value);
        assert!(
            symbols::prepare_edit(
                &f.session,
                f.session.revision(),
                review::actor(),
                &SymbolEditRequest {
                    record: f.placed[0].record,
                    overrides: values
                }
            )
            .is_err()
        );
        assert_eq!(f.session.document(), &before);
    }
    let owner = symbols::read_instance(f.session.document(), f.placed[0].record)?;
    let mut private = f
        .session
        .document()
        .get(owner.ids[&f.library.panel.width])
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut private.data else {
        panic!()
    };
    *expression = "999".into();
    let before = f.session.document().clone();
    assert!(
        f.session
            .apply(Request {
                expected_revision: f.session.revision(),
                actor: review::actor(),
                label: "Forbidden independent private edit".into(),
                edits: vec![Mutation::Replace(private)]
            })
            .is_err()
    );
    assert_eq!(f.session.document(), &before);
    review::verify(&f, ORIGINAL)?;
    Ok(())
}
#[test]
fn review_files_are_native_editable_and_reopen_without_library_or_sidecar_resources()
-> Result<(), review::Error> {
    let temp = tempfile::tempdir()?;
    let directory = temp.path().join("native-symbol-review");
    let report = review::generate(&directory)?;
    let projects = report["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 4);
    for project in projects {
        let bytes = std::fs::read(directory.join(project["file"].as_str().unwrap()))?;
        let opened = review::reopen(&bytes)?;
        assert_eq!(symbols::definitions(opened.document())?.len(), 1);
        assert_eq!(
            symbols::instances(
                opened.document(),
                EntityId::parse(project["definition"].as_str().unwrap())?
            )?
            .len(),
            2
        );
        Compiler::default().compile(opened.document(), opened.revision())?;
    }
    Ok(())
}
