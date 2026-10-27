use super::{Hit, PickFilter, Scene, SceneStamp, Selection, paint_scene};
use crate::{
    camera::Camera,
    icons::{self, Icon},
    navigation::DisplayStyle,
};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    references::TopologyKind,
};
use std::sync::Arc;
#[derive(Default)]
pub struct Interaction {
    selected: Option<Hit>,
    hovered: Option<Hit>,
    part: Option<EntityId>,
    pub filter: PickFilter,
    last_stamp: Option<SceneStamp>,
    last_query: Option<(Pos2, Rect, Camera, DisplayStyle, PickFilter)>,
}
impl Interaction {
    pub fn clear(&mut self) {
        self.selected = None;
        self.hovered = None;
        self.part = None;
        self.last_query = None;
    }
    pub fn selected_id(&self) -> Option<EntityId> {
        self.part
            .or_else(|| self.selected.as_ref().map(|h| h.instance))
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
                        .selectable_label(self.selected_id() == Some(e.id), &e.name)
                        .on_hover_text("Select this part instance")
                        .clicked()
                    {
                        self.clear();
                        self.part = Some(e.id);
                        changed = true;
                    }
                }
            });
        changed
    }
    pub fn inspector(&self, ui: &mut egui::Ui, document: &Document) {
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
                self.part = self
                    .part
                    .filter(|id| scene.instances().iter().any(|i| i.id == *id));
                self.hovered = None;
                self.last_query = None;
            } else {
                self.clear();
            }
            self.last_stamp = Some(scene.stamp());
        }
        if enabled
            && response.hovered()
            && let Some(pointer) = response.hover_pos()
        {
            let query = (pointer, response.rect, camera, style, self.filter);
            if self.last_query != Some(query) {
                self.hovered = scene.pick_screen(
                    camera,
                    response.rect,
                    pointer,
                    self.filter,
                    matches!(style, DisplayStyle::Wire | DisplayStyle::AllEdges),
                );
                self.last_query = Some(query);
            }
            if response.clicked() {
                self.selected = self.hovered.clone();
                self.part = None;
            }
        } else {
            self.hovered = None;
            self.last_query = None;
        }
        let hit = self.hovered.as_ref().or(self.selected.as_ref());
        let selection = hit
            .map(|h| match h.key.kind {
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
            })
            .or(self.part.map(Selection::Part));
        paint_scene(ui.painter(), response.rect, scene, camera, selection, style);
        if let Some(hit) = hit
            && hit.key.kind == TopologyKind::Vertex
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
