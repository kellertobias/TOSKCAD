//! Several independently broken sketch references are staged and repaired as
//! one checked native transaction.
mod support;
use multicad_core::{
    commands::{ErrorCode, Session},
    constraint_sketch::source::Source,
    document::{Document, EntityData, EntityId, Mutation},
    model,
    project_file::{self, ProjectFile},
    sketch_external::{self, RepairSet, RepairStamp, RepairStep, document_edits},
};
use multicad_model::Compiler;
use support::broken_references::{Fixture, actor, broken, edge};
fn set(session: &Session, steps: Vec<RepairStep>) -> RepairSet {
    RepairSet {
        stamp: RepairStamp::current(session.document(), session.revision(), None).unwrap(),
        steps,
    }
}
fn volume(compiler: &Compiler, document: &Document, revision: u64, part: EntityId) -> (f64, f64) {
    let model = compiler.compile(document, revision).unwrap();
    let shape = &model.snapshot().shapes[&part];
    (shape.volume_mm3, shape.bounds.min[0])
}

#[test]
fn two_broken_references_repair_in_one_checked_transaction() {
    let compiler = Compiler::default();
    let Fixture {
        mut session,
        a,
        b,
        replacement_a,
        replacement_b,
    } = broken(true);
    let broken = session.document().clone();
    let base = session.revision();
    let message = compiler
        .compile(&broken, base)
        .err()
        .expect("both references are broken")
        .to_string();
    assert!(!message.is_empty());
    let key_a = edge(&compiler, &broken, base, replacement_a.feature, 40.);
    let key_b = edge(&compiler, &broken, base, replacement_b.feature, 50.);

    // The single-reference path cannot pass the full-model gate while the
    // other independent reference remains broken.
    let single = multicad_solver::prepare_external_repair(
        &session,
        base,
        actor(),
        &multicad_solver::ExternalRepairRequest {
            sketch: a.sketch,
            reference: a.reference,
            source: replacement_a.feature,
            selected: key_a,
        },
        &compiler
            .compile_sources(&broken, base, &[replacement_a.feature])
            .unwrap()
            .snapshot()
            .shapes
            .iter()
            .map(|(id, s)| (*id, s.topology.clone()))
            .collect(),
    )
    .unwrap();
    assert!(
        compiler
            .compile(single.preview.unwrap().document(), base)
            .is_err()
    );

    // Staging only one row keeps Apply disabled and names the other reference.
    let partial = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &set(
                &session,
                vec![RepairStep::Rebind {
                    sketch: a.sketch,
                    reference: a.reference,
                    source: replacement_a.feature,
                    selected: key_a,
                }],
            ),
            None,
            None,
        )
        .unwrap();
    assert!(!partial.can_apply());
    assert!(partial.prepared.is_none());
    assert!(partial.rows[0].error.is_none());
    assert!(
        partial
            .blockers
            .iter()
            .any(|b| b.contains("rebuild") || b.contains("unresolved")),
        "{:?}",
        partial.blockers
    );

    // An invalid row is reported and blocks Apply; the live document is unchanged.
    let face = compiler
        .compile_sources(&broken, base, &[replacement_b.feature])
        .unwrap()
        .snapshot()
        .shapes[&replacement_b.feature]
        .topology
        .elements
        .iter()
        .find(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
        .unwrap()
        .key;
    let invalid = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &set(
                &session,
                vec![
                    RepairStep::Rebind {
                        sketch: a.sketch,
                        reference: a.reference,
                        source: replacement_a.feature,
                        selected: key_a,
                    },
                    RepairStep::Rebind {
                        sketch: b.sketch,
                        reference: b.reference,
                        source: replacement_b.feature,
                        selected: face,
                    },
                ],
            ),
            None,
            None,
        )
        .unwrap();
    assert!(!invalid.can_apply());
    assert!(invalid.rows[0].error.is_none());
    assert!(
        invalid.rows[1]
            .error
            .as_deref()
            .unwrap()
            .contains("wrong kind")
    );
    assert!(invalid.blockers[0].starts_with("Row 2:"));
    assert_eq!(session.document(), &broken);

    // The complete set validates and becomes commit-capable.
    let complete = set(
        &session,
        vec![
            RepairStep::Rebind {
                sketch: a.sketch,
                reference: a.reference,
                source: replacement_a.feature,
                selected: key_a,
            },
            RepairStep::Rebind {
                sketch: b.sketch,
                reference: b.reference,
                source: replacement_b.feature,
                selected: key_b,
            },
        ],
    );
    let checked = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &complete,
            None,
            None,
        )
        .unwrap();
    assert!(checked.can_apply(), "{:?}", checked.blockers);
    assert_eq!(checked.sketches.len(), 2);
    assert!(checked.rows.iter().all(|r| r.old.contains("Original wall")));
    assert!(checked.rows[0].new.contains("Replacement wall A"));
    assert!(checked.rows[0].affected.contains(&a.part));
    assert!(checked.report.as_ref().unwrap().all_resolved());
    assert_eq!(session.document(), &broken);

    // Resource and source guards reject the staged set.
    let resources = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &complete,
            Some("changed"),
            None,
        )
        .err()
        .unwrap()
        .to_string();
    assert!(resources.contains("resources changed"), "{resources}");
    let mut foreign = Session::new(broken.clone()).unwrap();
    let error = foreign
        .commit(checked.prepared.clone().unwrap())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ForeignPreview);

    // One attributable commit; undo and redo restore the whole set.
    let activity = session.activity().count();
    let change = session.commit(checked.prepared.unwrap()).unwrap();
    assert_eq!(change.revision, base + 1);
    assert_eq!(session.activity().count(), activity + 1);
    let last = session.activity().last().unwrap();
    assert_eq!(last.label, "Repair 2 sketch references");
    assert_eq!(last.actor, actor());
    for (consumer, source) in [(&a, replacement_a.feature), (&b, replacement_b.feature)] {
        assert_eq!(
            sketch_external::read(session.document(), consumer.reference)
                .unwrap()
                .source,
            source
        );
    }
    let repaired = session.document().clone();
    let (va, xa) = volume(&compiler, &repaired, session.revision(), a.part);
    let (vb, xb) = volume(&compiler, &repaired, session.revision(), b.part);
    assert!((va - 70.).abs() < 1e-6 && (xa - 40.).abs() < 1e-6);
    assert!((vb - 70.).abs() < 1e-6 && (xb - 50.).abs() < 1e-6);
    // Compatible rebinding keeps output IDs and the dependent constraints.
    for consumer in [&a, &b] {
        let EntityData::Sketch { definition, .. } = &repaired.get(consumer.sketch).unwrap().data
        else {
            panic!()
        };
        let source = Source::from_definition(definition).unwrap();
        assert!(
            source
                .sketch
                .constraints
                .iter()
                .any(|c| c.id == consumer.coincident)
        );
    }
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &broken);
    assert!(
        compiler
            .compile(session.document(), session.revision())
            .is_err()
    );
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &repaired);

    // A set staged before any later change is stale as a whole.
    let stale = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &complete,
            None,
            None,
        )
        .err()
        .unwrap()
        .to_string();
    assert!(stale.contains("Source changed"), "{stale}");

    // Save/reopen keeps both the initial broken project and the repaired one.
    for (document, ok) in [(&broken, false), (&repaired, true)] {
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(document.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        assert_eq!(&reopened, document);
        assert_eq!(compiler.compile(&reopened, 0).is_ok(), ok);
        if ok {
            let (vb, xb) = volume(&compiler, &reopened, 0, b.part);
            assert!((vb - 70.).abs() < 1e-6 && (xb - 50.).abs() < 1e-6);
        }
    }
}

