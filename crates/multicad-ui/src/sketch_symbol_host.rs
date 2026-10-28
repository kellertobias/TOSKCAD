//! Symbol tools share the model viewport, selection and fixed Inspector lane.
use super::*;
use crate::symbols::{SymbolAction, SymbolOutcome, SymbolSelection, SymbolTool};
use multicad_core::project_file::ProjectResources;

impl SketchEditor {
    pub fn set_symbol_backend(&mut self, backend: Box<dyn crate::viewport::SceneBackend>) {
        self.symbols.set_backend(backend);
    }
    pub fn has_symbol_backend(&self) -> bool {
        self.symbols.has_backend()
    }
    pub fn symbol_active(&self) -> bool {
        self.symbols.active()
    }
    pub fn close_symbol_browser(&mut self) {
        if self.symbols.active() && !self.symbols.dirty() {
            self.symbols.cancel();
        }
    }
    pub fn set_symbol_library(
        &mut self,
        definitions: Vec<multicad_core::symbols::SymbolDefinition>,
    ) -> Result<(), String> {
        self.symbols.set_library_snapshot(definitions)
    }
    pub fn take_symbol_action(&mut self) -> Option<SymbolAction> {
        self.symbols.take_action()
    }
    pub fn begin_symbol(
        &mut self,
        tool: SymbolTool,
        session: &Session,
        resources: &ProjectResources,
        feed: &crate::viewport::SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if self.has_pending_edits() || self.symbols.active() || self.measurement.active() {
            return Err("Apply or cancel the active tool before opening symbols".into());
        }
        let mut selected = self.tree_selection.clone();
        selected.extend(self.solid.selected_ids());
        let instance = selected.iter().copied().find(|id| {
            matches!(
                session.document().get(*id).map(|e| &e.data),
                Some(EntityData::Instance { .. })
            )
        });
        let component =
            selected
                .iter()
                .find_map(|id| match session.document().get(*id).map(|e| &e.data) {
                    Some(EntityData::Part { .. } | EntityData::Assembly { .. }) => Some(*id),
                    Some(EntityData::Instance { component, .. }) => Some(*component),
                    _ => None,
                });
        let plan_sketch = self.selected.filter(|id| {
            matches!(
                session.document().get(*id).map(|e| &e.data),
                Some(EntityData::Sketch { .. })
            )
        });
        self.finish()?;
        self.symbols.begin(
            tool,
            session,
            resources,
            feed.visible()
                .filter(|_| feed.is_current(session.document().id(), session.revision())),
            SymbolSelection {
                component,
                instance,
                plan_sketch,
            },
            channel,
        )?;
        self.message = format!(
            "{} · choose content and parameters in the Inspector",
            self.symbols.label()
        );
        Ok(())
    }
    pub(super) fn accept_symbol_outcome(&mut self, outcome: SymbolOutcome) {
        match outcome {
            SymbolOutcome::Applied { selected, label } => {
                self.tree_selection = selected.into_iter().collect();
                self.selected = None;
                self.solid.clear();
                self.fit_requested = true;
                self.message = format!("{label} applied · one undoable transaction");
            }
            SymbolOutcome::Cancelled => {
                self.message = "Symbol operation cancelled · document unchanged".into()
            }
        }
    }
    pub(super) fn symbol_edit_controls(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        feed: &crate::viewport::SceneFeed,
    ) {
        if self.tree_selection.len() != 1 || self.has_pending_edits() || self.symbols.active() {
            return;
        }
        let id = *self.tree_selection.first().expect("one selected");
        let Ok(instance) = multicad_core::symbols::read_instance(session.document(), id) else {
            return;
        };
        if let Ok(definition) =
            multicad_core::symbols::read_definition(session.document(), instance.definition)
        {
            ui.label(format!(
                "Symbol · {} · revision {}",
                definition.name, definition.revision
            ));
            ui.small(format!("Pinned {}", instance.pin));
        }
        if ui.button("Edit symbol parameters").on_hover_text("Change only this symbol's exposed values; native Preview checks its complete model").clicked()
            && let Err(error)=self.symbols.begin_existing(session,resources,
                feed.visible().filter(|_|feed.is_current(session.document().id(),session.revision())),id,Channel::Ui){self.message=error;}
        if ui
            .button("Review symbol update")
            .on_hover_text(
                "Compare an explicitly chosen library revision before changing placed instances",
            )
            .clicked()
            && let Err(error) =
                self.begin_symbol(SymbolTool::Update, session, resources, feed, Channel::Ui)
        {
            self.message = error;
        }
    }
}
