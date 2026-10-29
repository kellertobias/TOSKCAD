//! The window's staged repair set against the real native model.
#[path = "../../multicad-model/tests/support/broken_references.rs"]
mod broken_references;
use super::*;
use broken_references::{Fixture, actor, broken, edge};
use multicad_core::{
    commands::{Actor, Channel},
    document::Mutation,
    references,
};
use multicad_model::Compiler;
use std::time::{Duration, Instant};

fn checker() -> RepairSetChecker {
    Arc::new(|snapshot, set, resources, cancel| {
        let interrupt = || cancel.load(Ordering::Relaxed);
        let checked = Compiler::default()
            .prepare_repair_set(
                &snapshot,
                Actor::local(Channel::Ui),
                &set,
                Some(&resources),
                Some(&interrupt),
            )
            .map_err(|e| e.to_string())?;
        Ok(RepairSetCheck {
            rows: checked.rows,
            sketches: checked.sketches,
            blockers: checked.blockers,
            prepared: checked.prepared,
        })
    })
}
fn wait(panel: &mut RepairSetPanel, session: &Session, resources: &ProjectResources) {
    let start = Instant::now();
    while panel.checking() {
        panel.poll(session, resources);
        assert!(start.elapsed() < Duration::from_secs(60), "check timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
/// Both original walls are deleted, so both bindings lose their source.
fn missing() -> (Fixture, Session) {
    let fixture = broken(true);
    let mut session = Session::new(fixture.session.document().clone()).unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: 0,
            actor: actor(),
            label: "Delete original walls".into(),
            edits: vec![
                Mutation::Remove(fixture.original_a.body),
                Mutation::Remove(fixture.original_a.feature),
                Mutation::Remove(fixture.original_b.body),
                Mutation::Remove(fixture.original_b.feature),
            ],
        })
        .unwrap();
    (fixture, session)
}
fn report(session: &Session, sources: &[EntityId]) -> ReferenceReport {
    let model = Compiler::default()
        .compile_sources(session.document(), session.revision(), sources)
        .unwrap();
    let topologies = model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| (*id, s.topology.clone()))
        .collect();
    references::inspect(session.document(), session.revision(), &topologies)
}

#[test]
fn window_stages_checks_and_applies_two_reference_repairs_atomically() {
    let (fixture, mut session) = missing();
    let Fixture {
        a,
        b,
        replacement_a,
        replacement_b,
        ..
    } = fixture;
    let compiler = Compiler::default();
    let resources = ProjectResources::default();
    let broken_doc = session.document().clone();
    assert!(compiler.compile(&broken_doc, 1).is_err());
    let sources = [replacement_a.feature, replacement_b.feature];
    let listed = broken_references(&broken_doc, &report(&session, &sources), &BTreeSet::new());
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().all(|r| r.kind == Some(TopologyKind::Edge)));
    assert!(
        listed
            .iter()
            .all(|r| r.status == "Source geometry is unavailable")
    );

    let mut panel = RepairSetPanel::default();
    assert!(panel.request_check(&session, &resources).is_err());
    panel.set_checker(checker());
    let key_a = edge(&compiler, &broken_doc, 1, replacement_a.feature, 40.);
    let key_b = edge(&compiler, &broken_doc, 1, replacement_b.feature, 50.);
    panel
        .stage_rebind(
            &session,
            &resources,
            a.sketch,
            a.reference,
            replacement_a.feature,
            key_a,
        )
        .unwrap();
    assert!(
        panel
            .stage_rebind(
                &session,
                &resources,
                a.sketch,
                a.reference,
                replacement_a.feature,
                key_a
            )
            .unwrap_err()
            .contains("already has a staged repair")
    );
    // One staged row alone cannot enable Apply.
    panel.request_check(&session, &resources).unwrap();
    wait(&mut panel, &session, &resources);
    let partial = panel.result().unwrap();
    assert!(!partial.can_apply());
    assert!(!partial.blockers.is_empty());
    assert!(!panel.can_apply(&session, &resources));
    assert!(panel.apply(&mut session, &resources).is_err());
    assert_eq!(session.document(), &broken_doc);

    // Staging the second row invalidates the earlier verdict.
    panel
        .stage_rebind(
            &session,
            &resources,
            b.sketch,
            b.reference,
            replacement_b.feature,
            key_b,
        )
        .unwrap();
    assert!(panel.result().is_none());
    panel.request_check(&session, &resources).unwrap();
    wait(&mut panel, &session, &resources);
    let checked = panel.result().unwrap();
    assert!(checked.can_apply(), "{:?}", checked.blockers);
    assert_eq!(checked.rows.len(), 2);
    assert!(
        checked
            .rows
            .iter()
            .all(|r| r.error.is_none() && !r.old.is_empty())
    );
    assert_eq!(session.document(), &broken_doc, "checking never commits");
    assert!(panel.can_apply(&session, &resources));
    // Changed project resources disable Apply.
    let mut changed = ProjectResources::default();
    changed
        .metadata
        .insert("note".into(), serde_json::json!("changed"));
    assert!(!panel.can_apply(&session, &changed));

    panel.apply(&mut session, &resources).unwrap();
    assert!(!panel.has_pending());
    assert_eq!(session.revision(), 2);
    assert_eq!(
        session.activity().last().unwrap().label,
        "Repair 2 sketch references"
    );
    let model = compiler.compile(session.document(), 2).unwrap();
    for (part, x) in [(a.part, 40.), (b.part, 50.)] {
        let shape = &model.snapshot().shapes[&part];
        assert!((shape.volume_mm3 - 70.).abs() < 1e-6);
        assert!((shape.bounds.min[0] - x).abs() < 1e-6);
    }
    let repaired = session.document().clone();
    session.undo(2, actor()).unwrap();
    assert_eq!(session.document(), &broken_doc);
    session.redo(3, actor()).unwrap();
    assert_eq!(session.document(), &repaired);
}

