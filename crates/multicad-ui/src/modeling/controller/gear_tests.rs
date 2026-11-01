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
    let deadline = Instant::now() + Duration::from_secs(180);
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

#[test]
fn native_pair_preview_requires_both_placed_members_and_preserves_atomic_history() {
    let mut s = Session::new(Document::new("Native pair UI").unwrap()).unwrap();
    let before = s.document().clone();
    let request = model::GearRequest::new(
        "Pair",
        model::GearParameters {
            face_width: model::length(4.),
            ..Default::default()
        },
    );
    let mut panel = GearPanel::new(request, s.parameters().clone(), false).unwrap();
    panel.enable_pair();
    let ModelRequest::GearPair(command) = panel.model_request(s.document(), true).unwrap() else {
        panic!("pair conversion must use the pair transaction");
    };
    let pair = command.pair();
    let root = command.occurrence().unwrap();
    let mut c = controller();
    for cancel in [true, false] {
        c.begin_form(
            &s,
            &Default::default(),
            Channel::Ui,
            ModelForm::Gear(Box::new(panel.clone())),
        )
        .unwrap();
        assert_eq!(c.label(), "Gear pair");
        c.preview(&s, &Default::default(), Channel::Ui).unwrap();
        wait(&mut c, &s);
        assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
        assert_eq!(s.document(), &before);
        let candidate = c.candidate.as_ref().unwrap();
        let scene = candidate.scene.as_ref().unwrap();
        let source = candidate.prepared.document();
        let leaves = scene.assembly_evaluation().unwrap();
        assert_eq!(
            leaves.leaves.iter().filter(|l| l.path.root == root).count(),
            2
        );
        let aliases = candidate
            .outputs
            .iter()
            .map(|o| (o.body, o.feature))
            .collect();
        let missing_placement = Scene::new(
            scene.stamp(),
            scene.geometries().values().cloned().collect(),
            scene.reference_instances().to_vec(),
        )
        .unwrap()
        .with_reference_aliases(aliases)
        .unwrap()
        .with_model_evaluation(scene.model_evaluation().unwrap().clone())
        .unwrap();
        assert!(
            candidate
                .proof
                .validate(source, &missing_placement, &candidate.outputs)
                .unwrap_err()
                .contains("physical occurrence evidence")
        );
        let duplicated = [candidate.outputs[0].clone(), candidate.outputs[0].clone()];
        assert!(
            candidate
                .proof
                .validate(source, scene, &duplicated)
                .unwrap_err()
                .contains("member identities")
        );
        if cancel {
            c.cancel();
        } else {
            let output = c.apply(&mut s, &Default::default()).unwrap();
            assert_eq!(output.len(), 2);
            let committed = s.document().clone();
            let definition = model::read_gear_pair(s.document(), pair).unwrap();
            for selected in [
                root,
                pair,
                definition.outputs.assembly,
                definition.outputs.instances[0],
                definition.outputs.joints[1],
                output[0].body,
                output[1].feature,
            ] {
                assert_eq!(
                    ModelToolController::gear_pair_for(s.document(), selected),
                    Some(pair)
                );
            }
            s.undo(1, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &before);
            s.redo(2, Actor::local(Channel::Ui)).unwrap();
            assert_eq!(s.document(), &committed);
            c.begin_existing(&s, &Default::default(), pair, Channel::Ui)
                .unwrap();
            assert_eq!(c.label(), "Gear pair");
            c.cancel();
            assert_eq!(s.document(), &committed);
        }
    }
}
