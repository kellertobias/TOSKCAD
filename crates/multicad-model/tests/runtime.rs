use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, BodyInput, CutTargetProbe, CutTargetQuery, EvaluationOptions, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, length,
    },
};
use multicad_model::{
    Compiler, EvaluationScope, GeometryBackend, ModelReply, Runtime, ShapeSnapshot,
};
use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
fn reply<T: Send + 'static>(runtime: &mut Runtime<T>) -> ModelReply<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = runtime.poll() {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "Model worker did not publish a reply"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn rust_scoped_preview_rebuilds_the_edited_feature_and_apply_reuses_both() {
    let actor = Actor::local(Channel::Ui);
    let mut session = Session::new(Document::new("Incremental Rust edit").unwrap()).unwrap();
    let box_parameters = |x, width| PrimitiveParameters::Box {
        size: [length(width), length(5.), length(6.)],
        frame: PrimitiveFrame {
            origin: [length(x), length(0.), length(0.)],
            ..Default::default()
        },
    };
    let edited = PrimitiveRequest::new("Edited box", box_parameters(0., 4.));
    let unchanged = PrimitiveRequest::new("Unchanged box", box_parameters(40., 4.));
    for request in [&edited, &unchanged] {
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor.clone(), request).unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let mut canonical = Runtime::with_model_and_snapshot_backend(
        Compiler::default(),
        GeometryBackend::Rust,
        |_, model, snapshot, _| {
            assert!(
                snapshot
                    .shapes
                    .keys()
                    .all(|id| model.solid(*id).unwrap().rust_prism().is_some())
            );
            Ok(Arc::new(snapshot.clone()))
        },
    )
    .unwrap();
    let mut preview = canonical.fork().unwrap();
    let base_revision = session.revision();
    canonical
        .request(session.document(), base_revision)
        .unwrap();
    let original = reply(&mut canonical).result.unwrap();
    let initial_stats = canonical.last_compile_stats().unwrap();
    assert_eq!(
        initial_stats.evaluated,
        [edited.feature, unchanged.feature].into()
    );
    assert!(initial_stats.reused.is_empty());
    let edit = model::prepare_primitive_edit(
        &session,
        base_revision,
        actor,
        edited.feature,
        &box_parameters(0., 8.),
    )
    .unwrap();
    preview
        .request_sources(
            edit.prepared.document(),
            base_revision,
            0,
            &[edited.feature],
        )
        .unwrap();
    let checked = reply(&mut preview).result.unwrap();
    let preview_stats = preview.last_compile_stats().unwrap();
    assert_eq!(preview_stats.evaluated, [edited.feature].into());
    assert!(preview_stats.reused.is_empty());
    assert_eq!(
        checked.scope,
        EvaluationScope::Sources(vec![edited.feature])
    );
    assert_eq!(checked.shapes[&edited.feature].volume_mm3, 240.);
    assert_eq!(original.shapes[&edited.feature].volume_mm3, 120.);
    assert_ne!(checked.stamp.source_hash, original.stamp.source_hash);
    assert_eq!(checked.stamp.revision, base_revision);
    assert!(canonical.poll().is_none());

    session.commit(edit.prepared).unwrap();
    canonical
        .request(session.document(), session.revision())
        .unwrap();
    let applied = reply(&mut canonical).result.unwrap();
    let stats = canonical.last_compile_stats().unwrap();
    assert_eq!(stats.reused, [edited.feature, unchanged.feature].into());
    assert!(stats.evaluated.is_empty());
    assert_eq!(applied.scope, EvaluationScope::Full);
    assert_eq!(applied.stamp.source_hash, checked.stamp.source_hash);
    assert_eq!(applied.stamp.revision, session.revision());
    assert_eq!(
        applied.shapes[&edited.feature].topology.revision,
        session.revision()
    );
    assert_eq!(
        applied.shapes[&unchanged.feature].topology.revision,
        session.revision()
    );
    assert_eq!(applied.shapes[&edited.feature].volume_mm3, 240.);
    assert_eq!(applied.shapes[&unchanged.feature].volume_mm3, 120.);
    for (before, after) in [
        (
            &checked.shapes[&edited.feature],
            &applied.shapes[&edited.feature],
        ),
        (
            &original.shapes[&unchanged.feature],
            &applied.shapes[&unchanged.feature],
        ),
    ] {
        assert!(Arc::ptr_eq(&before.mesh, &after.mesh));
        assert!(Arc::ptr_eq(&before.edges, &after.edges));
        assert!(Arc::ptr_eq(&before.vertices, &after.vertices));
    }
    assert_eq!(
        original.shapes[&unchanged.feature].topology.revision,
        base_revision
    );
}

