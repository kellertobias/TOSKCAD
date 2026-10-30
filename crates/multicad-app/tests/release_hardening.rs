//! Release hardening on the acceptance scenario models: a compile interrupted
//! at any evaluation boundary returns a supersession error without touching
//! the document, and the next compile reproduces the uninterrupted result;
//! the background model runtime publishes only the latest edit, honors
//! cancellation and never blocks its poller while a native rebuild runs.
//!
//! Incremental-rebuild status is measured, not assumed: the compiler has no
//! cross-call cache, so a single parameter edit re-runs every evaluation
//! step. `incremental_status_*` pins that observable fact so a future
//! incremental engine has to update this test and the release-budget report.
#[path = "../examples/makerspace_review.rs"]
#[allow(dead_code)]
mod makerspace;

#[path = "../examples/tool_case_review.rs"]
#[allow(dead_code)]
mod tool_case;

use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
};
use multicad_model::{Compiler, Model, Runtime, is_superseded};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

/// Output fingerprint per evaluated producer: volume, bounds, mesh size.
fn fingerprints(model: &Model) -> BTreeMap<EntityId, String> {
    model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| {
            (
                *id,
                format!(
                    "{:.6}|{:?}|{:?}|{}|{}",
                    s.volume_mm3,
                    s.bounds.min,
                    s.bounds.max,
                    s.mesh.indices.len(),
                    s.topology.elements.len()
                ),
            )
        })
        .collect()
}
/// Number of interruption checks a full compile performs.
fn checkpoints(document: &Document, revision: u64) -> usize {
    let count = AtomicUsize::new(0);
    let probe = || {
        count.fetch_add(1, Ordering::Relaxed);
        false
    };
    Compiler::default()
        .compile_interruptible(document, revision, Some(&probe))
        .unwrap();
    count.into_inner()
}

/// Interrupt at a spread of boundaries, including the first and the last.
fn interrupt_everywhere(session: &Session) {
    let document = session.document();
    let revision = session.revision();
    let before = document.clone();
    let baseline = fingerprints(&Compiler::default().compile(document, revision).unwrap());
    let total = checkpoints(document, revision);
    assert!(total > 8, "too few evaluation boundaries: {total}");
    let mut stops: Vec<usize> = (1..=total).step_by((total / 16).max(1)).collect();
    stops.push(total);
    for stop in stops {
        let calls = AtomicUsize::new(0);
        let interrupt = || calls.fetch_add(1, Ordering::Relaxed) + 1 >= stop;
        let error = Compiler::default()
            .compile_interruptible(document, revision, Some(&interrupt))
            .err()
            .unwrap_or_else(|| panic!("interrupt at boundary {stop}/{total} was ignored"));
        // Never reported as a modeling error (for example a nested symbol
        // or sketch evaluation blaming its source for the cancellation).
        assert!(is_superseded(&error), "boundary {stop}/{total}: {error}");
        // The request stops at the boundary it was raised on; at most one
        // further probe classifies a nested evaluator's failure.
        let probes = calls.into_inner();
        assert!(
            (stop..=stop + 1).contains(&probes),
            "boundary {stop}/{total}: {probes} probes"
        );
        assert_eq!(document, &before);
        let again = Compiler::default().compile(document, revision).unwrap();
        assert_eq!(
            fingerprints(&again),
            baseline,
            "after interrupt {stop}/{total}"
        );
    }
}

#[test]
fn makerspace_compile_interrupts_at_every_boundary_and_recovers() {
    interrupt_everywhere(&makerspace::build().unwrap().session);
}
#[test]
fn tool_case_compile_interrupts_at_every_boundary_and_recovers() {
    interrupt_everywhere(&tool_case::build().unwrap().session);
}

fn wait<T: Send + 'static>(runtime: &mut Runtime<T>) -> multicad_model::ModelReply<T> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(reply) = runtime.poll() {
            return reply;
        }
        assert!(Instant::now() < deadline, "model worker did not publish");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn runtime_publishes_only_the_latest_edit_and_never_blocks_polling() {
    let mut s = tool_case::build().unwrap();
    let original = s.session.document().clone();
    let original_revision = s.session.revision();
    let started = Instant::now();
    let expected_original = fingerprints(
        &Compiler::default()
            .compile(&original, original_revision)
            .unwrap(),
    );
    let compile_time = started.elapsed();
    tool_case::set_parameter(&mut s.session, s.rail_pitch, "80").unwrap();
    let edited = s.session.document().clone();
    let edited_revision = s.session.revision();
    let expected_edited = fingerprints(
        &Compiler::default()
            .compile(&edited, edited_revision)
            .unwrap(),
    );
    assert_ne!(expected_original, expected_edited);

    let mut runtime = Runtime::new(Compiler::default()).unwrap();
    // An edit arriving while the previous rebuild runs supersedes it.
    runtime.request(&original, original_revision).unwrap();
    let latest = runtime.request(&edited, edited_revision).unwrap();
    let mut slowest_poll = Duration::ZERO;
    let deadline = Instant::now() + Duration::from_secs(120);
    let reply = loop {
        let before = Instant::now();
        let reply = runtime.poll();
        slowest_poll = slowest_poll.max(before.elapsed());
        if let Some(reply) = reply {
            break reply;
        }
        assert!(Instant::now() < deadline, "model worker did not publish");
        thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(reply.generation, latest);
    let snapshot = reply.result.unwrap();
    assert_eq!(snapshot.stamp.revision, edited_revision);
    assert_eq!(snapshot.shapes.len(), expected_edited.len());
    // Polling reads a slot; it never waits for the native rebuild.
    assert!(
        slowest_poll < Duration::from_millis(50),
        "poll blocked for {slowest_poll:?}"
    );
    assert!(runtime.poll().is_none());

    // Cancel supersedes the active job without a publication; the worker
    // stays usable for the next request.
    runtime.request(&original, original_revision).unwrap();
    runtime.cancel();
    thread::sleep(compile_time * 2);
    assert!(runtime.poll().is_none(), "a cancelled rebuild published");
    let next = runtime.request(&original, original_revision).unwrap();
    let reply = wait(&mut runtime);
    assert_eq!(reply.generation, next);
    let snapshot = reply.result.unwrap();
    assert_eq!(snapshot.stamp.revision, original_revision);
    assert_eq!(snapshot.shapes.len(), expected_original.len());
}

#[test]
fn incremental_status_parameter_edit_reevaluates_the_whole_graph() {
    let mut s = makerspace::build().unwrap();
    let before = Compiler::default()
        .compile(s.session.document(), s.session.revision())
        .unwrap();
    let steps_before = checkpoints(s.session.document(), s.session.revision());
    makerspace::set_parameter(&mut s.session, s.room_width, "9600").unwrap();
    let after = Compiler::default()
        .compile(s.session.document(), s.session.revision())
        .unwrap();
    let (before, after) = (fingerprints(&before), fingerprints(&after));
    let changed = after
        .iter()
        .filter(|(id, print)| before.get(id) != Some(print))
        .count();
    // The edit changes only the walls and the floor slab ...
    assert_eq!(changed, 2, "{after:?}");
    assert_eq!(after.len(), 10);
    // ... yet the rebuild performs every evaluation step again.
    assert_eq!(
        checkpoints(s.session.document(), s.session.revision()),
        steps_before
    );
}
