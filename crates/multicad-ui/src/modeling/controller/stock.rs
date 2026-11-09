use super::*;
use crate::{
    inspector_actions::{self, Button, Owner},
    materials::{LibraryEditor, ToolStock},
};

impl ModelToolController {
    pub(super) fn begin_stock_editor(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        kind: ToolStock,
    ) -> Result<(), String> {
        self.current(session, resources, true)?;
        self.keyboard_confirm = None;
        self.stock_editor = Some((
            kind,
            Box::new(LibraryEditor::for_model_stock(session, kind)),
        ));
        Ok(())
    }

    pub(crate) fn cancel_stock_editor(&mut self) -> bool {
        self.stock_editor.take().is_some()
    }

    fn save_stock(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<EntityId, String> {
        self.current(session, resources, true)?;
        let (kind, editor) = self
            .stock_editor
            .as_ref()
            .ok_or("Open the stock editor first")?;
        let form = self.form.as_mut().ok_or("Start a modeling tool first")?;
        let compatible = match (&*form, kind) {
            (ModelForm::Cabinet(_), ToolStock::Panel) => true,
            (ModelForm::Sheet(_), ToolStock::Sheet) => true,
            (ModelForm::Weldment(panel), ToolStock::Profile | ToolStock::Batten) => {
                panel.is_batten() == (*kind == ToolStock::Batten)
            }
            _ => false,
        };
        if !compatible {
            return Err("The current tool no longer accepts this stock".into());
        }
        let (stock, prepared) = editor.prepare_model_stock(session)?;
        let source = ContentHash::document(prepared.document())?;
        session
            .commit(prepared)
            .map_err(|error| error.to_string())?;
        match form {
            ModelForm::Cabinet(panel) => panel.select_stock(stock),
            ModelForm::Sheet(panel) => panel.select_stock(stock),
            ModelForm::Weldment(panel) => panel.select_stock(stock),
            _ => unreachable!("checked stock consumer"),
        }
        // Only this known additive edit may advance the current form's scope.
        let scope = self.scope.as_mut().expect("validated modeling scope");
        scope.session = session.stamp();
        scope.source = source;
        self.invalidate();
        self.stock_editor = None;
        Ok(stock)
    }

    pub(super) fn show_stock_editor(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
    ) {
        let mut valid = false;
        if let Some((kind, editor)) = &mut self.stock_editor {
            editor.show_model_stock(ui, session, *kind);
            valid = editor.valid_panel_draft();
        }
        let action = inspector_actions::row(
            ui,
            Owner::Model,
            None,
            Button::new(
                "Apply",
                valid,
                "Save the Project definition and select it in this tool",
            ),
            Button::new(
                "Cancel",
                true,
                "Discard this stock draft and return to the modeling tool",
            ),
        );
        if action.cancel {
            self.cancel_stock_editor();
        } else if action.apply
            && let Err(error) = self.save_stock(session, resources)
            && let Some((_, editor)) = &mut self.stock_editor
        {
            editor.model_stock_error(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        document::Document,
        materials::{self, LibraryDefinition},
        project_file::{self, FileLimits, ProjectFile},
    };
    use std::time::{Duration, Instant};

    #[test]
    fn inline_stock_keeps_the_form_and_persists_native_assignments() {
        let mut session = Session::new(Document::new("In-context stock").unwrap()).unwrap();
        let resources = ProjectResources::default();
        let mut model = ModelToolController::default();
        model.set_backend(Box::new(
            crate::modeling::native_fixture::NativeBackend::new(),
        ));
        let mut outputs = Vec::new();
        let mut shared_material = None;
        for kind in [
            ToolStock::Panel,
            ToolStock::Sheet,
            ToolStock::Profile,
            ToolStock::Batten,
        ] {
            match kind {
                ToolStock::Panel => model.begin_panel(&session, &resources, Channel::Ui),
                ToolStock::Sheet => model.begin_sheet(&session, &resources, Channel::Ui),
                ToolStock::Profile => model.begin_weldment(&session, &resources, Channel::Ui),
                ToolStock::Batten => model.begin_wood_preset(
                    &session,
                    &resources,
                    Vec::new(),
                    Channel::Ui,
                    crate::wood_navigation::Preset::Batten,
                ),
            }
            .unwrap();
            let original = session.stamp();
            let form = model.stamp().unwrap();
            model
                .begin_stock_editor(&session, &resources, kind)
                .unwrap();
            assert!(model.cancel_stock_editor());
            assert!(session.validate_stamp(&original).is_ok());
            assert_eq!(model.stamp(), Some(form));

            model
                .begin_stock_editor(&session, &resources, kind)
                .unwrap();
            let stock = model.save_stock(&mut session, &resources).unwrap();
            let updated = model.stamp().unwrap();
            assert_eq!(updated.tool, form.tool);
            assert_eq!(updated.revision, form.revision + 1);
            let Some(LibraryDefinition::Stock(definition)) =
                materials::read(session.document().get(stock).unwrap()).unwrap()
            else {
                panic!("Expected a shared stock definition");
            };
            let material = shared_material.get_or_insert(definition.material);
            assert_eq!(*material, definition.material);

            model.preview(&session, &resources, Channel::Ui).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while model.pending() {
                model.poll(&session, &resources);
                assert!(
                    Instant::now() < deadline,
                    "Native stock consumer preview timed out"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(model.can_apply(&session, &resources), "{:?}", model.error());
            let applied = model.apply(&mut session, &resources).unwrap();
            assert_eq!(applied.len(), 1);
            outputs.push((kind, stock, applied[0].feature));
        }

        let mut bytes = Vec::new();
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            FileLimits::default(),
        )
        .unwrap();
        let restored = project_file::read(
            &mut bytes.as_slice(),
            FileLimits::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        for (kind, stock, feature) in outputs {
            let assigned = match kind {
                ToolStock::Panel => match model::read_panel(&restored, feature).unwrap().source {
                    model::PanelSource::Rectangle { stock, .. } => stock,
                    _ => panic!("Expected a rectangular panel"),
                },
                ToolStock::Sheet => model::read_sheet(&restored, feature).unwrap().stock,
                ToolStock::Profile | ToolStock::Batten => {
                    model::read_member(&restored, feature).unwrap().stock
                }
            };
            assert_eq!(assigned, stock);
            assert!(restored.get(stock).is_some());
        }
    }
}
