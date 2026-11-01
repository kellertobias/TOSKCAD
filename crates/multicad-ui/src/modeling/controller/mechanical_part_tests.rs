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
    let end = Instant::now() + Duration::from_secs(90);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < end, "native hardware preview timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn all_three_hardware_forms_use_real_native_preview_cancel_apply_history_and_reopen() {
    let recipes = [
        model::MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(0.5),
            frame: Default::default(),
        },
        model::MechanicalPartParameters::HexNut {
            across_flats: length(5.5),
            thread: Box::new(model::ThreadParameters {
                major_diameter: length(3.),
                pitch: length(0.5),
                length: length(2.4),
                ..Default::default()
            }),
        },
        model::MechanicalPartParameters::BearingEnvelope {
            bore_diameter: length(10.),
            outside_diameter: length(30.),
            width: length(9.),
            frame: Default::default(),
        },
    ];
    for parameters in recipes {
        let mut s = Session::new(Document::new("Native hardware UI").unwrap()).unwrap();
        let before = s.document().clone();
        let r = model::MechanicalPartRequest::new("Hardware", parameters.clone());
        let mut c = controller();
        for cancel in [true, false] {
            let panel = MechanicalPartPanel::new(r.clone(), s.parameters().clone(), false).unwrap();
            c.begin_form(
                &s,
                &Default::default(),
                Channel::Ui,
                ModelForm::MechanicalPart(Box::new(panel)),
            )
            .unwrap();
            assert_eq!(c.label(), "Hardware");
            assert!(!c.can_apply(&s, &Default::default()));
            c.preview(&s, &Default::default(), Channel::Ui).unwrap();
            wait(&mut c, &s);
            assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
            assert_eq!(s.document(), &before);
            let candidate = c.candidate.as_ref().unwrap();
            assert!(candidate.requires_info);
            assert!(
                candidate
                    .scene
                    .as_ref()
                    .unwrap()
                    .model_evaluation()
                    .is_some()
            );
            if cancel {
                c.cancel();
                assert_eq!(s.document(), &before);
            } else {
                let outputs = c.apply(&mut s, &Default::default()).unwrap();
                assert_eq!(outputs.len(), 1);
                assert_eq!(outputs[0].feature, r.feature);
                assert_eq!(outputs[0].body, r.body);
                let source = model::read_mechanical_part(s.document(), r.feature).unwrap();
                assert_eq!(source.parameters, parameters);
                let saved = s.document().clone();
                s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
                assert_eq!(s.document(), &before);
                s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
                assert_eq!(s.document(), &saved);
                assert!(ModelToolController::editable(s.document(), r.feature));
                c.begin_existing(&s, &Default::default(), r.feature, Channel::Ui)
                    .unwrap();
                assert_eq!(c.label(), "Feature edit");
                c.preview(&s, &Default::default(), Channel::Ui).unwrap();
                wait(&mut c, &s);
                assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
                c.cancel();
                assert_eq!(s.document(), &saved);
            }
        }
    }
}
#[test]
fn below_native_resolution_hardware_never_enables_apply_or_mutates_source() {
    let mut s = Session::new(Document::new("Invalid hardware UI").unwrap()).unwrap();
    let before = s.document().clone();
    let r = model::MechanicalPartRequest::new(
        "Too thin",
        model::MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(1e-10),
            frame: Default::default(),
        },
    );
    let panel = MechanicalPartPanel::new(r, s.parameters().clone(), false).unwrap();
    let mut c = controller();
    c.begin_form(
        &s,
        &Default::default(),
        Channel::Ui,
        ModelForm::MechanicalPart(Box::new(panel)),
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
