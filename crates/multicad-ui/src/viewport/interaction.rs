use super::{
    Hit, PickFilter, PickOptions, Scene, SceneStamp, SectionPlane, SectionView, Selection,
    paint_scene_section,
};
use crate::{
    camera::Camera,
    icons::{self, Icon},
    navigation::DisplayStyle,
};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    references::TopologyKind,
};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Default)]
pub struct Interaction {
    selected: Option<Hit>,
    hovered: Option<Hit>,
    parts: Arc<BTreeSet<EntityId>>,
    range_anchor: Option<EntityId>,
    action_error: Option<String>,
    pub filter: PickFilter,
    pub section: SectionView,
    reference_highlight: Option<Selection>,
    last_stamp: Option<SceneStamp>,
    last_query: Option<(
        Pos2,
        Rect,
        Camera,
        DisplayStyle,
        PickFilter,
        Option<SectionPlane>,
    )>,
}
impl Interaction {
    pub fn highlight_reference(&mut self, selection: Option<Selection>) {
        self.reference_highlight = selection;
    }
    pub fn clear(&mut self) {
        self.selected = None;
        self.hovered = None;
        Arc::make_mut(&mut self.parts).clear();
        self.range_anchor = None;
        self.action_error = None;
        self.last_query = None;
    }
    pub fn has_topology_selection(&self) -> bool {
        self.selected.is_some()
    }
    pub fn hovered_instance(&self) -> Option<EntityId> {
        self.hovered.as_ref().map(|hit| hit.instance)
    }
    pub fn selected_id(&self) -> Option<EntityId> {
        self.parts
            .first()
            .copied()
            .or_else(|| self.selected.as_ref().map(|h| h.instance))
    }
    pub fn selected_ids(&self) -> BTreeSet<EntityId> {
        let mut ids = (*self.parts).clone();
        if let Some(hit) = &self.selected {
            ids.insert(hit.instance);
        }
        ids
    }
    /// Synchronize canonical tree selection with visible placed occurrences.
    /// The host retains non-rendered source/container selection separately.
    pub fn select_source_entities(
        &mut self,
        document: &Document,
        revision: u64,
        scene: &Scene,
        source: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        if scene.stamp().document != document.id() || scene.stamp().revision != revision {
            return Err("Wait for a current scene before selecting model occurrences".into());
        }
        if source
            .iter()
            .any(|id| *id != document.id() && document.get(*id).is_none())
        {
            return Err("The selected model entry no longer exists".into());
        }
        let selected = scene.instances().iter().filter(|instance| {
            source.contains(&instance.id) || document.get(instance.id).is_some_and(|entity| {
                matches!(entity.data, EntityData::Instance { component, .. } if source.contains(&component))
            })
        }).map(|instance| instance.id).collect();
        self.clear();
        self.parts = Arc::new(selected);
        self.last_stamp = Some(scene.stamp());
        Ok(())
    }
    pub fn selected_bounds(&self, scene: &Scene) -> Option<[[f64; 3]; 2]> {
        let mut result: Option<[[f64; 3]; 2]> = None;
        for id in self.selected_ids() {
            if let Some(bounds) = scene.instance_bounds(id) {
                if let Some(current) = result.as_mut() {
                    for i in 0..3 {
                        current[0][i] = current[0][i].min(bounds[0][i]);
                        current[1][i] = current[1][i].max(bounds[1][i]);
                    }
                } else {
                    result = Some(bounds);
                }
            }
        }
        result
    }
    fn select_part(&mut self, id: EntityId, toggle: bool) {
        let old = self.selected.take().map(|hit| hit.instance);
        let parts = Arc::make_mut(&mut self.parts);
        if !toggle {
            parts.clear();
        } else if let Some(old) = old {
            parts.insert(old);
        }
        if !toggle || !parts.remove(&id) {
            parts.insert(id);
        }
        self.range_anchor = Some(id);
        self.action_error = None;
        self.hovered = None;
        self.last_query = None;
    }
    pub fn tree(&mut self, ui: &mut egui::Ui, document: &Document, scene: &Scene) -> bool {
        let mut rows: Vec<_> = scene
            .instances()
            .iter()
            .filter_map(|i| document.get(i.id))
            .collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        if rows.is_empty() {
            return false;
        }
        ui.strong("Parts & instances");
        let mut changed = false;
        egui::ScrollArea::vertical()
            .id_salt("solid-tree")
            .max_height(340.)
            .show_rows(ui, 22., rows.len(), |ui, range| {
                for i in range {
                    let e = rows[i];
                    if ui
                        .selectable_label(self.parts.contains(&e.id)||self.selected.as_ref().is_some_and(|h|h.instance==e.id), &e.name)
                        .on_hover_text("Select this part · Cmd/Ctrl-click toggles · Shift-click selects a range")
                        .clicked()
                    {
                        let modifiers=ui.input(|i|i.modifiers);
                        if modifiers.shift && let Some(start)=self.range_anchor.and_then(|id|rows.iter().position(|row|row.id==id)) {
                            self.selected=None;
                            let parts=Arc::make_mut(&mut self.parts);
                            if !modifiers.command {parts.clear();}
                            parts.extend(rows[start.min(i)..=start.max(i)].iter().map(|row|row.id));
                            self.hovered=None;
                            self.last_query=None;
                        } else {self.select_part(e.id,modifiers.command);}
                        changed = true;
                    }
                }
            });
        changed
    }
    pub fn inspector(&self, ui: &mut egui::Ui, document: &Document) {
        if self.parts.len() > 1 {
            ui.strong(format!("{} selected parts", self.parts.len()));
            for id in self.parts.iter().take(12) {
                if let Some(entity) = document.get(*id) {
                    ui.label(&entity.name);
                }
            }
            if self.parts.len() > 12 {
                ui.small(format!("… and {} more", self.parts.len() - 12));
            }
            let components: Vec<_> = self
                .parts
                .iter()
                .filter_map(|id| document.get(*id))
                .map(|entity| match entity.data {
                    EntityData::Instance { component, .. } => {
                        document.get(component).unwrap_or(entity)
                    }
                    _ => entity,
                })
                .collect();
            let materials: BTreeSet<_> = components
                .iter()
                .map(|entity| match entity.data {
                    EntityData::Part { material, .. } => material,
                    _ => None,
                })
                .collect();
            if materials.len() > 1 {
                ui.label("Material: Mixed");
            } else if let Some(material) = materials
                .first()
                .copied()
                .flatten()
                .and_then(|id| document.get(id))
            {
                ui.label(format!("Material: {}", material.name));
            }
            let shared_tags = components
                .iter()
                .map(|entity| match &entity.data {
                    EntityData::Part { tags, .. } => tags.clone(),
                    _ => BTreeSet::new(),
                })
                .reduce(|a, b| a.intersection(&b).cloned().collect())
                .unwrap_or_default();
            if !shared_tags.is_empty() {
                ui.label(format!(
                    "Shared tags: {}",
                    shared_tags.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
            return;
        }
        let Some(id) = self.selected_id() else { return };
        let Some(entity) = document.get(id) else {
            return;
        };
        ui.strong(&entity.name);
        if let Some(hit) = &self.selected {
            ui.label(format!("{:?}", hit.key.kind));
            if hit.intent.is_none() {
                ui.small("No unique source reference is available for this selection.");
            }
            ui.small(format!(
                "X {:.3} · Y {:.3} · Z {:.3} mm",
                hit.point_mm[0], hit.point_mm[1], hit.point_mm[2]
            ));
        }
        let component = match entity.data {
            EntityData::Instance { component, .. } => document.get(component).unwrap_or(entity),
            _ => entity,
        };
        if let EntityData::Part { material, tags, .. } = &component.data {
            if let Some(material) = material.and_then(|id| document.get(id)) {
                ui.label(format!("Material: {}", material.name));
            }
            if !tags.is_empty() {
                ui.label(format!(
                    "Tags: {}",
                    tags.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    fn duplicate_selected(&mut self, session: &mut Session) -> Result<(), String> {
        let stamp = self.last_stamp.ok_or("The selected scene is not ready")?;
        if stamp.document != session.document().id() || stamp.revision != session.revision() {
            return Err(
                "Selection belongs to an earlier source revision; wait for the model update".into(),
            );
        }
        let ids = self.selected_ids();
        if ids.is_empty() {
            return Err("Select an instance to duplicate".into());
        }
        let mut created = BTreeSet::new();
        let mut edits = Vec::new();
        for id in ids {
            let source = session
                .document()
                .get(id)
                .ok_or("Selected source no longer exists")?;
            let data = match &source.data {
                EntityData::Instance { .. } => source.data.clone(),
                EntityData::Part { .. } | EntityData::Assembly { .. } => EntityData::Instance {
                    component: id,
                    placement: Placement::default(),
                },
                _ => {
                    return Err(
                        "Only parts, assemblies and their instances can be duplicated".into(),
                    );
                }
            };
            let mut copy = Entity::new(format!("{} copy", source.name), data);
            copy.description = source.description.clone();
            created.insert(copy.id);
            edits.push(Mutation::Insert(copy));
        }
        session
            .apply(Request {
                expected_revision: stamp.revision,
                actor: Actor::local(Channel::Ui),
                label: "Duplicate selected instances".into(),
                edits,
            })
            .map_err(|e| e.to_string())?;
        self.clear();
        self.parts = Arc::new(created);
        Ok(())
    }
    /// Source actions use the canonical transaction API; view actions return a
    /// fit request to the host so all camera fitting uses one implementation.
    pub fn quick_actions(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        current: bool,
    ) -> bool {
        if self.selected_id().is_none() {
            return false;
        }
        let mut fit = false;
        ui.horizontal_wrapped(|ui| {
            fit = ui
                .button("Fit selection")
                .on_hover_text("Frame all selected parts without changing source")
                .clicked();
            let duplicate = current
                && self.selected_ids().iter().all(|id| {
                    matches!(
                        session.document().get(*id).map(|e| &e.data),
                        Some(
                            EntityData::Instance { .. }
                                | EntityData::Part { .. }
                                | EntityData::Assembly { .. }
                        )
                    )
                });
            if ui
                .add_enabled(duplicate, egui::Button::new("Duplicate"))
                .on_hover_text(
                    "Create linked instances at the same placement in one undoable transaction",
                )
                .on_disabled_hover_text("Wait for a current model and select parts or instances")
                .clicked()
            {
                self.action_error = self.duplicate_selected(session).err();
            }
            if ui
                .button("Clear selection")
                .on_hover_text("Clear selected parts and topology")
                .clicked()
            {
                self.clear();
            }
        });
        if let Some(error) = &self.action_error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        fit
    }
    pub fn filter_control(&mut self, ui: &mut egui::Ui) {
        let button = icons::compact_button(
            ui,
            Icon::Edge,
            &format!("Selection filter · {}", self.filter.label()),
        );
        egui::Popup::menu(&button).show(|ui| {
            for filter in PickFilter::ALL {
                if ui
                    .selectable_value(&mut self.filter, filter, filter.label())
                    .clicked()
                {
                    ui.close();
                }
            }
        });
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        scene: Arc<Scene>,
        camera: Camera,
        style: DisplayStyle,
        enabled: bool,
    ) {
        if self.last_stamp != Some(scene.stamp()) {
            if self.last_stamp.is_some_and(|old| {
                old.document == scene.stamp().document && old.revision == scene.stamp().revision
            }) {
                self.selected = self
                    .selected
                    .as_ref()
                    .and_then(|hit| scene.refresh_hit(hit));
                Arc::make_mut(&mut self.parts)
                    .retain(|id| scene.instances().iter().any(|i| i.id == *id));
                self.hovered = None;
                self.last_query = None;
            } else {
                let same_document = self
                    .last_stamp
                    .is_some_and(|old| old.document == scene.stamp().document);
                let parts = if same_document {
                    self.parts.clone()
                } else {
                    Arc::default()
                };
                self.clear();
                self.parts = parts;
                Arc::make_mut(&mut self.parts)
                    .retain(|id| scene.instances().iter().any(|i| i.id == *id));
            }
            self.last_stamp = Some(scene.stamp());
        }
        if enabled
            && response.hovered()
            && let Some(pointer) = response.hover_pos()
        {
            let query = (
                pointer,
                response.rect,
                camera,
                style,
                self.filter,
                self.section.plane(),
            );
            if self.last_query != Some(query) {
                self.hovered = scene.pick_screen_with(
                    camera,
                    response.rect,
                    pointer,
                    PickOptions {
                        filter: self.filter,
                        include_hidden: matches!(
                            style,
                            DisplayStyle::Wire | DisplayStyle::AllEdges
                        ),
                        section: self.section.plane(),
                    },
                );
                self.last_query = Some(query);
            }
            if response.clicked() {
                let modifiers = ui.input(|i| i.modifiers);
                if let Some(hit) = self.hovered.clone()
                    && (modifiers.command || modifiers.shift || self.filter == PickFilter::Parts)
                {
                    self.select_part(hit.instance, modifiers.command || modifiers.shift);
                    // The host captures the actual clicked occurrence even when
                    // a Parts/modifier click stores a set rather than a face.
                    self.hovered = Some(hit);
                } else {
                    self.selected = self.hovered.clone();
                    Arc::make_mut(&mut self.parts).clear();
                    self.range_anchor = None;
                }
            }
        } else {
            self.hovered = None;
            self.last_query = None;
        }
        let hit = self.hovered.as_ref().or(self.selected.as_ref());
        let selection = hit.map(|h| match h.key.kind {
            TopologyKind::Face => {
                if self.filter == PickFilter::Parts {
                    Selection::Part(h.instance)
                } else {
                    Selection::Face {
                        instance: h.instance,
                        ordinal: h.key.ordinal,
                    }
                }
            }
            TopologyKind::Edge => Selection::Edge {
                instance: h.instance,
                ordinal: h.key.ordinal,
            },
            TopologyKind::Vertex => Selection::Vertex {
                instance: h.instance,
                ordinal: h.key.ordinal,
            },
        });
        let selection = if self.parts.is_empty() {
            selection
        } else {
            Some(Selection::Parts(self.parts.clone()))
        };
        let bounds = scene.world_bounds();
        paint_scene_section(
            ui.painter(),
            response.rect,
            scene,
            camera,
            self.reference_highlight.clone().or(selection),
            style,
            self.section.plane(),
        );
        self.section
            .overlay(ui.painter(), camera, response.rect, bounds);
        if let Some(hit) = hit
            && hit.key.kind == TopologyKind::Vertex
            && self
                .section
                .plane()
                .is_none_or(|s| s.contains(hit.point_mm))
        {
            let p = camera.project(hit.point_mm.map(|v| v as f32), response.rect);
            ui.painter().circle_stroke(
                Pos2::new(p[0], p[1]),
                4.,
                Stroke::new(1.5, Color32::from_rgb(245, 184, 75)),
            );
        }
    }
}
#[cfg(test)]
mod tests;
