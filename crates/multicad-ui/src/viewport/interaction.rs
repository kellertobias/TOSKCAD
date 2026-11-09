use super::{
    Hit, PickFilter, PickOptions, Scene, SceneStamp, SectionPlane, SectionView, Selection,
    gpu::{View, paint_scene_view},
};
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
use std::{collections::BTreeSet, sync::Arc};
#[derive(Default)]
pub struct Interaction {
    selected: Option<Hit>,
    /// Command (macOS) / Control-click face set (CAD-152). Empty for a plain
    /// single selection; otherwise every face in click order, and `selected`
    /// is the last one so single-hit consumers keep a primary face.
    faces: Vec<Hit>,
    hovered: Option<Hit>,
    parts: Arc<BTreeSet<EntityId>>,
    member_faces: Arc<Vec<(EntityId, u32)>>,
    range_anchor: Option<EntityId>,
    pub filter: PickFilter,
    pub(crate) origin_axes: bool,
    pub section: SectionView,
    section_override: Option<Option<SectionPlane>>,
    reference_highlight: Option<Selection>,
    last_stamp: Option<SceneStamp>,
    display_scene: Option<Arc<Scene>>,
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
    /// A derived paint surface keeps the original native scene for hit/reference
    /// queries. A stale or foreign surface is never used by show().
    pub fn set_display_scene(&mut self, scene: Option<Arc<Scene>>) {
        self.display_scene = scene;
    }
    /// Explicit host clipping, including an unsectioned predecessor inspection.
    /// Other hosts retain their ordinary SectionView until they opt in.
    pub fn set_section_override(&mut self, section: Option<SectionPlane>) {
        self.section_override = Some(section);
    }
    fn effective_section(&self, derived: bool) -> Option<SectionPlane> {
        if derived {
            None
        } else {
            self.section_override
                .unwrap_or_else(|| self.section.plane())
        }
    }
    pub fn highlight_reference(&mut self, selection: Option<Selection>) {
        self.reference_highlight = selection;
    }
    pub fn clear(&mut self) {
        self.selected = None;
        self.faces.clear();
        self.hovered = None;
        Arc::make_mut(&mut self.parts).clear();
        self.member_faces = Arc::default();
        self.range_anchor = None;
        self.last_query = None;
    }
    pub fn has_topology_selection(&self) -> bool {
        self.selected.is_some()
    }
    pub fn selected_hit(&self) -> Option<&Hit> {
        self.selected.as_ref()
    }
    /// Every selected face: the Command/Control-click set, or the single
    /// selected face. Edges, vertices and part sets are not included.
    pub fn selected_faces(&self) -> Vec<Hit> {
        if !self.faces.is_empty() {
            return self.faces.clone();
        }
        self.selected
            .iter()
            .filter(|hit| hit.key.kind == TopologyKind::Face)
            .cloned()
            .collect()
    }
    /// Command/Control-click on a face adds it to the face set, or removes it
    /// when it is already selected. A current single face selection seeds
    /// the set; part sets are replaced.
    fn toggle_face(&mut self, hit: Hit) {
        if self.faces.is_empty()
            && let Some(current) = self
                .selected
                .take()
                .filter(|h| h.key.kind == TopologyKind::Face)
        {
            self.faces.push(current);
        }
        if let Some(index) = self
            .faces
            .iter()
            .position(|f| f.instance == hit.instance && f.key == hit.key)
        {
            self.faces.remove(index);
        } else {
            self.faces.push(hit);
        }
        self.selected = self.faces.last().cloned();
        Arc::make_mut(&mut self.parts).clear();
        self.range_anchor = None;
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
        ids.extend(self.faces.iter().map(|hit| hit.instance));
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
        self.member_faces = Arc::new(scene.member_faces(source, &selected));
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
        let faces: Vec<_> = self.faces.drain(..).map(|hit| hit.instance).collect();
        let parts = Arc::make_mut(&mut self.parts);
        if !toggle {
            parts.clear();
        } else {
            parts.extend(old);
            parts.extend(faces);
        }
        if !toggle || !parts.remove(&id) {
            parts.insert(id);
        }
        self.range_anchor = Some(id);
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
                    let response = ui.selectable_label(self.parts.contains(&e.id)||self.selected.as_ref().is_some_and(|h|h.instance==e.id), &e.name)
                        .on_hover_text("Select this part · Cmd/Ctrl-click toggles · Shift-click selects a range");
                    if response.clicked() {
                        self.member_faces = Arc::default();
                        let modifiers=crate::selection_input::click_modifiers(ui, &response);
                        if modifiers.shift && let Some(start)=self.range_anchor.and_then(|id|rows.iter().position(|row|row.id==id)) {
                            self.selected=None;
                            self.faces.clear();
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
        crate::controls::inspector_layout(ui);
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
        if self.faces.len() > 1 {
            ui.label(format!("{} faces selected", self.faces.len()));
            ui.small("Command/Control-click adds or removes a face · Escape clears");
        } else if let Some(hit) = &self.selected {
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
        let derived = self
            .display_scene
            .as_ref()
            .filter(|display| {
                display.stamp() == scene.stamp()
                    && display.model_evaluation().map(|info| info.source_hash)
                        == scene.model_evaluation().map(|info| info.source_hash)
            })
            .cloned();
        let section = self.effective_section(derived.is_some());
        if self.last_stamp != Some(scene.stamp()) {
            if self.last_stamp.is_some_and(|old| {
                old.document == scene.stamp().document && old.revision == scene.stamp().revision
            }) {
                self.selected = self
                    .selected
                    .as_ref()
                    .and_then(|hit| scene.refresh_hit(hit));
                if !self.faces.is_empty() {
                    // Display refinement of the same revision: keep faces that
                    // still resolve; the primary follows the remaining set.
                    self.faces = self
                        .faces
                        .iter()
                        .filter_map(|hit| scene.refresh_hit(hit))
                        .collect();
                    self.selected = self.faces.last().cloned();
                }
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
            let query = (pointer, response.rect, camera, style, self.filter, section);
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
                        section,
                    },
                );
                self.last_query = Some(query);
            }
            if response.clicked() {
                self.member_faces = Arc::default();
                let modifiers = crate::selection_input::click_modifiers(ui, response);
                if let Some(hit) = self.hovered.clone()
                    && modifiers.command
                    && !modifiers.shift
                    && self.filter != PickFilter::Parts
                    && hit.key.kind == TopologyKind::Face
                {
                    // CAD-152: Command (macOS) / Control toggles faces.
                    self.toggle_face(hit);
                } else if let Some(hit) = self.hovered.clone()
                    && (modifiers.command || modifiers.shift || self.filter == PickFilter::Parts)
                {
                    self.select_part(hit.instance, modifiers.command || modifiers.shift);
                    // The host captures the actual clicked occurrence even when
                    // a Parts/modifier click stores a set rather than a face.
                    self.hovered = Some(hit);
                } else {
                    self.selected = self.hovered.clone();
                    self.faces.clear();
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
        let selection =
            if !self.member_faces.is_empty() {
                Some(Selection::PartsAndFaces {
                    parts: self.parts.clone(),
                    faces: self.member_faces.clone(),
                })
            } else if !self.faces.is_empty() {
                let mut faces: Vec<_> = self
                    .faces
                    .iter()
                    .map(|hit| (hit.instance, hit.key.ordinal))
                    .collect();
                if let Some(hover) = self.hovered.as_ref().filter(|h| {
                    h.key.kind == TopologyKind::Face && self.filter != PickFilter::Parts
                }) && !faces.contains(&(hover.instance, hover.key.ordinal))
                {
                    faces.push((hover.instance, hover.key.ordinal));
                }
                Some(Selection::Faces(Arc::new(faces)))
            } else if self.parts.is_empty() {
                selection
            } else {
                Some(Selection::Parts(self.parts.clone()))
            };
        let bounds = scene.world_bounds();
        let display = derived.clone().unwrap_or_else(|| scene.clone());
        paint_scene_view(
            ui.painter(),
            response.rect,
            display,
            View {
                camera,
                size: response.rect.size(),
                selection: self.reference_highlight.clone().or(selection),
                style,
                section,
                origin_axes: self.origin_axes,
            },
        );
        if derived.is_none() {
            self.section
                .overlay(ui.painter(), camera, response.rect, bounds);
        }
        if let Some(hit) = hit
            && hit.key.kind == TopologyKind::Vertex
            && section.is_none_or(|s| s.contains(hit.point_mm))
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
