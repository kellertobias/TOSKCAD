use super::*;
use crate::modeling::native_fixture;
use multicad_core::{
    document::{Document, Entity, Mutation},
    model::*,
};
use std::time::{Duration, Instant};
fn session() -> Session {
    let material = Entity::new(
        "Timber",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [150, 100, 70, 255],
            section_hatch: None,
        },
    );
    Session::new(
        Document::new("Native fence controller")
            .unwrap()
            .edited(&[Mutation::Insert(material)])
            .unwrap(),
    )
    .unwrap()
}
fn controller() -> ModelToolController {
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c
}
fn wait(c: &mut ModelToolController, s: &Session) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < deadline, "Fence native worker timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn native_fence_preview_cancel_apply_history_and_member_reopen_use_the_real_gate() {
    let mut s = session();
    let before = s.document().clone();
    let mut c = controller();
    c.begin_fence(&s, &Default::default(), Channel::Ui).unwrap();
    assert_eq!(c.label(), "Fence");
    assert!(!c.can_apply(&s, &Default::default()));
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    assert_eq!(s.document(), &before);
    c.cancel();
    assert_eq!(s.document(), &before);
    c.begin_fence(&s, &Default::default(), Channel::Ui).unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    let outputs = c.apply(&mut s, &Default::default()).unwrap();
    assert_eq!(outputs.len(), 5);
    let run = fence_runs(s.document()).next().unwrap().id;
    let definition = read_fence_run(s.document(), run).unwrap();
    let committed = s.document().clone();
    s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &committed);
    for target in [run, definition.group, outputs[0].body, outputs[0].feature] {
        assert!(ModelToolController::editable(s.document(), target));
        c.begin_existing(&s, &Default::default(), target, Channel::Ui)
            .unwrap();
        assert_eq!(c.label(), "Fence");
        let ModelRequest::Fence(change) = c
            .form
            .as_ref()
            .unwrap()
            .request(s.document(), c.capabilities())
            .unwrap()
        else {
            panic!("Fence source change")
        };
        assert!(matches!(&change.run,Some(FenceRunChange::Edit(request))if request.id==run));
        c.cancel();
        assert_eq!(s.document(), &committed);
    }
}

#[test]
fn active_fence_level_drives_native_members_and_saved_runs_keep_their_attachment() {
    let mut s = session();
    let (level, prepared) = prepare_level(
        &s,
        s.revision(),
        Actor::local(Channel::Ui),
        "Upper",
        &LevelDefinition {
            elevation: length(3000.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    s.commit(prepared).unwrap();
    let before = s.document().clone();
    let mut c = controller();
    c.set_architecture_level(Some(level));
    c.begin_fence(&s, &Default::default(), Channel::Ui).unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    let scene = c.display_scene(&s, &Default::default()).unwrap();
    assert!(scene.world_bounds().unwrap()[0][2] >= 3000.);
    assert_eq!(s.document(), &before);
    let outputs = c.apply(&mut s, &Default::default()).unwrap();
    let run = fence_runs(s.document()).next().unwrap().id;
    let definition = read_fence_run(s.document(), run).unwrap();
    assert_eq!(definition.level, Some(level));
    let facts = crate::architecture_view::Levels::new(s.document(), s.revision());
    assert_eq!(facts.level_for(definition.group), Some(Some(level)));
    for output in outputs {
        assert_eq!(facts.level_for(output.body), Some(Some(level)));
        assert_eq!(facts.level_for(output.feature), Some(Some(level)));
    }
    assert_eq!(
        facts
            .rows
            .iter()
            .find(|row| row.id == Some(level))
            .unwrap()
            .members
            .iter()
            .filter(|member| member.kind == "Fence")
            .count(),
        1
    );
    c.set_architecture_level(None);
    c.begin_existing(&s, &Default::default(), definition.group, Channel::Ui)
        .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()));
    let candidate = c.candidate.as_ref().unwrap().prepared.document();
    assert_eq!(read_fence_run(candidate, run).unwrap().level, Some(level));
    c.cancel();
}

#[test]
fn subsequent_runs_reuse_the_selected_saved_construction() {
    let mut s = session();
    let mut c = controller();
    c.begin_fence(&s, &Default::default(), Channel::Ui).unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    c.apply(&mut s, &Default::default()).unwrap();
    let selected = fence_types(s.document()).next().unwrap().id;
    c.begin_fence(&s, &Default::default(), Channel::Ui).unwrap();
    let ModelRequest::Fence(change) = c
        .form
        .as_ref()
        .unwrap()
        .request(s.document(), c.capabilities())
        .unwrap()
    else {
        panic!("Fence")
    };
    assert!(change.new_type.is_none());
    assert!(
        matches!(&change.run,Some(FenceRunChange::Create(request))if request.definition.fence_type==selected)
    );
    c.cancel();
}

#[test]
fn a_native_overlap_failure_never_enables_apply_or_modifies_the_source() {
    let mut s = session();
    let panel =
        super::super::fence::FencePanel::new(&s, "Unused construction".into(), None).unwrap();
    let mut change = panel.request(s.document(), true).unwrap();
    let mut ty = change.new_type.take().unwrap();
    let mut duplicate = ty.construction.infills[0].clone();
    duplicate.id = EntityId::new();
    duplicate.name = "Overlapping panel".into();
    ty.construction.infills.push(duplicate);
    let prepared = prepare_fence_change(
        &s,
        s.revision(),
        Actor::local(Channel::Ui),
        &FenceChange {
            new_type: Some(ty),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(prepared.model.outputs.is_empty());
    s.commit(prepared.model.prepared).unwrap();
    let before = s.document().clone();
    let mut c = controller();
    c.begin_fence(&s, &Default::default(), Channel::Ui).unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(!c.can_apply(&s, &Default::default()));
    assert!(
        c.error().is_some_and(|error| error.contains("overlap")),
        "{:?}",
        c.error()
    );
    assert!(c.apply(&mut s, &Default::default()).is_err());
    c.cancel();
    assert_eq!(s.document(), &before);
}
