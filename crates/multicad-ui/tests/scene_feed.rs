use multicad_core::document::Document;
use multicad_ui::viewport::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct State {
    requests: Vec<(SceneStamp, String)>,
    replies: VecDeque<SceneReply>,
    reject: bool,
    refinements: Vec<(SceneStamp, DisplayQuality)>,
    refinement: Option<Refinement>,
}
struct Backend(Arc<Mutex<State>>);
impl SceneBackend for Backend {
    fn request(&mut self, doc: &Document, stamp: SceneStamp) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.requests.push((stamp, doc.name().into()));
        if state.reject {
            Err("Invalid source".into())
        } else {
            Ok(())
        }
    }
    fn poll(&mut self) -> Option<SceneReply> {
        self.0.lock().unwrap().replies.pop_front()
    }
    fn refine(&mut self, stamp: SceneStamp, quality: DisplayQuality) -> Result<Refinement, String> {
        let mut state = self.0.lock().unwrap();
        state.refinements.push((stamp, quality));
        Ok(state.refinement.unwrap_or(Refinement::Unsupported))
    }
}
#[test]
fn refinement_is_debounced_keeps_source_usable_and_rejects_obsolete_replies() {
    let state = Arc::new(Mutex::new(State {
        refinement: Some(Refinement::Started),
        ..Default::default()
    }));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let doc = Document::new("Model").unwrap();
    feed.tick_at(&doc, 0, 1.);
    let first = state.lock().unwrap().requests[0].0;
    reply(&state, first);
    feed.tick_at(&doc, 0, 1.);
    let original = feed.visible().unwrap().clone();
    feed.note_quality(DisplayQuality::Normal, 1.);
    feed.note_quality(DisplayQuality::Fine, 1.1);
    feed.tick_at(&doc, 0, 1.24);
    assert!(state.lock().unwrap().refinements.is_empty());
    assert!(feed.quality_wakeup(1.24).unwrap().as_secs_f64() < 0.011);
    assert!(feed.quality_wakeup(f64::NAN).is_none());
    feed.tick_at(&doc, 0, 1.26);
    let refining = state.lock().unwrap().refinements[0].0;
    assert!(feed.refining());
    assert!(feed.is_current(doc.id(), 0));
    assert!(Arc::ptr_eq(&original, feed.visible().unwrap()));
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: refining,
        result: Err("Display budget exceeded".into()),
    });
    feed.tick_at(&doc, 0, 1.3);
    assert_eq!(feed.quality_error(), Some("Display budget exceeded"));
    assert!(feed.error().is_none());
    assert!(feed.is_current(doc.id(), 0));
    assert!(feed.quality_wakeup(2.).is_none()); // No error retry loop.
    feed.retry_quality();
    feed.tick_at(&doc, 0, 2.);
    let retry = state.lock().unwrap().refinements[1].0;
    assert!(retry.generation > refining.generation);
    feed.tick_at(&doc, 1, 2.1);
    let source = state.lock().unwrap().requests[1].0;
    assert!(!feed.is_current(doc.id(), 1));
    reply(&state, retry);
    feed.tick_at(&doc, 1, 2.2);
    assert!(feed.pending());
    assert_eq!(feed.visible().unwrap().stamp(), original.stamp());
    reply(&state, source);
    feed.tick_at(&doc, 1, 2.3);
    assert!(feed.is_current(doc.id(), 1));
    feed.tick_at(&doc, 1, 2.4);
    let refined = state.lock().unwrap().refinements[2].0;
    reply(&state, refined);
    feed.tick_at(&doc, 1, 2.5);
    assert_eq!(feed.visible().unwrap().stamp(), refined);
    feed.note_quality(DisplayQuality::Coarse, 3.);
    feed.tick_at(&doc, 1, 3.59);
    assert_eq!(state.lock().unwrap().refinements.len(), 3);
    feed.tick_at(&doc, 1, 3.61);
    assert_eq!(state.lock().unwrap().refinements.len(), 4);
}
#[test]
fn unsupported_quality_provider_does_not_keep_idle_window_awake() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let doc = Document::new("Model").unwrap();
    feed.tick_at(&doc, 0, 1.);
    let stamp = state.lock().unwrap().requests[0].0;
    reply(&state, stamp);
    feed.tick_at(&doc, 0, 1.);
    feed.note_quality(DisplayQuality::Fine, 1.);
    feed.tick_at(&doc, 0, 2.);
    feed.note_quality(DisplayQuality::Precision, 2.);
    feed.tick_at(&doc, 0, 3.);
    assert_eq!(state.lock().unwrap().refinements.len(), 1);
    assert!(feed.quality_wakeup(3.).is_none());
    assert!(feed.is_current(doc.id(), 0));
    assert!(!feed.pending());
}
fn reply(state: &Arc<Mutex<State>>, stamp: SceneStamp) {
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp,
        result: Ok(Arc::new(Scene::new(stamp, vec![], vec![]).unwrap())),
    });
}
#[test]
fn each_source_revision_requests_once_and_obsolete_success_or_failure_cannot_publish() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let mut doc = Document::new("One").unwrap();
    feed.tick(&doc, 0);
    feed.tick(&doc, 0);
    assert!(feed.pending());
    assert_eq!(state.lock().unwrap().requests.len(), 1);
    let first = state.lock().unwrap().requests[0].0;
    reply(&state, first);
    feed.tick(&doc, 0);
    assert!(feed.is_current(doc.id(), 0));
    let old = feed.visible().unwrap().clone();
    doc.rename("Two").unwrap();
    feed.tick(&doc, 1);
    let second = state.lock().unwrap().requests[1].0;
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    assert!(!feed.is_current(doc.id(), 1));
    doc.rename("Three").unwrap();
    reply(&state, second);
    feed.tick(&doc, 2);
    let third = state.lock().unwrap().requests[2].0;
    assert!(feed.pending());
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: second,
        result: Err("Obsolete failure".into()),
    });
    feed.tick(&doc, 2);
    assert!(feed.error().is_none());
    assert!(feed.pending());
    reply(&state, third);
    feed.tick(&doc, 2);
    assert!(feed.is_current(doc.id(), 2));
    assert_eq!(state.lock().unwrap().requests[2].1, "Three");
}
#[test]
fn failed_or_malformed_replies_keep_visible_geometry_and_retry_changes_generation() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let doc = Document::new("Model").unwrap();
    feed.tick(&doc, 0);
    let first = state.lock().unwrap().requests[0].0;
    reply(&state, first);
    feed.tick(&doc, 0);
    let old = feed.visible().unwrap().clone();
    feed.tick(&doc, 1);
    let second = state.lock().unwrap().requests[1].0;
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: second,
        result: Err("Boolean failed".into()),
    });
    feed.tick(&doc, 1);
    assert_eq!(feed.error(), Some("Boolean failed"));
    assert!(!feed.pending());
    assert!(!feed.is_current(doc.id(), 1));
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    feed.tick(&doc, 1);
    assert_eq!(state.lock().unwrap().requests.len(), 2);
    feed.retry();
    feed.tick(&doc, 1);
    let third = state.lock().unwrap().requests[2].0;
    assert!(third.generation > second.generation);
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: third,
        result: Ok(old.clone()),
    });
    feed.tick(&doc, 1);
    assert!(feed.error().unwrap().contains("mismatched"));
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    feed.retry();
    feed.tick(&doc, 1);
    let fourth = state.lock().unwrap().requests[3].0;
    reply(&state, fourth);
    feed.tick(&doc, 1);
    assert!(feed.is_current(doc.id(), 1));
    state.lock().unwrap().reject = true;
    feed.tick(&doc, 2);
    assert_eq!(feed.error(), Some("Invalid source"));
    assert!(!feed.pending());
    assert_eq!(feed.visible().unwrap().stamp(), fourth);
}
