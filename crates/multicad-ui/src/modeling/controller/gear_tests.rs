use super::*;
use crate::modeling::native_fixture;
use multicad_core::{document::Document, model};
use std::time::{Duration, Instant};
fn controller() -> ModelToolController {
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c
}
fn wait(c: &mut ModelToolController, s: &Session) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < deadline, "native gear preview timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn native_gear_preview_cancel_apply_history_and_historical_edit_use_the_real_gate() {
    let mut s = Session::new(Document::new("Native gear UI").unwrap()).unwrap();
    let before = s.document().clone();
    let mut c = controller();
    for cancel in [true, false] {
        c.begin_gear(&s, &Default::default(), Channel::Ui).unwrap();
        assert_eq!(c.label(), "Gear");
        assert!(!c.can_apply(&s, &Default::default()));
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        if cancel {
            c.cancel();
        } else {
            let output = c.apply(&mut s, &Default::default()).unwrap();
            let feature = output[0].feature;
            assert_eq!(
                model::read_gear(s.document(), feature).unwrap().output.body,
                output[0].body
            );
            let committed = s.document().clone();
            s.undo(1, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &before);
            s.redo(2, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &committed);
            assert!(ModelToolController::editable(s.document(), feature));
            c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
                .unwrap();
            c.cancel();
            assert_eq!(s.document(), &committed);
        }
    }
}
#[test]
fn native_unresolvable_width_never_enables_apply_or_changes_the_document() {
    let mut s = Session::new(Document::new("Invalid native gear").unwrap()).unwrap();
    let request = model::GearRequest::new(
        "Too thin",
        model::GearParameters {
            face_width: model::length(0.00000001),
            ..Default::default()
        },
    );
    let panel = GearPanel::new(request, s.parameters().clone(), false).unwrap();
    let before = s.document().clone();
    let mut c = controller();
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::Gear(Box::new(panel)),
    )
    .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(!c.can_apply(&s, &Default::default()));
    assert!(c.error().is_some());
    assert!(c.apply(&mut s, &Default::default()).is_err());
    c.cancel();
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), 0);
}
