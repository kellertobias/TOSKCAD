use super::SketchEditor;
use crate::viewport::SceneFeed;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::EntityData,
    model::{self, BodyInput, InstanceCopyRequest, InstanceCopySource},
    project_file::ContentHash,
};

impl SketchEditor {
    pub(crate) fn duplicate_ready(&self, session: &Session, feed: &SceneFeed) -> bool {
        !self.measurement.active() && self.placement_ready(session, feed)
    }

    pub(crate) fn duplicate_selection(
        &mut self,
        session: &mut Session,
        feed: &SceneFeed,
        channel: Channel,
    ) -> Result<usize, String> {
        if !self.duplicate_ready(session, feed) {
            return Err(
                "Select current Parts or Instances and finish any active tool before duplicating"
                    .into(),
            );
        }
        let document = session.document();
        let scene = feed.visible().expect("current placement scene");
        let source = ContentHash::document(document)?;
        if scene
            .model_evaluation()
            .is_none_or(|evaluation| evaluation.source_hash != source)
        {
            return Err("Wait for the current source-bound native model before duplicating".into());
        }
        let requests = self
            .tree_selection
            .iter()
            .map(|&id| {
                let entity = document.get(id).ok_or("Selected source no longer exists")?;
                let source = match &entity.data {
                    EntityData::Part { .. } => {
                        let placed = scene
                            .reference_instances()
                            .iter()
                            .find(|placed| placed.id == id)
                            .ok_or("Selected native Part is no longer available")?;
                        InstanceCopySource::Part {
                            source: BodyInput {
                                body: id,
                                producer: placed.geometry,
                            },
                        }
                    }
                    EntityData::Instance { .. } => InstanceCopySource::Instance { instance: id },
                    _ => return Err("Select Parts or Instances to duplicate"),
                };
                let mut request = InstanceCopyRequest::new(source, Default::default());
                let name = &entity.name[..entity.name.floor_char_boundary(995)];
                request.name = format!("{name} copy");
                Ok(request)
            })
            .collect::<Result<Vec<_>, &str>>()?;

        let prepared = model::prepare_instance_copies(
            session,
            session.revision(),
            Actor::local(channel),
            &requests,
        )
        .map_err(|error| error.to_string())?;
        session
            .commit(prepared.prepared)
            .map_err(|error| error.to_string())?;
        let count = prepared.instances.len();
        self.accept_model_outputs(Vec::new(), "Duplicate", false);
        self.tree_selection.extend(prepared.instances);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::native_fixture::NativeBackend;
    use multicad_core::{
        document::Document,
        model::{
            InstanceMoveRequest, NumericTransform, PrimitiveParameters, PrimitiveRequest, length,
        },
    };
    use std::time::{Duration, Instant};

    fn translated(x: f64) -> NumericTransform {
        NumericTransform {
            translation: [length(x), length(0.), length(0.)],
            ..Default::default()
        }
    }

    #[test]
    fn duplicate_keeps_current_pose_and_copies_numeric_drivers_as_one_undoable_edit() {
        let actor = Actor::local(Channel::Ui);
        let mut session = Session::new(Document::new("Duplicate test").unwrap()).unwrap();
        let part = PrimitiveRequest::new(
            "Block",
            PrimitiveParameters::Box {
                size: [length(24.), length(16.), length(12.)],
                frame: Default::default(),
            },
        );
        let prepare = model::prepare_primitive(&session, 0, actor.clone(), &part).unwrap();
        session.commit(prepare.prepared).unwrap();
        let source = InstanceCopyRequest::new(
            InstanceCopySource::Part {
                source: BodyInput {
                    body: part.body,
                    producer: part.feature,
                },
            },
            translated(30.),
        );
        let prepare = model::prepare_instance_copies(
            &session,
            1,
            actor.clone(),
            std::slice::from_ref(&source),
        )
        .unwrap();
        session.commit(prepare.prepared).unwrap();

        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(NativeBackend::new()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !feed.is_current(session.document().id(), session.revision()) {
            feed.tick(session.document(), session.revision());
            assert!(Instant::now() < deadline, "{:?}", feed.error());
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(NativeBackend::new()));
        assert!(!editor.duplicate_ready(&session, &feed));
        assert!(
            editor
                .duplicate_selection(&mut session, &feed, Channel::Ui)
                .is_err()
        );
        let before = session.document().clone();
        editor.tree_selection.extend([part.body, source.instance]);
        assert!(editor.duplicate_ready(&session, &feed));
        assert_eq!(
            editor
                .duplicate_selection(&mut session, &feed, Channel::Ui)
                .unwrap(),
            2
        );
        assert_eq!(session.revision(), 3);
        assert_eq!(
            session.document().get(source.instance),
            before.get(source.instance)
        );
        assert!(!editor.duplicate_ready(&session, &feed));

        let original_pose =
            model::resolve_instance_pose(session.document(), source.instance, session.parameters())
                .unwrap();
        let copy = editor
            .tree_selection
            .iter()
            .copied()
            .find(|&id| {
                model::resolve_instance_pose(session.document(), id, session.parameters()).ok()
                    == Some(original_pose)
            })
            .unwrap();
        assert_ne!(copy, source.instance);
        let move_copy = InstanceMoveRequest::new(copy, translated(6.));
        let prepare =
            model::prepare_instance_moves(&session, 3, actor.clone(), &[move_copy]).unwrap();
        session.commit(prepare.prepared).unwrap();
        assert_eq!(
            model::resolve_instance_pose(session.document(), source.instance, session.parameters())
                .unwrap(),
            original_pose
        );
        assert_eq!(
            model::resolve_instance_pose(session.document(), copy, session.parameters())
                .unwrap()
                .translation_mm,
            [36., 0., 0.]
        );
        session.undo(4, actor.clone()).unwrap();
        session.undo(5, actor).unwrap();
        assert_eq!(session.document(), &before);
    }
}
