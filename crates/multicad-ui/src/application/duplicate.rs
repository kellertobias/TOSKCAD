use super::{Application, Channel, Mode};

impl Application {
    pub(super) fn duplicate_model_selection(&mut self, channel: Channel) -> Result<(), String> {
        if self.mode != Mode::Model {
            return Err("Open Model and select Parts or Instances to duplicate".into());
        }
        let project = self.current_mut();
        if project.pending_edits() {
            return Err("Apply or cancel pending edits before duplicating".into());
        }
        let count =
            project
                .sketch
                .duplicate_selection(&mut project.session, &project.scene, channel)?;
        self.message = if count == 1 {
            "Created one placed copy · Undo removes the copy".into()
        } else {
            format!("Created {count} placed copies · Undo removes the copies")
        };
        Ok(())
    }
}