#[test]
fn drawing_page_tags_reuse_the_owned_model_but_same_revision_changed_source_rebuilds() {
    let mut session = Session::new(Document::new("Page model reuse").unwrap()).unwrap();
    let primitive = PrimitiveRequest::new(
        "Panel",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &primitive).unwrap();
    session.commit(prepared.prepared).unwrap();
    let source = primitive.body;
    let mut runtime = Runtime::with_cached_model_publisher(
        Compiler::default(),
        |_, _, _| None,
        move |_, model, tag| {
            // Observe ownership without exporting or dereferencing a native
            // handle. A fresh compile must allocate its own live solid map.
            Ok((
                model.solid(source).unwrap() as *const _ as usize,
                model.snapshot().shapes.len(),
                tag,
            ))
        },
    )
    .unwrap();
    runtime.request_tagged(session.document(), 1, 1).unwrap();
    let first = reply(&mut runtime).result.unwrap();
    runtime.request_tagged(session.document(), 1, 2).unwrap();
    let second = reply(&mut runtime).result.unwrap();
    assert_eq!(first.0, second.0);
    assert_eq!(second.2, 2);
    let additional = PrimitiveRequest::new(
        "Another panel",
        PrimitiveParameters::Box {
            size: std::array::from_fn(|_| length(5.)),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &additional,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    // Revisions alone cannot authorize reuse (drafts share a base revision).
    runtime.request_tagged(session.document(), 1, 3).unwrap();
    let changed = reply(&mut runtime).result.unwrap();
    assert_ne!(first.0, changed.0);
    assert_eq!(changed.1, first.1 + 1);
    assert_eq!(changed.2, 3);
}

#[test]
fn publisher_panic_delivers_a_terminal_failure_and_refuses_future_requests() {
    let mut session = Session::new(Document::new("Publisher failure").unwrap()).unwrap();
    let primitive = PrimitiveRequest::new(
        "Retained native box",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::Mcp), &primitive).unwrap();
    session.commit(prepared.prepared).unwrap();
    let retained = Arc::new(Mutex::new(None::<Weak<ShapeSnapshot>>));
    let observed = Arc::clone(&retained);
    let mut runtime = Runtime::with_publisher(Compiler::default(), move |_, model, tag| {
        if tag == 1 {
            panic!("Controlled publisher panic");
        }
        *observed.lock().unwrap() = Some(Arc::downgrade(model.shapes.values().next().unwrap()));
        Ok(())
    })
    .unwrap();
    let document = session.document();
    runtime.request_tagged(document, 1, 0).unwrap();
    reply(&mut runtime).result.unwrap();
    assert!(
        retained
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .is_some()
    );
    let generation = runtime.request_tagged(document, 1, 1).unwrap();
    let terminal = reply(&mut runtime);
    assert_eq!(terminal.generation, generation);
    let error = terminal.result.unwrap_err();
    assert!(error.message.contains("worker exited unexpectedly"));
    assert!(
        retained
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .is_none(),
        "Retained native model outlived its terminal failure publication"
    );
    assert!(
        runtime.poll().is_none(),
        "Terminal failure was delivered repeatedly"
    );
    assert!(
        runtime.request(document, 2).is_err(),
        "A failed native worker accepted another operation"
    );
}

#[test]
fn stale_active_panic_fails_the_latest_queued_generation_without_retrying_native_work() {
    let (started, started_rx) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::sync_channel(1);
    let publications = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&publications);
    let mut runtime = Runtime::with_publisher(Compiler::default(), move |_, _, tag| {
        observed.fetch_add(1, Ordering::SeqCst);
        if tag == 1 {
            started.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            panic!("Controlled stale publisher panic");
        }
        Ok(())
    })
    .unwrap();
    let first = Document::new("Failing active source").unwrap();
    let obsolete = runtime.request_tagged(&first, 0, 1).unwrap();
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    runtime.request_tagged(&first, 1, 2).unwrap();
    let latest_source = Document::new("Latest pending source").unwrap();
    let latest = runtime.request_tagged(&latest_source, 2, 3).unwrap();
    assert_ne!(latest, obsolete);
    release.send(()).unwrap();
    let terminal = reply(&mut runtime);
    assert_eq!(terminal.generation, latest);
    assert!(
        terminal
            .result
            .unwrap_err()
            .message
            .contains("worker exited unexpectedly")
    );
    assert_eq!(
        publications.load(Ordering::SeqCst),
        1,
        "Queued native work was retried after a panic"
    );
    assert!(runtime.poll().is_none());
    assert!(runtime.request(&latest_source, 3).is_err());
}

