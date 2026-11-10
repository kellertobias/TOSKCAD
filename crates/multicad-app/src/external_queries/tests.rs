use super::*;
use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::OccurrencePath,
    references::{GeometryIntent, Origin, TopologyKind},
    sketch_external::{OccurrenceSource, QueryMethod},
};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

fn source() -> Session {
    Session::new(Document::new("Worker ownership fixture").unwrap()).unwrap()
}
fn fixture_query() -> ExternalQuery {
    let component = EntityId::new();
    ExternalQuery {
        reference: EntityId::new(),
        sketch: EntityId::new(),
        source: OccurrenceSource {
            component,
            source: EntityId::new(),
            path: OccurrencePath {
                root: component,
                members: vec![],
            },
        },
        sketch_context: None,
        intent: GeometryIntent {
            id: EntityId::new(),
            kind: TopologyKind::Edge,
            origins: BTreeSet::from([Origin {
                source: EntityId::new(),
                role: "fixture.edge".into(),
            }]),
            boundary: None,
            attachment: None,
        },
        method: QueryMethod::Intersect,
    }
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "Worker did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn reply(backend: &mut Backend) -> ExternalQueryReply {
    let mut result = None;
    wait(|| {
        result = backend.poll();
        result.is_some()
    });
    result.unwrap()
}

#[test]
fn held_native_job_keeps_only_latest_queue_and_never_publishes_cancelled_source() {
    let entered = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let visited = Arc::new(Mutex::new(vec![]));
    let (seen, release, visits) = (entered.clone(), released.clone(), visited.clone());
    let worker = Worker::with_processor(move || {
        Box::new(move |job, _| {
            visits.lock().unwrap().push(job.header.request);
            if job.header.request == 1 {
                seen.store(true, Ordering::Release);
                wait(|| release.load(Ordering::Acquire));
            }
            job.header
                .failure(format!("Processed {}", job.header.request))
        })
    })
    .unwrap();
    let mut backend = Backend {
        worker: Some(worker),
        ..Backend::new()
    };
    let session = source();
    let canonical = session.document().clone();
    let first = fixture_query();
    backend
        .request_catalog(session.preparation_snapshot(), Default::default(), first)
        .unwrap();
    wait(|| entered.load(Ordering::Acquire));
    backend
        .request_catalog(
            session.preparation_snapshot(),
            Default::default(),
            fixture_query(),
        )
        .unwrap();
    backend.cancel();
    let last = fixture_query();
    assert_eq!(
        backend
            .request_catalog(
                session.preparation_snapshot(),
                Default::default(),
                last.clone()
            )
            .unwrap(),
        3
    );
    released.store(true, Ordering::Release);
    let ExternalQueryReply::Catalog {
        request,
        query,
        result,
    } = reply(&mut backend)
    else {
        panic!("catalogue reply")
    };
    assert_eq!(request, 3);
    assert_eq!(query, last);
    assert_eq!(result.err().as_deref(), Some("Processed 3"));
    assert_eq!(*visited.lock().unwrap(), vec![1, 3]);
    assert!(backend.poll().is_none());
    assert_eq!(session.document(), &canonical);
    assert_eq!(session.revision(), 0);
}

#[test]
fn worker_exit_reports_latest_request_and_allows_an_independent_retry() {
    let worker =
        Worker::with_processor(|| Box::new(|_, _| panic!("Controlled query failure"))).unwrap();
    let mut backend = Backend {
        worker: Some(worker),
        ..Backend::new()
    };
    let session = source();
    let input = fixture_query();
    backend
        .request_catalog(
            session.preparation_snapshot(),
            Default::default(),
            input.clone(),
        )
        .unwrap();
    let ExternalQueryReply::Catalog {
        request,
        query,
        result,
    } = reply(&mut backend)
    else {
        panic!("catalogue reply")
    };
    assert_eq!(request, 1);
    assert_eq!(query, input);
    assert!(result.err().unwrap().contains("worker exited"));
    let old = backend.worker.as_ref().unwrap().0.clone();
    let input = fixture_query();
    assert_eq!(
        backend
            .request_catalog(
                session.preparation_snapshot(),
                Default::default(),
                input.clone()
            )
            .unwrap(),
        2
    );
    assert!(!Arc::ptr_eq(&old, &backend.worker.as_ref().unwrap().0));
    let ExternalQueryReply::Catalog {
        request,
        query,
        result,
    } = reply(&mut backend)
    else {
        panic!("catalogue reply")
    };
    assert_eq!(request, 2);
    assert_eq!(query, input);
    // The real provider diagnoses this deliberately missing model source.
    assert!(result.is_err());
    assert_eq!(session.revision(), 0);
}
