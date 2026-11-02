//! Architecture's local view/level controls and canonical modeling entry points.
use super::*;
use crate::{architecture_view, model_tree::TreeEvents};
use multicad_core::project_file::ProjectResources;
use std::collections::BTreeSet;

impl SketchEditor {
    pub fn architecture_recall(&self) -> architecture_view::Recall {
        self.architecture.recall
    }
    pub fn saved_architecture_view(&self) -> Option<architecture_view::Recall> {
        self.architecture.saved()
    }
    pub fn restore_architecture_view(&mut self, recall: architecture_view::Recall) {
        self.architecture.restore(recall, &mut self.camera);
    }
    pub fn set_architecture_active(&mut self, active: bool, session: &Session) {
        if active {
            self.architecture
                .sync(session.document(), session.revision());
        }
        let was_planar = self.architecture.planar();
        self.architecture
            .set_enabled(active && !self.editing && !self.picking, &mut self.camera);
        if !was_planar
            && self.architecture.planar()
            && !session.document().entities().any(|entity| {
                matches!(
                    entity.data,
                    EntityData::Part { .. } | EntityData::Instance { .. }
                )
            })
        {
            self.camera.scale = 0.05;
        }
    }
    pub(super) fn prepare_architecture_level(&mut self, session: &Session) -> Result<(), String> {
        self.architecture
            .sync(session.document(), session.revision());
        self.architecture.elevation()?;
        self.model
            .set_architecture_level(self.architecture.recall.level);
        Ok(())
    }
    pub(super) fn architecture_panel(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        feed: &crate::viewport::SceneFeed,
        scene: Option<&crate::viewport::Scene>,
    ) {
        if !self.architecture.enabled {
            return;
        }
        ui.heading("Architecture");
        let mut plan = self.architecture.recall.plan;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut plan, true, "2D plan");
            ui.selectable_value(&mut plan, false, "3D model");
        });
        if plan != self.architecture.recall.plan {
            self.architecture.set_plan(plan, &mut self.camera);
            if plan
                && !session.document().entities().any(|entity| {
                    matches!(
                        entity.data,
                        EntityData::Part { .. } | EntityData::Instance { .. }
                    )
                })
            {
                self.camera.scale = 0.05;
            }
            ui.ctx().request_repaint();
        }
        if self.model.active() {
            ui.label(format!("Object · {}", self.model.label()));
        } else if let Some(member) = self.architecture.levels.as_ref().and_then(|levels| {
            levels
                .rows
                .iter()
                .flat_map(|level| &level.members)
                .find(|member| self.tree_selection.contains(&member.id))
        }) {
            ui.label(format!("Object · {}", member.kind));
        }
        let can_change = !self.has_pending_edits() && !self.measurement.active();
        let mut active = self.architecture.recall.level;
        let mut selected = None;
        let mut edit = false;
        let mut new_level = false;
        let mut edit_level = false;
        ui.horizontal(|ui| {
            ui.strong("Levels");
            new_level = ui
                .add_enabled(
                    can_change && self.model.has_backend(),
                    egui::Button::new("New level"),
                )
                .clicked();
            edit_level = ui
                .add_enabled(
                    can_change && self.model.has_backend() && active.is_some(),
                    egui::Button::new("Edit level"),
                )
                .clicked();
        });
        if let Err(error) = self.architecture.elevation() {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        egui::ScrollArea::vertical()
            .id_salt("architecture-level-groups")
            .max_height(230.)
            .show(ui, |ui| {
                if let Some(levels) = &self.architecture.levels {
                    for level in &levels.rows {
                        let elevation = level.elevation_mm.as_ref().map_or_else(
                            |_| "needs repair".into(),
                            |value| format!("{value:.0} mm"),
                        );
                        let response = ui.add_enabled(
                            can_change,
                            egui::Button::selectable(
                                active == level.id,
                                format!("{} · {elevation}", level.name),
                            ),
                        );
                        if response.clicked() {
                            active = level.id;
                        }
                        response.on_hover_text(
                            level
                                .owner
                                .as_ref()
                                .map_or_else(Clone::clone, |owner| format!("Owner · {owner}")),
                        );
                        if active == level.id {
                            ui.indent(("architecture-level-members", level.id), |ui| {
                                match &level.owner {
                                    Ok(owner) => {
                                        ui.weak(format!("Owner · {owner}"));
                                    }
                                    Err(error) => {
                                        ui.colored_label(Color32::LIGHT_RED, error);
                                    }
                                }
                                if let Err(error) = &level.elevation_mm {
                                    ui.colored_label(Color32::LIGHT_RED, error);
                                }
                                if level.members.is_empty() {
                                    ui.weak("No level-owned objects");
                                }
                                for member in &level.members {
                                    let response = ui.add_enabled(
                                        can_change,
                                        egui::Button::selectable(
                                            self.tree_selection.contains(&member.id),
                                            format!("{} · {}", member.kind, member.name),
                                        ),
                                    );
                                    if response.clicked() || response.double_clicked() {
                                        selected = Some(member.id);
                                    }
                                    edit |= response.double_clicked();
                                }
                            });
                        }
                    }
                    for error in &levels.diagnostics {
                        ui.colored_label(Color32::LIGHT_RED, error);
                    }
                }
            });
        if active != self.architecture.recall.level {
            self.architecture.recall.level = active;
            if self.architecture.planar() {
                self.architecture.constrain(&mut self.camera);
            }
            self.model.set_architecture_level(active);
            ui.ctx().request_repaint();
        }
        if let Some(id) = selected {
            self.handle_tree_events(
                session,
                scene,
                TreeEvents {
                    selection: Some(BTreeSet::from([id])),
                    ..Default::default()
                },
            );
            if edit && let Err(error) = self.edit_architecture_member(session, resources, feed, id)
            {
                self.message = error;
            }
        }
        if new_level || edit_level {
            let existing = edit_level.then_some(active).flatten();
            match self
                .model
                .begin_level(session, resources, existing, Channel::Ui)
            {
                Ok(()) => {
                    self.message =
                        "Level · change its source dimensions, then Preview and Apply".into()
                }
                Err(error) => self.message = error,
            }
        }
        ui.separator();
    }
    fn edit_architecture_member(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &crate::viewport::SceneFeed,
        id: EntityId,
    ) -> Result<(), String> {
        if let Some(EntityData::Part { features, .. }) = session.document().get(id).map(|e| &e.data)
            && let Some(feature) = features.iter().rev().find(|feature| {
                crate::modeling::ModelToolController::editable(session.document(), **feature)
            })
        {
            return self.begin_feature_edit(session, resources, feed, *feature, Channel::Ui);
        }
        let record = session.document().entities().find_map(|entity| {
            if matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == multicad_core::symbols::INSTANCE_TYPE) {
                multicad_core::symbols::read_instance(session.document(),entity.id).ok()
                    .filter(|symbol| symbol.instance == id).map(|_|entity.id)
            } else { None }
        }).ok_or("Select an editable Architecture object")?;
        if !self.primitive_ready() {
            return Err("Apply or cancel the active operation before editing equipment".into());
        }
        self.model
            .begin_equipment_existing(session, resources, record, Channel::Ui)?;
        self.message =
            "Equipment · edit its saved controls or choose Place in view, then Preview".into();
        Ok(())
    }
}