#[test]
fn superseded_active_and_queued_jobs_never_publish_as_current() {
    let (started, started_rx) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::sync_channel(1);
    let mut runtime = Runtime::with_publisher(Compiler::default(), move |_, model, tag| {
        if tag == 1 {
            started.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        Ok((model.stamp, tag))
    })
    .unwrap();
    let first = Document::new("First source").unwrap();
    runtime.request_tagged(&first, 0, 1).unwrap();
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let final_source = Document::new("Latest source").unwrap();
    for revision in 1..100 {
        runtime.request_tagged(&first, revision, 2).unwrap();
    }
    let latest = runtime.request_tagged(&final_source, 100, 3).unwrap();
    assert!(runtime.poll().is_none());
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let reply = loop {
        if let Some(reply) = runtime.poll() {
            break reply;
        }
        assert!(Instant::now() < deadline, "Model worker did not publish");
        thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(reply.generation, latest);
    let (stamp, tag) = reply.result.unwrap();
    assert_eq!(stamp.document, final_source.id());
    assert_eq!(stamp.revision, 100);
    assert_eq!(tag, 3);
    assert!(runtime.poll().is_none());
}

#[test]
fn shutdown_does_not_join_or_block_on_an_active_native_publication() {
    let (started, started_rx) = mpsc::sync_channel(1);
    let (release, release_rx) = mpsc::sync_channel(1);
    let (finished, finished_rx) = mpsc::sync_channel(1);
    let mut runtime = Runtime::with_publisher(Compiler::default(), move |_, _, _| {
        started.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        finished.send(()).unwrap();
        Ok(())
    })
    .unwrap();
    runtime
        .request(&Document::new("Close while working").unwrap(), 0)
        .unwrap();
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    // Drop must return before releasing the deliberately blocked callback.
    drop(runtime);
    release.send(()).unwrap();
    finished_rx.recv_timeout(Duration::from_secs(10)).unwrap();
}

fn probe_fixture() -> (Session, Vec<BodyInput>, CutTargetQuery) {
    let mut session = Session::new(Document::new("Runtime query fixture").unwrap()).unwrap();
    let mut bodies = vec![];
    for (name, origin, size) in [
        ("Target A", [0.; 3], [10.; 3]),
        ("Target B", [0.; 3], [10.; 3]),
        ("Tool", [4., 4., -1.], [2., 2., 12.]),
    ] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: size.map(length),
                frame: PrimitiveFrame {
                    origin: origin.map(length),
                    ..Default::default()
                },
            },
        );
        let p = model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::Mcp),
            &request,
        )
        .unwrap();
        session.commit(p.prepared).unwrap();
        bodies.push(BodyInput {
            body: request.body,
            producer: request.feature,
        });
    }
    let query = CutTargetQuery {
        tools: vec![bodies[2].producer],
        component: None,
        include: vec![],
        exclude: vec![],
    };
    (session, bodies, query)
}
#[test]
fn same_document_revision_tag_and_probe_id_still_publish_only_latest_owned_options() {
    let (session, bodies, query) = probe_fixture();
    let before = session.document().clone();
    let (first_entered, entered) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let publications = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&publications);
    let mut runtime = Runtime::with_publisher(Compiler::default(), move |_, snapshot, tag| {
        assert_eq!(tag, 77);
        if observed.fetch_add(1, Ordering::SeqCst) == 0 {
            first_entered
                .send((snapshot.stamp, snapshot.cut_targets.clone()))
                .unwrap();
            released.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        Ok(Arc::new(snapshot.clone()))
    })
    .unwrap();
    let id = EntityId::new();
    let mut first_options = EvaluationOptions {
        cut_targets: vec![CutTargetProbe { id, query }],
        ..Default::default()
    };
    first_options.cut_targets[0].query.exclude = vec![bodies[0].body];
    let first = runtime
        .request_evaluation(&before, session.revision(), 77, &first_options)
        .unwrap();
    first_options.cut_targets[0].query.exclude = vec![bodies[1].body];
    let (stamp, old) = entered.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        old[0].result.as_ref().unwrap().targets,
        vec![bodies[1].clone()]
    );
    let mut latest_options = first_options.clone();
    let latest = runtime
        .request_evaluation(&before, session.revision(), 77, &latest_options)
        .unwrap();
    assert!(latest > first);
    latest_options.cut_targets[0].query.exclude = vec![bodies[0].body];
    assert!(runtime.poll().is_none());
    release.send(()).unwrap();
    let current = reply(&mut runtime);
    assert_eq!(current.generation, latest);
    let snapshot = current.result.unwrap();
    assert_eq!(snapshot.stamp, stamp);
    assert_eq!(snapshot.cut_targets[0].id, id);
    assert_eq!(
        snapshot.cut_targets[0].result.as_ref().unwrap().targets,
        vec![bodies[0].clone()]
    );
    assert!(runtime.poll().is_none());
    assert_eq!(session.document(), &before);
    let next = runtime
        .request_tagged(&before, session.revision(), 77)
        .unwrap();
    let empty = reply(&mut runtime);
    assert_eq!(empty.generation, next);
    assert!(empty.result.unwrap().cut_targets.is_empty());
}
#[test]
fn query_errors_are_correlated_receipts_and_do_not_replace_valid_base_geometry() {
    let (session, bodies, query) = probe_fixture();
    let before = session.document().clone();
    let valid = EntityId::new();
    let invalid = EntityId::new();
    let bad = EntityId::new();
    let options = EvaluationOptions {
        cut_targets: vec![
            CutTargetProbe {
                id: invalid,
                query: CutTargetQuery {
                    tools: vec![bad],
                    ..query.clone()
                },
            },
            CutTargetProbe { id: valid, query },
        ],
        ..Default::default()
    };
    let mut runtime = Runtime::new(Compiler::default()).unwrap();
    let generation = runtime
        .request_evaluation(&before, session.revision(), 42, &options)
        .unwrap();
    let result = reply(&mut runtime);
    assert_eq!(result.generation, generation);
    let snapshot = result.result.unwrap();
    assert_eq!(snapshot.bodies.len(), 3);
    assert_eq!(snapshot.shapes.len(), 3);
    assert_eq!(snapshot.cut_targets.len(), 2);
    assert_eq!(snapshot.cut_targets[0].id, invalid);
    let error = snapshot.cut_targets[0].result.as_ref().unwrap_err();
    assert_eq!(error.entity, Some(bad));
    assert!(error.message.contains("current unconsumed"));
    assert_eq!(snapshot.cut_targets[1].id, valid);
    let targets = &snapshot.cut_targets[1].result.as_ref().unwrap().targets;
    assert!(targets.contains(&bodies[0]));
    assert!(targets.contains(&bodies[1]));
    assert_eq!(session.document(), &before);
    assert!(
        Compiler::default()
            .compile(&before, session.revision())
            .unwrap()
            .snapshot()
            .cut_targets
            .is_empty()
    );
}
#[test]
fn invalid_options_do_not_supersede_pending_jobs_or_advance_private_generation() {
    let (session, _, query) = probe_fixture();
    let (first_entered, entered) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let published = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&published);
    let mut runtime = Runtime::with_publisher(Compiler::default(), move |_, snapshot, _| {
        if observed.fetch_add(1, Ordering::SeqCst) == 0 {
            first_entered.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        Ok(Arc::new(snapshot.clone()))
    })
    .unwrap();
    let probe = CutTargetProbe {
        id: EntityId::new(),
        query,
    };
    let options = EvaluationOptions {
        cut_targets: vec![probe.clone()],
        ..Default::default()
    };
    let first = runtime
        .request_evaluation(session.document(), session.revision(), 1, &options)
        .unwrap();
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    let nil = EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap();
    let invalid = [
        EvaluationOptions {
            cut_targets: vec![probe.clone(), probe.clone()],
            ..Default::default()
        },
        EvaluationOptions {
            cut_targets: vec![CutTargetProbe {
                id: nil,
                query: probe.query.clone(),
            }],
            ..Default::default()
        },
        EvaluationOptions {
            cut_targets: (0..9)
                .map(|_| CutTargetProbe {
                    id: EntityId::new(),
                    query: probe.query.clone(),
                })
                .collect(),
            ..Default::default()
        },
    ];
    for options in invalid {
        assert!(
            runtime
                .request_evaluation(session.document(), session.revision(), 2, &options)
                .is_err()
        );
    }
    release.send(()).unwrap();
    let kept = reply(&mut runtime);
    assert_eq!(kept.generation, first);
    assert_eq!(kept.result.unwrap().cut_targets[0].id, probe.id);
    let next = runtime
        .request(session.document(), session.revision())
        .unwrap();
    assert_eq!(next, first + 1);
    assert!(reply(&mut runtime).result.unwrap().cut_targets.is_empty());
}
