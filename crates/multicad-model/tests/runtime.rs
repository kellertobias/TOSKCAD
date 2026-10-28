use multicad_core::document::Document;
use multicad_model::{Compiler, Runtime};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

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
