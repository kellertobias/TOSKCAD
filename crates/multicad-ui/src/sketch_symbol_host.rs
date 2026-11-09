//! Library browsing opens over the model. Symbol editing shares the viewport and Inspector.
use super::*;
use crate::symbols::{SymbolAction, SymbolOutcome, SymbolSelection, SymbolTool};
use multicad_core::project_file::ProjectResources;

pub(super) struct ArchitectureSelectionCache {
    stamp: multicad_core::commands::SessionStamp,
    selected: EntityId,
    instance: Option<(EntityId, EntityId, multicad_core::project_file::ContentHash)>,
}

impl SketchEditor {
    pub(crate) fn hierarchy_tool_active(&self) -> bool {
        self.tree.tool_active()
    }
    pub(crate) fn cancel_hierarchy_tool(&mut self) {
        self.tree.cancel_tool();
        self.message = "Assembly draft discarded · document unchanged".into();
    }
    pub(crate) fn begin_create_assembly(&mut self, session: &Session) -> Result<(), String> {
        if self.has_pending_edits() || self.symbols.active() || self.measurement.active() {
            return Err("Apply or cancel the active tool before creating an assembly".into());
        }
        self.finish()?;
        self.tree.begin_create(
            session,
            multicad_core::hierarchy::ContainerKind::Component,
            session.document().hierarchy().active_component,
        );
        self.message = "Create Assembly · review the name and parent in the Inspector".into();
        Ok(())
    }
    pub fn set_installed_symbol_library(
        &mut self,
        entries: std::sync::Arc<Vec<crate::library::LibraryEntry>>,
    ) {
        self.model.set_connection_library_entries(entries.clone());
        self.symbols.set_installed_library(entries);
    }
    pub fn begin_architectural_symbol(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        definition: EntityId,
        role: multicad_core::symbols::architecture::ArchitectureRole,
        channel: Channel,
    ) -> Result<(), String> {
        self.close_symbol_browser();
        if self.has_pending_edits() || self.symbols.active() || self.measurement.active() {
            return Err(
                "Apply or cancel the active tool before drawing a reusable construction".into(),
            );
        }
        self.finish()?;
        match role {
            multicad_core::symbols::architecture::ArchitectureRole::Wall => {
                self.model.begin_wall(session, resources, channel)?
            }
            multicad_core::symbols::architecture::ArchitectureRole::Fence => {
                self.model.begin_fence(session, resources, channel)?
            }
        }
        self.model
            .select_architectural_definition(session, definition)?;
        self.message = "Draw the construction path in the viewer, then Preview".into();
        Ok(())
    }
    pub fn set_symbol_backend(&mut self, backend: Box<dyn crate::viewport::SceneBackend>) {
        self.symbols.set_backend(backend);
    }
    pub fn has_symbol_backend(&self) -> bool {
        self.symbols.has_backend()
    }
    pub fn symbol_active(&self) -> bool {
        self.symbols.active()
    }
    pub(crate) fn symbol_library_active(&self) -> bool {
        self.symbols.library_active()
    }
    pub(crate) fn show_symbol_library(
        &mut self,
        ctx: &egui::Context,
        session: &mut Session,
        resources: &ProjectResources,
    ) {
        if self.symbols.library_active()
            && let Some(outcome) = self.symbols.show_library_modal(ctx, session, resources)
        {
            self.accept_symbol_outcome(outcome);
        }
    }
    pub(crate) fn cancel_symbol_tool(&mut self) {
        self.symbols.cancel();
        self.accept_symbol_outcome(SymbolOutcome::Cancelled);
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
    pub(crate) fn set_symbol_library_from_file(
        &mut self,
        definitions: Vec<multicad_core::symbols::SymbolDefinition>,
        file_text: &str,
    ) -> Result<(), String> {
        self.symbols
            .set_library_file_snapshot(definitions, file_text)
    }
    pub(crate) fn library_placement_intent(&self) -> Option<u64> {
        self.symbols.library_placement_intent()
    }
    pub fn place_loaded_library(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        self.symbols.place_loaded_library(session, resources)
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
        let instance = instance.or_else(|| {
            selected.iter().find_map(|id| {
                multicad_core::symbols::architecture::architecture_instance_for_output(
                    session.document(),
                    *id,
                )
                .ok()
                .flatten()
                .map(|(record, _)| record)
            })
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
        // Closing the active modeling context must not drop an exact viewport
        // selection just because the user opened the Library to browse.
        let solid_selection =
            (tool == SymbolTool::Library && !self.editing).then(|| std::mem::take(&mut self.solid));
        let finished = self.finish();
        if let Some(selection) = solid_selection {
            self.solid = selection;
        }
        finished?;
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
                parent: session.document().hierarchy().active_component,
            },
            channel,
        )?;
        self.message = if tool == SymbolTool::Library {
            "Library · browse parts and symbols".into()
        } else {
            format!(
                "{} · choose content and parameters in the Inspector",
                self.symbols.label()
            )
        };
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
        if self
            .architectural_symbol_context
            .as_ref()
            .is_none_or(|cache| cache.stamp != session.stamp() || cache.selected != id)
        {
            let value = multicad_core::symbols::architecture::architecture_instance_for_output(
                session.document(),
                id,
            )
            .ok()
            .flatten()
            .map(|(record, value)| (record, value.definition, value.pin));
            self.architectural_symbol_context = Some(ArchitectureSelectionCache {
                stamp: session.stamp(),
                selected: id,
                instance: value,
            });
        }
        if let Some((record, definition, pin)) = self
            .architectural_symbol_context
            .as_ref()
            .and_then(|cache| cache.instance)
        {
            if let Some(multicad_core::document::Entity {
                data: EntityData::Extension { definition, .. },
                ..
            }) = session.document().get(definition)
                && let Some(name) = definition.data.get("name").and_then(|v| v.as_str())
            {
                ui.label(format!("Architectural symbol · {name}"));
            }
            ui.small(format!("Pinned {pin}"));
            if ui.button("Edit exposed construction inputs").clicked()
                && let Err(error) = self.symbols.begin_existing(
                    session,
                    resources,
                    feed.visible()
                        .filter(|_| feed.is_current(session.document().id(), session.revision())),
                    record,
                    Channel::Ui,
                )
            {
                self.message = error;
            }
            if ui.button("Review definition update").clicked()
                && let Err(error) =
                    self.begin_symbol(SymbolTool::Update, session, resources, feed, Channel::Ui)
            {
                self.message = error;
            }
            return;
        }
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
