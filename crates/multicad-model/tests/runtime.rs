use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
};
use multicad_model::{Compiler, ModelReply, Runtime, ShapeSnapshot};
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