#[test]
fn source_or_resource_changes_discard_the_whole_staged_set() {
    let (fixture, mut session) = missing();
    let compiler = Compiler::default();
    // Lowered walls still resolve topologically; their failed consumer
    // sketches are listed for staging all the same.
    let lowered = &fixture.session;
    let walls = [fixture.original_a.feature, fixture.original_b.feature];
    let resolved = report(lowered, &walls);
    assert!(broken_references(lowered.document(), &resolved, &BTreeSet::new()).is_empty());
    let failed = BTreeSet::from([fixture.a.sketch, fixture.b.sketch]);
    let listed = broken_references(lowered.document(), &resolved, &failed);
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .all(|r| r.status == "Consumer sketch fails to evaluate")
    );
    let resources = ProjectResources::default();
    let mut panel = RepairSetPanel::default();
    panel.set_checker(checker());
    let key = edge(
        &compiler,
        session.document(),
        1,
        fixture.replacement_a.feature,
        40.,
    );
    let stage = |panel: &mut RepairSetPanel, session: &Session, resources: &ProjectResources| {
        panel
            .stage_rebind(
                session,
                resources,
                fixture.a.sketch,
                fixture.a.reference,
                fixture.replacement_a.feature,
                key,
            )
            .unwrap();
    };
    stage(&mut panel, &session, &resources);
    // A check in flight is superseded, and an unrelated commit discards all rows.
    panel.request_check(&session, &resources).unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: 1,
            actor: actor(),
            label: "Rename".into(),
            edits: vec![Mutation::RenameDocument("Renamed".into())],
        })
        .unwrap();
    assert!(!panel.synchronize(&session, &resources));
    assert!(!panel.has_pending());
    assert!(!panel.checking());
    assert!(panel.message().unwrap().1.contains("discarded"));

    stage(&mut panel, &session, &resources);
    let mut changed = ProjectResources::default();
    changed
        .metadata
        .insert("note".into(), serde_json::json!("changed"));
    assert!(!panel.synchronize(&session, &changed));
    assert!(!panel.has_pending());

    // Explicit removal lists the dependent coincident constraint.
    let dependents = dependents(session.document(), fixture.a.sketch, fixture.a.reference);
    assert_eq!(dependents.constraints, vec![fixture.a.coincident]);
    panel
        .stage_removal(
            &session,
            &resources,
            fixture.a.sketch,
            fixture.a.reference,
            &dependents,
        )
        .unwrap();
    assert_eq!(panel.steps().len(), 2);
    assert!(matches!(
        panel.steps()[0],
        RepairStep::RemoveSketchItems { .. }
    ));
    panel.unstage(1);
    panel.unstage(0);
    assert!(!panel.has_pending());
    panel.discard();
    assert_eq!(session.revision(), 2);
}