#[test]
fn a_set_creates_replacement_source_and_removes_dependents_explicitly() {
    let compiler = Compiler::default();
    let Fixture {
        mut session,
        a,
        b,
        replacement_b,
        ..
    } = broken(false);
    let broken = session.document().clone();
    let base = session.revision();
    assert!(broken.get(replacement_b.feature).is_none());

    // Replacement geometry is created as an uncommitted candidate row with an
    // ordinary command on a scratch Session.
    let scratch = Session::new(broken.clone()).unwrap();
    let created = model::prepare_primitive(&scratch, 0, actor(), &replacement_b).unwrap();
    let creation = document_edits(&broken, created.prepared.document());
    let key_b = edge(
        &compiler,
        created.prepared.document(),
        base,
        replacement_b.feature,
        50.,
    );

    // Removing A while its coincident constraint remains is refused.
    let refused = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &set(
                &session,
                vec![RepairStep::Remove {
                    sketch: a.sketch,
                    reference: a.reference,
                }],
            ),
            None,
            None,
        )
        .unwrap();
    assert!(
        refused.rows[0]
            .error
            .as_deref()
            .unwrap()
            .contains("remove that constraint")
    );

    // Reference output geometry is not removable as own items.
    let EntityData::Sketch { definition, .. } = &broken.get(a.sketch).unwrap().data else {
        panic!()
    };
    let output = sketch_external::read(&broken, a.reference)
        .unwrap()
        .definition
        .output
        .points();
    let source_a = Source::from_definition(definition).unwrap();
    assert!(source_a.sketch.points.iter().any(|p| p.id == output[0]));
    let protected = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &set(
                &session,
                vec![RepairStep::RemoveSketchItems {
                    sketch: a.sketch,
                    constraints: vec![],
                    curves: vec![],
                    points: vec![output[0]],
                }],
            ),
            None,
            None,
        )
        .unwrap();
    assert!(
        protected.rows[0]
            .error
            .as_deref()
            .unwrap()
            .contains("removed with its reference")
    );

    // A source-edit row cannot rewrite reference records.
    let tampered = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &set(
                &session,
                vec![RepairStep::SourceEdit {
                    label: "Rewrite".into(),
                    edits: vec![Mutation::Remove(a.reference)],
                }],
            ),
            None,
            None,
        )
        .unwrap();
    assert!(
        tampered.rows[0]
            .error
            .as_deref()
            .unwrap()
            .contains("rebind or remove")
    );

    let steps = vec![
        RepairStep::SourceEdit {
            label: "Create replacement wall B".into(),
            edits: creation,
        },
        RepairStep::Rebind {
            sketch: b.sketch,
            reference: b.reference,
            source: replacement_b.feature,
            selected: key_b,
        },
        RepairStep::RemoveSketchItems {
            sketch: a.sketch,
            constraints: vec![a.coincident],
            curves: vec![],
            points: vec![],
        },
        RepairStep::Remove {
            sketch: a.sketch,
            reference: a.reference,
        },
    ];
    let checked = compiler
        .prepare_repair_set(
            &session.preparation_snapshot(),
            actor(),
            &set(&session, steps),
            None,
            None,
        )
        .unwrap();
    assert!(
        checked.can_apply(),
        "{:?} {:?}",
        checked.blockers,
        checked.rows
    );
    assert_eq!(session.document(), &broken);
    session.commit(checked.prepared.unwrap()).unwrap();
    assert_eq!(session.revision(), base + 1);
    assert!(session.document().get(replacement_b.feature).is_some());
    assert!(session.document().get(a.reference).is_none());
    let (vb, xb) = volume(&compiler, session.document(), session.revision(), b.part);
    assert!((vb - 70.).abs() < 1e-6 && (xb - 50.).abs() < 1e-6);
    // Sketch A keeps its own 5 × 7 profile, no longer driven by the wall.
    let (va, _) = volume(&compiler, session.document(), session.revision(), a.part);
    assert!((va - 70.).abs() < 1e-6);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &broken);
}
