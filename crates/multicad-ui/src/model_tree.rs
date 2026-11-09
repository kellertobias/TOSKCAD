//! Reusable source hierarchy tree and Inspector tools. Collapse and selection are
//! host/UI state; model edits use the same prepared commands as external callers.
use crate::{
    egui::{self, Color32},
    icons::{self, Icon},
};
use multicad_core::{
    commands::{Actor, Change, Channel, Session},
    document::{Document, EntityData, EntityId, Placement},
    hierarchy::{self, Action, ContainerKind, Index},
};
use std::collections::{BTreeMap, BTreeSet};
mod presentation;
use presentation::{Catalog, Presentation};
#[derive(Clone, Copy)]
pub struct TreeOptions {
    pub enabled: bool,
    pub allow_open_sketch: bool,
}
impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_open_sketch: true,
        }
    }
}
#[derive(Default)]
pub struct TreeEvents {
    pub selection: Option<BTreeSet<EntityId>>,
    pub open_sketch: Option<EntityId>,
    pub change: Option<Change>,
    pub message: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Root,
    Component,
    Group,
    Body,
    Sketch,
    Instance,
    Plane,
    Feature,
}
impl NodeKind {
    fn label(self) -> &'static str {
        match self {
            Self::Root => "Root",
            Self::Component => "Component",
            Self::Group => "Group",
            Self::Body => "Part",
            Self::Sketch => "Sketch",
            Self::Instance => "Instance",
            Self::Plane => "Plane",
            Self::Feature => "Feature",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Self::Root => Icon::Home,
            Self::Component => Icon::Assembly,
            Self::Body => Icon::Box,
            Self::Group => Icon::Library,
            Self::Sketch => Icon::Sketch,
            Self::Instance => Icon::Link,
            Self::Plane => Icon::Grid,
            Self::Feature => Icon::Cut,
        }
    }
    fn container(self) -> bool {
        matches!(self, Self::Root | Self::Component | Self::Group)
    }
}
fn kind(data: &EntityData) -> Option<NodeKind> {
    match data {
        EntityData::Assembly { .. } => Some(NodeKind::Component),
        EntityData::Group {} => Some(NodeKind::Group),
        EntityData::Part { .. } => Some(NodeKind::Body),
        EntityData::Sketch { .. } => Some(NodeKind::Sketch),
        EntityData::Instance { .. } => Some(NodeKind::Instance),
        EntityData::Plane { .. } => Some(NodeKind::Plane),
        EntityData::Feature { .. } => Some(NodeKind::Feature),
        _ => None,
    }
}
#[derive(Clone)]
pub struct TreeRow {
    pub id: EntityId,
    pub depth: usize,
    pub name: String,
    pub kind: NodeKind,
    presentation: Presentation,
    pub owner: String,
    pub local_visible: bool,
    pub effective_visible: bool,
    pub active: bool,
    pub expandable: bool,
    pub expanded: bool,
    movable: bool,
    destination: bool,
    /// UI reference row; this is not a containment edge or ownership claim.
    pub history_of: Option<EntityId>,
    pub available: bool,
    key: egui::Id,
}
#[derive(Clone)]
struct Destination {
    id: Option<EntityId>,
    path: String,
}
struct TreeView {
    stamp: (EntityId, u64),
    rows: Vec<TreeRow>,
    destinations: Vec<Destination>,
    active: String,
    diagnostics: Vec<String>,
    occurrence_aliases: BTreeMap<EntityId, EntityId>,
}
pub(crate) fn container_path(
    document: &Document,
    index: &Index<'_>,
    id: Option<EntityId>,
) -> String {
    let mut path = vec![];
    let mut next = id;
    while let Some(id) = next {
        if id == document.id() {
            break;
        }
        let Some(entity) = document.get(id) else {
            break;
        };
        path.push(entity.name.clone());
        next = index.parent(id).ok().flatten();
    }
    path.push(document.name().into());
    path.reverse();
    path.join(" / ")
}
impl TreeView {
    fn build(
        session: &Session,
        expansion: &BTreeMap<EntityId, bool>,
        salt: egui::Id,
        show_sources: bool,
        raw_feature: Option<EntityId>,
    ) -> Result<Self, String> {
        let document = session.document();
        let revision = session.revision();
        let index = Index::new(document).map_err(|error| error.to_string())?;
        let catalog = Catalog::new(document)?;
        let part_features: BTreeSet<_> = document
            .entities()
            .flat_map(|entity| match &entity.data {
                EntityData::Part { features, .. } => features.as_slice(),
                _ => &[],
            })
            .copied()
            .collect();
        let symbol_owned = multicad_core::symbols::owned_entities_in_session(session)
            .map_err(|error| error.to_string())?;
        // A single same-name placement at the definition's own location is
        // the same item to the operator. Keep the canonical definition row
        // (which owns the children and history) and expose the raw placement
        // in Show sources. Repeated, relocated or renamed copies stay visible.
        let mut occurrences = BTreeMap::<EntityId, Vec<EntityId>>::new();
        for entity in document.entities() {
            if let EntityData::Instance { component, .. } = entity.data {
                occurrences.entry(component).or_default().push(entity.id);
            }
        }
        let occurrence_aliases: BTreeMap<_, _> = if show_sources {
            BTreeMap::new()
        } else {
            occurrences
                .into_iter()
                .filter_map(|(component, instances)| {
                    let [instance] = instances.as_slice() else {
                        return None;
                    };
                    let definition = document.get(component)?;
                    let occurrence = document.get(*instance)?;
                    let EntityData::Instance { placement, .. } = occurrence.data else {
                        return None;
                    };
                    let same_owner =
                        index.parent(*instance).ok()? == index.parent(component).ok()?;
                    (same_owner
                        && definition.name == occurrence.name
                        && placement == Placement::default()
                        && !symbol_owned.contains(instance)
                        && !symbol_owned.contains(&component)
                        && !document.hierarchy().hidden.contains(instance))
                    .then_some((*instance, component))
                })
                .collect()
        };
        let mut destinations = vec![Destination {
            id: None,
            path: format!("{} (root)", document.name()),
        }];
        for entity in document.entities() {
            if hierarchy::container_kind(&entity.data).is_some()
                && !symbol_owned.contains(&entity.id)
            {
                destinations.push(Destination {
                    id: Some(entity.id),
                    path: container_path(document, &index, Some(entity.id)),
                });
            }
        }
        destinations[1..].sort_by(|a, b| a.path.cmp(&b.path));
        let mut counts = BTreeMap::new();
        for target in &destinations {
            *counts.entry(target.path.clone()).or_insert(0usize) += 1;
        }
        for target in &mut destinations {
            if counts[&target.path] > 1
                && let Some(id) = target.id
            {
                target.path = format!("{} [{}]", target.path, &id.to_string()[..8]);
            }
        }
        let active = container_path(document, &index, document.hierarchy().active_component);
        let diagnostics = index.diagnostics().into_iter().map(|diagnostic| {
            let hierarchy::HierarchyDiagnostic::SharedLegacyInstance { entry, parents } = diagnostic;
            let name = document.get(entry).map(|entity| entity.name.as_str()).unwrap_or("Missing instance");
            let paths = parents.into_iter().map(|parent| container_path(document, &index, Some(parent))).collect::<Vec<_>>().join("; ");
            format!("{name} is shared by {paths}. Move it to choose one owner; its source has not been duplicated.")
        }).collect();
        let mut rows = vec![];
        let mut pending = vec![(document.id(), 0usize, salt.with(document.id()))];
        while let Some((id, depth, key)) = pending.pop() {
            if (!show_sources && symbol_owned.contains(&id)) || occurrence_aliases.contains_key(&id)
            {
                continue;
            }
            let root = id == document.id();
            let (name, node_kind) = if root {
                (document.name().to_owned(), NodeKind::Root)
            } else {
                let Some(entity) = document.get(id) else {
                    continue;
                };
                let Some(kind) = kind(&entity.data) else {
                    continue;
                };
                (entity.name.clone(), kind)
            };
            if !show_sources && matches!(node_kind, NodeKind::Plane | NodeKind::Feature) {
                continue;
            }
            if node_kind == NodeKind::Feature
                && part_features.contains(&id)
                && raw_feature != Some(id)
            {
                continue;
            }
            let presentation = catalog.row(document, id, node_kind);
            let children = if node_kind.container() {
                index
                    .children(if root { None } else { Some(id) })
                    .map_err(|error| error.to_string())?
            } else {
                vec![]
            };
            let owner = if root {
                "Document root".into()
            } else {
                match index.owning_component(id) {
                    Ok(owner) => container_path(document, &index, owner),
                    Err(_) => "Shared legacy ownership; move explicitly to resolve".into(),
                }
            };
            let local_visible = !document.hierarchy().hidden.contains(&id);
            let mut effective_visible = crate::viewport::source_visible(document, &index, id)?;
            if let Some(EntityData::Instance { component, .. }) =
                document.get(id).map(|entity| &entity.data)
            {
                effective_visible &= crate::viewport::source_visible(document, &index, *component)?;
            }
            let active = if root {
                document.hierarchy().active_component.is_none()
            } else {
                document.hierarchy().active_component == Some(id)
            };
            let history_part = presentation.part.filter(|_| node_kind == NodeKind::Body);
            let history = match history_part
                .and_then(|part| document.get(part))
                .map(|entity| &entity.data)
            {
                Some(EntityData::Part { features, .. }) => features.as_slice(),
                _ => &[],
            };
            let expandable = !children.is_empty() || !history.is_empty();
            let expanded = expandable
                && expansion
                    .get(&id)
                    .copied()
                    .unwrap_or(root || (history.is_empty() && children.len() <= 8));
            let editable = !symbol_owned.contains(&id);
            rows.push(TreeRow {
                id,
                depth,
                name,
                kind: node_kind,
                presentation,
                owner,
                local_visible,
                effective_visible,
                active,
                expandable,
                expanded,
                movable: !root && editable,
                destination: node_kind.container() && editable,
                history_of: None,
                available: true,
                key,
            });
            if expanded {
                for (position, feature) in history.iter().copied().enumerate() {
                    let entity = document.get(feature);
                    let available = entity
                        .is_some_and(|entity| matches!(entity.data, EntityData::Feature { .. }));
                    let name = match entity {
                        Some(_) if available => {
                            let operation = feature_operation(document, feature);
                            if position == 0 {
                                format!("Created by {operation}")
                            } else {
                                format!("{position}. {operation}")
                            }
                        }
                        Some(entity) => format!("{} · invalid feature reference", entity.name),
                        None => "Missing feature".into(),
                    };
                    let owner = index
                        .owning_component(feature)
                        .ok()
                        .map(|owner| container_path(document, &index, owner))
                        .unwrap_or_else(|| "Unavailable source ownership".into());
                    rows.push(TreeRow {
                        id: feature,
                        depth: depth + 1,
                        name,
                        kind: NodeKind::Feature,
                        presentation: catalog.row(document, feature, NodeKind::Feature),
                        owner,
                        local_visible: true,
                        effective_visible,
                        active: false,
                        expandable: false,
                        expanded: false,
                        movable: false,
                        destination: false,
                        history_of: history_part,
                        available,
                        key: key.with(("feature-history", position, feature)),
                    });
                }
                pending.extend(
                    children
                        .into_iter()
                        .rev()
                        .map(|child| (child, depth + 1, key.with(child))),
                );
            }
        }
        Ok(Self {
            stamp: (document.id(), revision),
            rows,
            destinations,
            active,
            diagnostics,
            occurrence_aliases,
        })
    }
}
#[derive(Clone)]
enum Tool {
    Create {
        document: EntityId,
        revision: u64,
        kind: ContainerKind,
        name: String,
        parent: Option<EntityId>,
    },
    Rename {
        document: EntityId,
        revision: u64,
        id: EntityId,
        name: String,
    },
}
impl Tool {
    fn document(&self) -> EntityId {
        match self {
            Self::Create { document, .. } | Self::Rename { document, .. } => *document,
        }
    }
    fn revision(&self) -> u64 {
        match self {
            Self::Create { revision, .. } | Self::Rename { revision, .. } => *revision,
        }
    }
}
pub struct ModelTree {
    salt: egui::Id,
    expansion: BTreeMap<EntityId, bool>,
    view: Option<TreeView>,
    anchor: Option<egui::Id>,
    tool: Option<Tool>,
    show_sources: bool,
    raw_feature: Option<EntityId>,
    history_selection: Option<(EntityId, EntityId)>,
    selected_references: BTreeSet<EntityId>,
    scroll_to_source: Option<EntityId>,
}
impl Default for ModelTree {
    fn default() -> Self {
        Self {
            salt: egui::Id::new(EntityId::new()),
            expansion: BTreeMap::new(),
            view: None,
            anchor: None,
            tool: None,
            show_sources: false,
            raw_feature: None,
            history_selection: None,
            selected_references: BTreeSet::new(),
            scroll_to_source: None,
        }
    }
}
impl ModelTree {
    pub fn tool_active(&self) -> bool {
        self.tool.is_some()
    }
    pub fn cancel_tool(&mut self) {
        self.tool = None;
    }
    pub fn begin_create(
        &mut self,
        session: &Session,
        kind: ContainerKind,
        parent: Option<EntityId>,
    ) {
        self.tool = Some(Tool::Create {
            document: session.document().id(),
            revision: session.revision(),
            kind,
            name: match kind {
                ContainerKind::Component => "Assembly",
                ContainerKind::Group => "Group",
            }
            .into(),
            parent,
        });
    }
    pub fn begin_rename(&mut self, session: &Session, id: EntityId) -> Result<(), String> {
        let entity = session
            .document()
            .get(id)
            .ok_or("The selected container no longer exists")?;
        if hierarchy::container_kind(&entity.data).is_none() {
            return Err("Rename a component or group".into());
        }
        self.tool = Some(Tool::Rename {
            document: session.document().id(),
            revision: session.revision(),
            id,
            name: entity.name.clone(),
        });
        Ok(())
    }
    fn ensure_view(&mut self, session: &Session) -> Result<(), String> {
        let stamp = (session.document().id(), session.revision());
        if self.view.as_ref().is_none_or(|view| view.stamp != stamp) {
            self.view = Some(TreeView::build(
                session,
                &self.expansion,
                self.salt,
                self.show_sources,
                self.raw_feature,
            )?);
        }
        Ok(())
    }
    pub fn rows(&mut self, session: &Session) -> Result<&[TreeRow], String> {
        self.ensure_view(session)?;
        Ok(&self.view.as_ref().unwrap().rows)
    }
    pub fn set_show_sources(&mut self, visible: bool) {
        if self.show_sources != visible {
            self.show_sources = visible;
            self.raw_feature = None;
            self.view = None;
        }
    }
    fn locate_source(&mut self, document: &Document, id: EntityId) {
        self.set_show_sources(true);
        self.raw_feature = Some(id);
        self.history_selection = None;
        self.selected_references.remove(&id);
        self.scroll_to_source = Some(id);
        self.expansion.insert(document.id(), true);
        if let Ok(index) = Index::new(document) {
            let mut next = index.parent(id).ok().flatten();
            while let Some(parent) = next {
                self.expansion.insert(parent, true);
                next = index.parent(parent).ok().flatten();
            }
        }
        self.view = None;
    }
    pub fn set_expanded(&mut self, id: EntityId, expanded: bool) {
        self.expansion.insert(id, expanded);
        self.view = None;
    }
    fn execute(&mut self, session: &mut Session, revision: u64, action: Action) -> TreeEvents {
        let mut events = TreeEvents::default();
        let deleting = matches!(&action, Action::Delete { .. });
        match hierarchy::prepare(session, revision, Actor::local(Channel::Ui), action) {
            Ok(pending) => {
                let created = pending.created;
                match session.commit(pending.prepared) {
                    Ok(change) => {
                        events.message = Some(
                            session
                                .activity()
                                .last()
                                .map(|activity| activity.label.clone())
                                .unwrap_or_else(|| "Hierarchy unchanged".into()),
                        );
                        events.change = Some(change);
                        if let Some(id) = created {
                            events.selection = Some(BTreeSet::from([id]));
                        } else if deleting {
                            events.selection = Some(BTreeSet::new());
                            self.history_selection = None;
                            self.selected_references.clear();
                        }
                        self.view = None;
                    }
                    Err(error) => events.error = Some(error.to_string()),
                }
            }
            Err(error) => events.error = Some(error.to_string()),
        }
        events
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        selected: &BTreeSet<EntityId>,
        options: TreeOptions,
    ) -> TreeEvents {
        self.show_internal(ui, session, selected, options, false)
    }
    /// Select targets for a host-owned tool without changing the document or
    /// opening tree tools. The caller validates compatibility before accepting
    /// the selection. Shift and Cmd/Ctrl toggle individual entries here.
    pub fn show_targets(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        selected: &BTreeSet<EntityId>,
        options: TreeOptions,
    ) -> TreeEvents {
        self.show_internal(ui, session, selected, options, true)
    }
    fn show_internal(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        selected: &BTreeSet<EntityId>,
        options: TreeOptions,
        targets_only: bool,
    ) -> TreeEvents {
        let mutations_enabled = options.enabled && !targets_only;
        if let Err(error) = self.ensure_view(session) {
            return TreeEvents {
                error: Some(error),
                ..Default::default()
            };
        }
        let mut events = TreeEvents::default();
        let mut action = None;
        let mut create = None;
        let mut rename = None;
        let mut drop = None;
        let mut toggle = None;
        ui.horizontal(|ui| {
            ui.strong("Model");
            if ui
                .add_enabled(mutations_enabled, egui::Button::new("Create Assembly").small())
                .on_hover_text("Create an assembly in the active modeling scope; review its parent in the Inspector")
                .clicked()
            {
                create = Some((ContainerKind::Component, session.document().hierarchy().active_component));
            }
            let response = ui
                .add_enabled_ui(mutations_enabled, |ui| {
                    icons::compact_button(ui, Icon::New, "Create group")
                })
                .inner;
            egui::Popup::menu(&response).show(|ui| {
                if ui
                    .add_enabled(mutations_enabled, egui::Button::new("New group"))
                    .clicked()
                {
                    create = Some((ContainerKind::Group, session.document().hierarchy().active_component));
                    ui.close();
                }
            });
        });
        let mut show_sources = self.show_sources;
        if ui.checkbox(&mut show_sources, "Show sources").on_hover_text("Show underlying placements, datum planes, ungrouped features and private Library content. A part’s features are in its history.").changed() {
            self.set_show_sources(show_sources);
            if let Err(error) = self.ensure_view(session) {
                return TreeEvents { error: Some(error), ..Default::default() };
            }
        }
        let view = self.view.as_ref().unwrap();
        ui.add(
            egui::Label::new(egui::RichText::new(format!("Active: {}", view.active)).small())
                .wrap(),
        );
        for warning in &view.diagnostics {
            ui.collapsing("Legacy ownership needs attention", |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(warning).color(Color32::LIGHT_YELLOW))
                        .wrap(),
                );
            });
        }
        let rows = &view.rows;
        let represented: BTreeSet<_> = selected
            .iter()
            .filter_map(|id| view.occurrence_aliases.get(id))
            .copied()
            .collect();
        let mut scroll = egui::ScrollArea::vertical().id_salt(self.salt);
        if let Some(target) = self.scroll_to_source.take()
            && let Some(index) = rows
                .iter()
                .position(|row| row.id == target && row.history_of.is_none())
        {
            scroll =
                scroll.vertical_scroll_offset(index as f32 * (26. + ui.spacing().item_spacing.y));
        }
        scroll.show_rows(ui, 26., rows.len(), |ui, range| {
                for row_index in range {
                    let row = &rows[row_index];
                    // A row consumes exactly its parent's width. Child widget
                    // minimums must never feed a wider size back into the panel.
                    let (row_rect, drop_zone) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(0.), 26.), egui::Sense::hover());
                    let mut row_ui = ui.new_child(egui::UiBuilder::new().max_rect(row_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                    row_ui.set_clip_rect(row_rect.intersect(ui.clip_rect()));
                    row_ui.push_id(row.key, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space((row.depth as f32 * 10.).min(80.).min((row_rect.width() - 100.).max(0.)));
                            if row.expandable {
                                if disclosure(ui, row).clicked() {
                                    toggle = Some((row.id, !row.expanded));
                                }
                            } else {
                                ui.add_space(16.);
                            }
                            let icon = ui.add_enabled_ui(options.enabled && row.available, |ui| {
                                let link = if row.presentation.linked { "Library-linked " } else { "" };
                                icons::model_tree_icon(ui, row.presentation.icon, &format!("Select {link}{} {}", row.presentation.label, row.name), row.presentation.linked)
                            }).inner;
                            let trailing = ui.spacing().interact_size.y.max(24.);
                            let width = (ui.available_width() - trailing - 2. * ui.spacing().item_spacing.x).max(0.);
                            let label = if row.active { format!("{} · active", row.name) } else { row.name.clone() };
                            let color = if row.effective_visible { ui.visuals().text_color() } else { Color32::GRAY };
                            let draggable = mutations_enabled && row.movable;
                            let response = flat_row_label(ui, width, &label, options.enabled && row.available, selected.contains(&row.id) || represented.contains(&row.id), color, draggable);
                            if draggable && response.drag_started_by(egui::PointerButton::Primary) {
                                let entries = if selected.contains(&row.id) {
                                    rows.iter()
                                        .filter(|entry| entry.movable && selected.contains(&entry.id) && !self.selected_references.contains(&entry.id))
                                        .map(|entry| entry.id)
                                        .collect()
                                } else {
                                    vec![row.id]
                                };
                                response.dnd_set_drag_payload(TreeDrag {
                                    stamp: view.stamp,
                                    entries,
                                });
                            }
                            let role = if row.history_of.is_some() { "Feature history" } else { row.presentation.label };
                            let link_hint = if row.presentation.linked { "\nLinked Library definition" } else { "" };
                            let selection_hint = if targets_only { "Shift or Cmd/Ctrl-click toggles one target" } else { "Cmd/Ctrl-click toggles; Shift-click selects a range" };
                            let drag_hint = if draggable { "\nDrag to move or reorder" } else { "" };
                            response.clone().on_hover_text(format!("{role}{link_hint}\nOwning component: {}\n{selection_hint}{drag_hint}", row.owner));
                            if response.clicked() || icon.clicked() {
                                let modifiers = crate::selection_input::click_modifiers(ui, if response.clicked() { &response } else { &icon });
                                let toggle_one = modifiers.command || (targets_only && modifiers.shift);
                                let mut next = if toggle_one { selected.clone() } else { BTreeSet::new() };
                                if !targets_only && modifiers.shift && let Some(start) = self.anchor.and_then(|id| rows.iter().position(|row| row.key == id)) {
                                    next.extend(rows[start.min(row_index)..=start.max(row_index)].iter().filter(|row| row.available).map(|row| row.id));
                                } else if !toggle_one || !next.remove(&row.id) {
                                    next.insert(row.id);
                                }
                                self.selected_references.retain(|id| next.contains(id));
                                if !toggle_one {
                                    self.selected_references.clear();
                                }
                                if !targets_only && modifiers.shift && let Some(start) = self.anchor.and_then(|key| rows.iter().position(|row| row.key == key)) {
                                    self.selected_references.extend(rows[start.min(row_index)..=start.max(row_index)].iter().filter(|row| row.available && row.history_of.is_some()).map(|row| row.id));
                                } else if row.history_of.is_some() && next.contains(&row.id) {
                                    self.selected_references.insert(row.id);
                                } else {
                                    self.selected_references.remove(&row.id);
                                }
                                self.anchor = Some(row.key);
                                self.history_selection = row.history_of.map(|body| (row.id, body));
                                events.selection = Some(next);
                            }
                            if response.double_clicked() && mutations_enabled {
                                if row.kind == NodeKind::Sketch && options.allow_open_sketch {
                                    events.open_sketch = Some(row.id);
                                } else if matches!(row.kind, NodeKind::Component | NodeKind::Root) {
                                    action = Some(Action::SetActive { component: if row.kind == NodeKind::Root { None } else { Some(row.id) } });
                                }
                            }
                            if row.history_of.is_some() {
                                ui.add_space(trailing);
                            } else {
                            let mut visible = row.local_visible;
                            let visibility = ui.add_enabled(mutations_enabled && !matches!(row.kind, NodeKind::Plane | NodeKind::Feature),
                                egui::Checkbox::new(&mut visible, "").indeterminate(row.local_visible && !row.effective_visible));
                            let hint = if matches!(row.kind, NodeKind::Plane | NodeKind::Feature) {
                                "This source is evaluated through its sketch or body; standalone visibility is unavailable"
                            } else if row.local_visible && !row.effective_visible {
                                "Local visibility is on; an ancestor or the component definition is hidden"
                            } else {
                                "Show or hide this source; descendant visibility is inherited"
                            };
                            if visibility.on_hover_text(hint).changed() {
                                action = Some(Action::SetVisible { id: row.id, visible });
                            }
                            }
                            response.union(icon).context_menu(|ui| {
                                if row.history_of.is_some() {
                                    ui.label("Feature used by part");
                                    ui.small("Feature history refers to the operation that produced this body.");
                                    let _ = delete_menu(ui, session, row.id, false, Some("Select the body to delete it, or reopen the feature to edit its controls."));
                                    return;
                                }
                                ui.add_enabled_ui(mutations_enabled && row.available, |ui| {
                                    if matches!(row.kind, NodeKind::Root | NodeKind::Component)
                                        && ui.button("Activate modeling scope").clicked() {
                                        action = Some(Action::SetActive { component: if row.kind == NodeKind::Root { None } else { Some(row.id) } });
                                        ui.close();
                                    }
                                    if row.kind.container() {
                                        for (kind, label) in [(ContainerKind::Component, "Create Assembly"), (ContainerKind::Group, "New child group")] {
                                            if ui.button(label).clicked() {
                                                create = Some((kind, if row.kind == NodeKind::Root { None } else { Some(row.id) }));
                                                ui.close();
                                            }
                                        }
                                    }
                                    if matches!(row.kind, NodeKind::Component | NodeKind::Group) && ui.button("Rename").clicked() {
                                        rename = Some(row.id);
                                        ui.close();
                                    }
                                    if row.kind == NodeKind::Sketch && options.allow_open_sketch && ui.button("Open sketch").clicked() {
                                        events.open_sketch = Some(row.id);
                                        ui.close();
                                    }
                                    let legacy = row.kind == NodeKind::Component
                                        && matches!(&session.document().get(row.id).unwrap().data, EntityData::Assembly { instances } if !instances.is_empty());
                                    if legacy && ui.button("Normalize legacy membership")
                                        .on_hover_text("Preserve IDs and order; shared legacy instances must be moved explicitly first").clicked() {
                                        action = Some(Action::NormalizeLegacy { component: row.id });
                                        ui.close();
                                    }
                                    if let Some(delete) = delete_menu(ui, session, row.id, mutations_enabled, None) {
                                        action = Some(delete);
                                    }
                                });
                            });
                        });
                    });
                    if mutations_enabled
                        && let Some(payload) = drop_zone.dnd_hover_payload::<TreeDrag>()
                        && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
                        && view.stamp == payload.stamp
                        && !payload.entries.is_empty()
                    {
                        let target = DropPosition::at(row, row_rect, pointer.y);
                        if let Some(target) = target.filter(|_| !payload.entries.contains(&row.id)) {
                            target.paint(ui, row_rect);
                        } else {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                        }
                        if ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary)) {
                            drop = drop_zone.dnd_release_payload::<TreeDrag>().map(|payload| (row.id, target, payload));
                        }
                    }
                }
            });
        if let Some((id, expanded)) = toggle {
            self.set_expanded(id, expanded);
        }
        if let Some((kind, parent)) = create {
            self.begin_create(session, kind, parent);
        }
        if let Some(id) = rename
            && let Err(error) = self.begin_rename(session, id)
        {
            events.error = Some(error);
        }
        if let Some((row, target, payload)) = drop {
            return self.drop_entries(session, &payload, row, target);
        }
        if let Some(action) = action {
            let mut result = self.execute(session, session.revision(), action);
            if result.selection.is_none() {
                result.selection = events.selection;
            }
            return result;
        }
        events
    }
    fn drop_entries(
        &mut self,
        session: &mut Session,
        payload: &TreeDrag,
        target: EntityId,
        position: Option<DropPosition>,
    ) -> TreeEvents {
        let document = session.document();
        let result = (|| {
            if payload.stamp != (document.id(), session.revision()) {
                return Err("The model changed during the drag. Drag the entries again.".into());
            }
            let position = position.ok_or("Drop onto a container or between model entries.")?;
            if payload.entries.contains(&target) {
                return Err("Choose a different drop target.".into());
            }
            let (parent, before) = match position {
                DropPosition::Into => ((target != document.id()).then_some(target), None),
                DropPosition::Before | DropPosition::After => {
                    let index = Index::new(document).map_err(|error| error.to_string())?;
                    let parent = index.parent(target).map_err(|error| error.to_string())?;
                    let before = if position == DropPosition::Before {
                        Some(target)
                    } else {
                        index
                            .children(parent)
                            .map_err(|error| error.to_string())?
                            .into_iter()
                            .skip_while(|id| *id != target)
                            .skip(1)
                            .find(|id| !payload.entries.contains(id))
                    };
                    (parent, before)
                }
            };
            Ok((parent, before, position))
        })();
        let (parent, before, position) = match result {
            Ok(target) => target,
            Err(error) => {
                return TreeEvents {
                    error: Some(error),
                    ..Default::default()
                };
            }
        };
        let mut events = self.execute(
            session,
            payload.stamp.1,
            Action::Move {
                entries: payload.entries.clone(),
                parent,
                before,
            },
        );
        if events.change.is_some() {
            self.history_selection = None;
            self.selected_references.clear();
            if position == DropPosition::Into {
                self.set_expanded(target, true);
            }
            events.selection = Some(payload.entries.iter().copied().collect());
        }
        events
    }

    pub fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        selected: &BTreeSet<EntityId>,
        options: TreeOptions,
    ) -> TreeEvents {
        crate::controls::inspector_layout(ui);
        let enabled = options.enabled;
        if let Err(error) = self.ensure_view(session) {
            return TreeEvents {
                error: Some(error),
                ..Default::default()
            };
        }
        let mut events = TreeEvents::default();
        if let Some(tool) = self.tool.as_mut() {
            let current =
                tool.revision() == session.revision() && tool.document() == session.document().id();
            let mut apply = false;
            let mut cancel = false;
            let view = self.view.as_ref().unwrap();
            ui.add_enabled_ui(enabled, |ui| {
                match tool {
                    Tool::Create { kind, name, parent, .. } => {
                        ui.strong(match kind { ContainerKind::Component => "Create Assembly", ContainerKind::Group => "New group" });
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)).on_hover_text("Name for the new model container");
                        parent_choice(ui, "create-parent", parent, &view.destinations);
                    }
                    Tool::Rename { name, .. } => {
                        ui.strong("Rename container");
                        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)).on_hover_text("New component or group name; identity and references remain stable");
                    }
                }
                if !current {
                    ui.colored_label(Color32::LIGHT_RED, "The document changed. Cancel and reopen this tool to review the current state.");
                }
                use crate::inspector_actions::{self,Button,Owner};
                let clicked=inspector_actions::row(ui,Owner::Tree,None,
                    Button::new("Apply",current,"Commit one undoable hierarchy transaction"),
                    Button::new("Cancel",true,"Discard this source-neutral form"));
                apply=clicked.apply;cancel=clicked.cancel;
            });
            if cancel {
                self.tool = None;
            } else if apply {
                return self.apply_tool(session);
            }
            return events;
        }
        let Some(id) = selected.first().copied() else {
            return events;
        };
        let root = id == session.document().id();
        let entity = session.document().get(id);
        let node_kind = if root {
            Some(NodeKind::Root)
        } else {
            entity.and_then(|entity| kind(&entity.data))
        };
        let Some(node_kind) = node_kind else {
            return events;
        };
        let (role, linked) = self
            .view
            .as_ref()
            .and_then(|view| view.rows.iter().find(|row| row.id == id))
            .map_or((node_kind.label(), false), |row| {
                (row.presentation.label, row.presentation.linked)
            });
        let history_body = self
            .history_selection
            .filter(|(feature, _)| *feature == id)
            .and_then(|(_, body)| session.document().get(body))
            .map(|body| body.name.clone());
        let mut action = None;
        let mut rename = false;
        let summary = ui
            .scope(|ui| {
                ui.strong(if root {
                    session.document().name()
                } else {
                    &entity.unwrap().name
                });
                ui.label(if linked {
                    format!("Library-linked {role}")
                } else {
                    role.to_owned()
                });
                if let Ok(index) = Index::new(session.document()) {
                    match index.owning_component(id) {
                        Ok(owner) => {
                            ui.label(format!(
                                "Owning component: {}",
                                container_path(session.document(), &index, owner)
                            ));
                        }
                        Err(error) => {
                            ui.colored_label(Color32::LIGHT_YELLOW, error.to_string());
                        }
                    }
                    if let Some(EntityData::Instance { component, .. }) =
                        entity.map(|entity| &entity.data)
                    {
                        ui.label(format!(
                            "Definition: {}",
                            session
                                .document()
                                .get(*component)
                                .map(|entity| entity.name.as_str())
                                .unwrap_or("Missing source")
                        ));
                    }
                }
                if let Some(EntityData::Part { material, tags, .. }) =
                    entity.map(|entity| &entity.data)
                {
                    if let Some(material) = material.and_then(|id| session.document().get(id)) {
                        ui.label(format!("Material: {}", material.name));
                    }
                    if !tags.is_empty() {
                        ui.label(format!(
                            "Tags: {}",
                            tags.iter().cloned().collect::<Vec<_>>().join(", ")
                        ));
                    }
                }
                if let Some(body) = &history_body {
                    ui.label(format!("Feature used by part: {body}"));
                    ui.small(
                        "This entry opens the saved feature that created or modified the part.",
                    );
                    if ui
                        .button("Show source entry")
                        .on_hover_text("Reveal this feature under its actual owning container")
                        .clicked()
                    {
                        self.locate_source(session.document(), id);
                    }
                }
                ui.add_enabled_ui(enabled, |ui| {
                    if matches!(node_kind, NodeKind::Root | NodeKind::Component)
                        && ui
                            .button("Activate modeling scope")
                            .on_hover_text("Use this component for new model geometry")
                            .clicked()
                    {
                        action = Some(Action::SetActive {
                            component: (!root).then_some(id),
                        });
                    }
                    if matches!(node_kind, NodeKind::Component | NodeKind::Group) {
                        rename = ui.button("Rename…").clicked();
                    }
                    if node_kind == NodeKind::Sketch
                        && options.allow_open_sketch
                        && ui
                            .button("Open selected sketch")
                            .on_hover_text("Open this sketch in the Sketch workspace")
                            .clicked()
                    {
                        events.open_sketch = Some(id);
                    }
                });
            })
            .response;
        let secondary_click = ui.rect_contains_pointer(summary.rect)
            && ui.input(|input| input.pointer.button_clicked(egui::PointerButton::Secondary));
        egui::Popup::context_menu(&summary)
            .id(self.salt.with(("inspector-delete", id)))
            .open_memory(secondary_click.then(|| true.into()))
            .show(|ui| {
                let reason = if history_body.is_some() {
                    Some(
                        "Select the body to delete it, or reopen the feature to edit its controls.",
                    )
                } else if selected.len() > 1 {
                    Some("Select one entity to delete it.")
                } else {
                    None
                };
                if let Some(delete) = delete_menu(ui, session, id, enabled, reason) {
                    action = Some(delete);
                }
            });
        if rename && let Err(error) = self.begin_rename(session, id) {
            events.error = Some(error);
        }
        if let Some(action) = action {
            return self.execute(session, session.revision(), action);
        }
        events
    }
    fn apply_tool(&mut self, session: &mut Session) -> TreeEvents {
        let Some(tool) = self.tool.clone() else {
            return TreeEvents::default();
        };
        if tool.document() != session.document().id() {
            return TreeEvents {
                error: Some(
                    "This hierarchy tool belongs to another document; cancel and reopen it".into(),
                ),
                ..Default::default()
            };
        }
        let revision = tool.revision();
        let action = match tool {
            Tool::Create {
                kind, name, parent, ..
            } => Action::Create { kind, name, parent },
            Tool::Rename { id, name, .. } => Action::Rename { id, name },
        };
        let events = self.execute(session, revision, action);
        if events.error.is_none() {
            self.tool = None;
        }
        events
    }
}
#[derive(Clone)]
struct TreeDrag {
    stamp: (EntityId, u64),
    entries: Vec<EntityId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DropPosition {
    Into,
    Before,
    After,
}

impl DropPosition {
    fn at(row: &TreeRow, rect: egui::Rect, y: f32) -> Option<Self> {
        if row.kind == NodeKind::Root
            || (row.destination && y > rect.top() + 6. && y < rect.bottom() - 6.)
        {
            Some(Self::Into)
        } else if row.movable {
            Some(if y < rect.center().y {
                Self::Before
            } else {
                Self::After
            })
        } else {
            None
        }
    }

    fn paint(self, ui: &egui::Ui, rect: egui::Rect) {
        let color = ui.visuals().selection.stroke.color;
        let stroke = egui::Stroke::new(2., color);
        match self {
            Self::Into => {
                ui.painter()
                    .rect_stroke(rect.shrink(1.), 3., stroke, egui::StrokeKind::Inside);
            }
            Self::Before | Self::After => {
                let y = if self == Self::Before {
                    rect.top() + 1.
                } else {
                    rect.bottom() - 1.
                };
                ui.painter().hline(rect.x_range(), y, stroke);
            }
        }
    }
}

fn disclosure(ui: &mut egui::Ui, row: &TreeRow) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16., 24.), egui::Sense::click());
    let verb = if row.expanded { "Collapse" } else { "Expand" };
    let label = if row.presentation.part.is_some() {
        format!("{verb} feature history for {}", row.name)
    } else {
        format!("{verb} {}", row.name)
    };
    let visuals = ui.style().interact(&response);
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 3., visuals.weak_bg_fill);
    }
    let chevron = if row.expanded {
        [[-4., -2.], [0., 2.], [4., -2.]]
    } else {
        [[-2., -4.], [2., 0.], [-2., 4.]]
    };
    ui.painter().add(egui::Shape::line(
        chevron
            .into_iter()
            .map(|[x, y]| rect.center() + egui::vec2(x, y))
            .collect(),
        visuals.fg_stroke,
    ));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    response.on_hover_text(label)
}

fn delete_menu(
    ui: &mut egui::Ui,
    session: &Session,
    id: EntityId,
    enabled: bool,
    reason: Option<&str>,
) -> Option<Action> {
    let availability = match reason {
        Some(reason) => Err(reason.to_owned()),
        None if !enabled => Err("Finish the current tool before deleting an entity.".into()),
        None => hierarchy::deletion_available(session, id),
    };
    if ui
        .add_enabled(availability.is_ok(), egui::Button::new("Delete"))
        .clicked()
    {
        ui.close();
        return Some(Action::Delete { id });
    }
    if let Err(reason) = availability {
        ui.small(reason);
    }
    None
}

pub(super) fn feature_operation(document: &Document, feature: EntityId) -> String {
    use multicad_core::model::{self, PrimitiveParameters};
    if let Ok(primitive) = model::read_primitive(document, feature) {
        return match primitive.parameters {
            PrimitiveParameters::Box { .. } => "Box",
            PrimitiveParameters::Cylinder { .. } => "Cylinder",
            PrimitiveParameters::Sphere { .. } => "Sphere",
        }
        .into();
    }
    document.get(feature).map_or_else(
        || "feature".into(),
        |entity| {
            entity
                .name
                .strip_suffix(" feature")
                .unwrap_or(&entity.name)
                .to_owned()
        },
    )
}

fn flat_row_label(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    enabled: bool,
    selected: bool,
    color: Color32,
    draggable: bool,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, 24.),
        if draggable {
            egui::Sense::click_and_drag()
        } else if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if selected || response.hovered() && enabled {
        ui.painter().rect_filled(
            rect,
            3,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.weak_bg_fill
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.),
            3,
            egui::Stroke::new(1., ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
    let color = if enabled {
        color
    } else {
        ui.visuals().weak_text_color()
    };
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (width - 4.).max(0.),
        egui::TextStyle::Button,
    );
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(
            egui::pos2(rect.left() + 2., rect.center().y - galley.size().y * 0.5),
            galley,
            color,
        );
    response
}
fn parent_choice(
    ui: &mut egui::Ui,
    id: &str,
    parent: &mut Option<EntityId>,
    destinations: &[Destination],
) {
    ui.label("Parent container");
    let current = destinations
        .iter()
        .find(|entry| entry.id == *parent)
        .map(|entry| entry.path.as_str())
        .unwrap_or("Missing container");
    crate::controls::inspector_combo(ui, id)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for destination in destinations {
                ui.selectable_value(parent, destination.id, &destination.path);
            }
        })
        .response
        .on_hover_text(format!(
            "{current}\nFull model paths distinguish nested components and groups"
        ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::Request,
        document::{Entity, Mutation},
        hierarchy::Hierarchy,
    };
    #[test]
    fn private_symbol_graph_is_source_inspection_and_never_a_destination() {
        use multicad_core::{model, symbols::*};
        let actor = multicad_core::commands::Actor::local(Channel::Ui);
        let mut session = Session::new(Document::new("Symbol tree").unwrap()).unwrap();
        let stock = model::PrimitiveRequest::new(
            "Source",
            model::PrimitiveParameters::Box {
                size: [model::length(10.), model::length(20.), model::length(5.)],
                frame: Default::default(),
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, 0, actor.clone(), &stock)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let author =
            SymbolAuthorRequest::new("Pinned box", stock.body, SymbolKind::Static, vec![], None);
        session
            .commit(
                prepare_author(&session, session.revision(), actor.clone(), &author)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let place = SymbolPlaceRequest::new(
            session.document(),
            author.definition,
            "Placed box",
            Default::default(),
            Default::default(),
        )
        .unwrap();
        session
            .commit(
                prepare_place(&session, session.revision(), actor, &place)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let owned = owned_entities(session.document()).unwrap();
        let build = |show_sources| {
            TreeView::build(
                &session,
                &Default::default(),
                egui::Id::new("symbol tree"),
                show_sources,
                None,
            )
            .unwrap()
        };
        let normal = build(false);
        assert!(normal.rows.iter().any(|row| row.id == place.instance));
        assert!(!normal.rows.iter().any(|row| owned.contains(&row.id)));
        let sources = build(true);
        assert!(sources.rows.iter().any(|row| row.id == place.wrapper));
        assert!(
            sources
                .rows
                .iter()
                .any(|row| row.id == place.ids[&stock.body])
        );
        assert!(
            !sources
                .destinations
                .iter()
                .any(|destination| destination.id.is_some_and(|id| owned.contains(&id)))
        );
    }
    fn fixture() -> (Session, EntityId, EntityId, EntityId) {
        let component = Entity::new("Cabinet", EntityData::Assembly { instances: vec![] });
        let group = Entity::new("Panels", EntityData::Group {});
        let body = Entity::new(
            "Side",
            EntityData::Part {
                features: vec![],
                material: None,
                tags: BTreeSet::new(),
            },
        );
        let document = Document::new("Workshop")
            .unwrap()
            .edited(&[
                Mutation::Insert(component.clone()),
                Mutation::Insert(group.clone()),
                Mutation::Insert(body.clone()),
                Mutation::SetHierarchy(Hierarchy {
                    members: BTreeMap::from([
                        (component.id, vec![group.id]),
                        (group.id, vec![body.id]),
                    ]),
                    active_component: Some(component.id),
                    ..Default::default()
                }),
            ])
            .unwrap();
        (
            Session::new(document).unwrap(),
            component.id,
            group.id,
            body.id,
        )
    }
    #[test]
    fn recursive_rows_collapse_locally_and_report_scope_and_visibility() {
        let (mut session, component, group, body) = fixture();
        let mut tree = ModelTree::default();
        let before = session.document().clone();
        let rows = tree.rows(&session).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].id, body);
        assert_eq!(rows[3].depth, 3);
        assert_eq!(rows[3].owner, "Workshop / Cabinet");
        assert!(rows[1].active);
        tree.set_expanded(group, false);
        assert_eq!(tree.rows(&session).unwrap().len(), 3);
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
        tree.set_expanded(group, true);
        let event = tree.execute(
            &mut session,
            0,
            Action::SetVisible {
                id: component,
                visible: false,
            },
        );
        assert!(event.error.is_none());
        let row = tree
            .rows(&session)
            .unwrap()
            .iter()
            .find(|row| row.id == body)
            .unwrap();
        assert!(row.local_visible);
        assert!(!row.effective_visible);
        session.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert!(
            tree.rows(&session)
                .unwrap()
                .iter()
                .find(|row| row.id == body)
                .unwrap()
                .effective_visible
        );
    }
    #[test]
    fn inspector_forms_cancel_stale_guard_and_commit_one_typed_transaction() {
        let (mut session, component, group, body) = fixture();
        let mut tree = ModelTree::default();
        let before = session.document().clone();
        tree.begin_create(&session, ContainerKind::Group, Some(component));
        assert!(tree.tool_active());
        tree.cancel_tool();
        assert_eq!(session.document(), &before);
        tree.begin_rename(&session, group).unwrap();
        if let Some(Tool::Rename { name, .. }) = &mut tree.tool {
            *name = "Carcass panels".into();
        }
        let events = tree.apply_tool(&mut session);
        assert!(events.error.is_none());
        assert_eq!(session.revision(), 1);
        assert_eq!(
            session.document().get(group).unwrap().name,
            "Carcass panels"
        );
        assert!(!tree.tool_active());
        let root = session.document().id();
        let events = tree.drop_entries(
            &mut session,
            &TreeDrag {
                stamp: (root, 1),
                entries: vec![body],
            },
            root,
            Some(DropPosition::Into),
        );
        assert!(events.error.is_none());
        assert_eq!(
            Index::new(session.document())
                .unwrap()
                .owning_component(body)
                .unwrap(),
            None
        );
        session.undo(2, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(
            Index::new(session.document())
                .unwrap()
                .owning_component(body)
                .unwrap(),
            Some(component)
        );
        tree.begin_create(&session, ContainerKind::Component, None);
        let pending = session
            .prepare(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Change document".into(),
                edits: vec![Mutation::RenameDocument("Changed".into())],
            })
            .unwrap();
        session.commit(pending).unwrap();
        let snapshot = session.document().clone();
        let revision = session.revision();
        assert!(tree.apply_tool(&mut session).error.is_some());
        assert!(tree.tool_active());
        assert_eq!(session.document(), &snapshot);
        assert_eq!(session.revision(), revision);
        tree.cancel_tool();
    }
    #[test]
    fn duplicate_names_have_unambiguous_destination_paths_and_cycles_reject() {
        let (mut session, component, group, _) = fixture();
        let other = Entity::new("Cabinet", EntityData::Assembly { instances: vec![] });
        let prepared = session
            .prepare(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Ui),
                label: "Other component".into(),
                edits: vec![Mutation::Insert(other)],
            })
            .unwrap();
        session.commit(prepared).unwrap();
        let mut tree = ModelTree::default();
        tree.ensure_view(&session).unwrap();
        let paths: Vec<_> = tree
            .view
            .as_ref()
            .unwrap()
            .destinations
            .iter()
            .map(|target| &target.path)
            .collect();
        assert_eq!(paths.iter().collect::<BTreeSet<_>>().len(), paths.len());
        let before = session.document().clone();
        let payload = TreeDrag {
            stamp: (before.id(), session.revision()),
            entries: vec![component],
        };
        assert!(
            tree.drop_entries(&mut session, &payload, group, Some(DropPosition::Into))
                .error
                .is_some()
        );
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn headless_widget_build_preserves_source_and_empty_inspector() {
        let (mut session, _, _, _) = fixture();
        let before = session.document().clone();
        let mut tree = ModelTree::default();
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let events = tree.show(ui, &mut session, &BTreeSet::new(), TreeOptions::default());
            assert!(events.change.is_none());
            let result = tree.inspector(ui, &mut session, &BTreeSet::new(), TreeOptions::default());
            assert!(result.change.is_none());
        })
        .drop_without_applying_deltas();
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
    }
    #[test]
    fn form_cannot_target_another_document_at_the_same_revision() {
        let (session, _, _, _) = fixture();
        let mut tree = ModelTree::default();
        tree.begin_create(&session, ContainerKind::Group, None);
        let mut other = Session::new(Document::new("Other").unwrap()).unwrap();
        let before = other.document().clone();
        assert!(tree.apply_tool(&mut other).error.is_some());
        assert_eq!(other.document(), &before);
        assert!(tree.tool_active());
    }
    #[test]
    fn tree_pointer_selection_is_source_neutral_and_modifier_aware() {
        let (mut session, component, group, body) = fixture();
        let mut tree = ModelTree::default();
        let before = session.document().clone();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut selected = BTreeSet::new();
        let mut frame = |events: Vec<egui::Event>| {
            ctx.run_ui(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340., 480.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let event = tree.show(ui, &mut session, &selected, TreeOptions::default());
                    if let Some(next) = event.selection {
                        selected = next;
                    }
                    assert!(event.change.is_none());
                },
            )
        };
        frame(vec![]).drop_without_applying_deltas();
        let output = frame(vec![]);
        let ax = output.platform_output.accesskit_update.as_ref().unwrap();
        let points: Vec<_> = ["Cabinet · active", "Panels", "Side"]
            .iter()
            .map(|label| {
                let bounds = ax
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label))
                    .and_then(|(_, node)| node.bounds())
                    .unwrap();
                egui::Pos2::new(
                    ((bounds.x0 + bounds.x1) * 0.5) as f32,
                    ((bounds.y0 + bounds.y1) * 0.5) as f32,
                )
            })
            .collect();
        output.drop_without_applying_deltas();
        for (index, modifiers) in [
            (0, egui::Modifiers::NONE),
            (1, egui::Modifiers::COMMAND),
            (2, egui::Modifiers::SHIFT),
        ] {
            for pressed in [true, false] {
                let pos = points[index];
                frame(vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ])
                .drop_without_applying_deltas();
            }
        }
        assert_eq!(selected, BTreeSet::from([group, body]));
        assert!(!selected.contains(&component));
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
    }
    /// Paint the production tree and Inspector together: every create action
    /// below is dispatched by a real pointer event, not by opening its tool.
    struct AssemblyTreeHarness {
        context: egui::Context,
        tree: ModelTree,
        session: Session,
        selected: BTreeSet<EntityId>,
    }
    impl AssemblyTreeHarness {
        fn new(session: Session) -> Self {
            let context = egui::Context::default();
            context.enable_accesskit();
            Self {
                context,
                tree: ModelTree::default(),
                session,
                selected: BTreeSet::new(),
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            self.context.run_ui(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 700.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::Panel::left("assembly-tree")
                        .exact_size(225.)
                        .show(ui, |ui| {
                            let event = self.tree.show(
                                ui,
                                &mut self.session,
                                &self.selected,
                                TreeOptions::default(),
                            );
                            assert!(event.error.is_none(), "{:?}", event.error);
                            if let Some(selected) = event.selection {
                                self.selected = selected;
                            }
                        });
                    egui::Panel::right("assembly-inspector")
                        .exact_size(250.)
                        .show(ui, |ui| {
                            let event = self.tree.inspector(
                                ui,
                                &mut self.session,
                                &self.selected,
                                TreeOptions::default(),
                            );
                            assert!(event.error.is_none(), "{:?}", event.error);
                            if let Some(selected) = event.selection {
                                self.selected = selected;
                            }
                        });
                    egui::CentralPanel::default().show(ui, |_| {});
                },
            )
        }
        fn bounds(&mut self, label: &str, last: bool) -> egui::Rect {
            self.frame(vec![]).drop_without_applying_deltas();
            let output = self.frame(vec![]);
            fn painted(shape: &egui::Shape, label: &str, matches: &mut Vec<egui::Rect>) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        matches.push(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            painted(shape, label, matches);
                        }
                    }
                    _ => {}
                }
            }
            let mut matches = vec![];
            for shape in &output.shapes {
                painted(&shape.shape, label, &mut matches);
            }
            let rect = if let Some(rect) = if last {
                matches.last()
            } else {
                matches.first()
            } {
                *rect
            } else {
                let ax = output.platform_output.accesskit_update.as_ref().unwrap();
                let mut matches = ax
                    .nodes
                    .iter()
                    .filter(|(_, node)| node.label() == Some(label));
                let (_, node) = if last {
                    matches.next_back()
                } else {
                    matches.next()
                }
                .unwrap_or_else(|| panic!("Missing painted {label}"));
                let bounds = node.bounds().unwrap();
                egui::Rect::from_min_max(
                    egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                    egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                )
            };
            output.drop_without_applying_deltas();
            rect
        }
        fn pointer(&mut self, pos: egui::Pos2, button: egui::PointerButton) {
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ])
                .drop_without_applying_deltas();
            }
        }
        fn drag(&mut self, start: egui::Pos2, end: egui::Pos2) {
            self.frame(vec![egui::Event::PointerMoved(start)])
                .drop_without_applying_deltas();
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![button(start, true)])
                .drop_without_applying_deltas();
            self.frame(vec![egui::Event::PointerMoved(end)])
                .drop_without_applying_deltas();
            self.frame(vec![button(end, false)])
                .drop_without_applying_deltas();
        }
        fn click(&mut self, label: &str, last: bool) {
            let pos = self.bounds(label, last).center();
            self.pointer(pos, egui::PointerButton::Primary);
        }
        fn draft_parent(&self) -> Option<EntityId> {
            match self.tree.tool.as_ref().expect("create form") {
                Tool::Create {
                    kind: ContainerKind::Component,
                    name,
                    parent,
                    ..
                } => {
                    assert_eq!(name, "Assembly");
                    *parent
                }
                _ => panic!("expected canonical assembly create"),
            }
        }
    }
    #[test]
    fn painted_tree_drags_between_containers_and_reorders_with_undo() {
        let (session, _, group, body) = fixture();
        let sibling = Entity::new(
            "Brace",
            EntityData::Part {
                features: vec![],
                material: None,
                tags: BTreeSet::new(),
            },
        );
        let document = session
            .document()
            .edited(&[Mutation::Insert(sibling.clone())])
            .unwrap();
        let mut h = AssemblyTreeHarness::new(Session::new(document.clone()).unwrap());

        let from = h.bounds("Side", false).center();
        let root = h.bounds("Workshop", false).center();
        h.drag(from, root);
        assert_eq!(h.session.revision(), 1);
        assert_eq!(
            Index::new(h.session.document())
                .unwrap()
                .parent(body)
                .unwrap(),
            None
        );
        assert_eq!(h.session.document().get(body), document.get(body));

        let from = h.bounds("Side", false).center();
        let before = h.bounds("Brace", false).center() - egui::vec2(0., 8.);
        h.drag(from, before);
        assert_eq!(h.session.revision(), 2);
        let children = Index::new(h.session.document())
            .unwrap()
            .children(None)
            .unwrap();
        assert!(children.windows(2).any(|ids| ids == [body, sibling.id]));
        h.session.undo(2, Actor::local(Channel::Ui)).unwrap();
        h.session.undo(3, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(
            Index::new(h.session.document())
                .unwrap()
                .parent(body)
                .unwrap(),
            Some(group)
        );
        assert_eq!(h.session.document(), &document);
    }

    #[test]
    fn painted_tree_and_inspector_context_menus_offer_transactional_delete() {
        let (session, _, group, body) = fixture();
        let mut h = AssemblyTreeHarness::new(session);
        let group_row = h.bounds("Panels", false).center();
        h.pointer(group_row, egui::PointerButton::Secondary);
        h.click("Delete", false);
        assert!(h.session.document().get(group).is_some());
        assert_eq!(h.session.revision(), 0);
        h.frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }])
        .drop_without_applying_deltas();

        h.click("Side", false);
        let summary = h.bounds("Side", true).center();
        assert!(
            summary.x > 850.,
            "expected the Inspector title, got {summary:?}"
        );
        h.pointer(summary, egui::PointerButton::Secondary);
        h.click("Delete", false);
        assert!(h.session.document().get(body).is_none());
        assert!(h.selected.is_empty());
        h.session.undo(1, Actor::local(Channel::Ui)).unwrap();
        let row = h.bounds("Side", false).center();
        h.pointer(row, egui::PointerButton::Secondary);
        h.click("Delete", false);
        assert!(h.session.document().get(body).is_none());
        assert_eq!(h.session.revision(), 3);
    }

    #[test]
    fn painted_create_assembly_preserves_active_scope_cancel_root_choice_history_and_reopen() {
        use multicad_core::project_file::{self, FileLimits, ProjectFile};
        let (session, component, _, _) = fixture();
        let before = session.document().clone();
        let mut h = AssemblyTreeHarness::new(session);
        let header = h.bounds("Create Assembly", false);
        assert!(
            header.left() >= 0. && header.right() <= 225.,
            "assembly action must remain in the tree"
        );
        h.click("Create Assembly", false);
        assert_eq!(h.draft_parent(), Some(component));
        assert_eq!(h.session.document(), &before);
        h.click("Cancel", false);
        assert!(!h.tree.tool_active());
        assert_eq!(h.session.revision(), 0);
        assert_eq!(h.session.document(), &before);

        h.click("Create Assembly", false);
        h.click("Apply", false);
        assert_eq!(h.session.revision(), 1);
        let child = *h.selected.iter().next().unwrap();
        assert!(matches!(
            h.session.document().get(child).unwrap().data,
            EntityData::Assembly { .. }
        ));
        assert_eq!(
            Index::new(h.session.document())
                .unwrap()
                .parent(child)
                .unwrap(),
            Some(component)
        );
        assert_eq!(
            h.session.document().hierarchy().active_component,
            Some(component)
        );
        let applied = h.session.document().clone();
        h.session.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(h.session.document(), &before);
        assert!(!h.session.can_undo());
        h.session.redo(2, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(h.session.document(), &applied);
        assert!(h.session.document().get(child).is_some());

        h.click("Create Assembly", false);
        h.click("Workshop / Cabinet", false);
        h.click("Workshop (root)", true);
        assert_eq!(h.draft_parent(), None);
        h.click("Apply", false);
        assert_eq!(h.session.revision(), 4);
        let root_child = *h.selected.iter().next().unwrap();
        assert_ne!(root_child, child);
        assert_eq!(
            Index::new(h.session.document())
                .unwrap()
                .parent(root_child)
                .unwrap(),
            None
        );
        assert_eq!(
            h.session.document().hierarchy().active_component,
            Some(component)
        );
        let final_document = h.session.document().clone();
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(final_document.clone()).unwrap(),
            &mut bytes,
            FileLimits::default(),
        )
        .unwrap();
        let opened = project_file::read(
            &mut bytes.as_slice(),
            FileLimits::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let reopened = Session::new(opened.project.document).unwrap();
        assert_eq!(reopened.document(), &final_document);
        assert_eq!(reopened.revision(), 0);
        assert!(!reopened.can_undo());
        assert!(!reopened.can_redo());
        assert!(reopened.document().get(child).is_some());
        assert!(reopened.document().get(root_child).is_some());
    }
    #[test]
    fn painted_create_assembly_context_parent_and_stale_revision_refuse_partial_creation() {
        let (session, _, group, _) = fixture();
        let mut h = AssemblyTreeHarness::new(session);
        let group_pos = h.bounds("Panels", false).center();
        h.pointer(group_pos, egui::PointerButton::Secondary);
        h.click("Create Assembly", true);
        assert_eq!(h.draft_parent(), Some(group));
        h.click("Apply", false);
        let created = *h.selected.iter().next().unwrap();
        assert_eq!(
            Index::new(h.session.document())
                .unwrap()
                .parent(created)
                .unwrap(),
            Some(group)
        );
        h.click("Create Assembly", false);
        let pending = h
            .session
            .prepare(Request {
                expected_revision: h.session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Concurrent owner edit".into(),
                edits: vec![Mutation::RenameDocument("Updated workshop".into())],
            })
            .unwrap();
        h.session.commit(pending).unwrap();
        let before = h.session.document().clone();
        let revision = h.session.revision();
        h.bounds(
            "The document changed. Cancel and reopen this tool to review the current state.",
            false,
        );
        h.click("Apply", false);
        assert_eq!(h.session.revision(), revision);
        assert_eq!(h.session.document(), &before);
        h.click("Cancel", false);
        assert!(!h.tree.tool_active());
        assert_eq!(h.session.document(), &before);
    }
    struct TargetTreeHarness {
        context: egui::Context,
        tree: ModelTree,
        session: Session,
        selected: BTreeSet<EntityId>,
        options: TreeOptions,
    }
    impl TargetTreeHarness {
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let before = self.session.document().clone();
            let revision = self.session.revision();
            let output = self.context.run_ui(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340., 480.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let event =
                        self.tree
                            .show_targets(ui, &mut self.session, &self.selected, self.options);
                    if let Some(next) = event.selection {
                        self.selected = next;
                    }
                    assert!(event.change.is_none());
                    assert!(event.open_sketch.is_none());
                    assert!(event.error.is_none());
                },
            );
            assert_eq!(self.session.document(), &before);
            assert_eq!(self.session.revision(), revision);
            output
        }
        fn click(
            &mut self,
            pos: egui::Pos2,
            modifiers: egui::Modifiers,
            button: egui::PointerButton,
        ) {
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers,
                    },
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ])
                .drop_without_applying_deltas();
            }
        }
        fn bounds(&mut self, label: &str) -> egui::Rect {
            self.frame(vec![]).drop_without_applying_deltas();
            let output = self.frame(vec![]);
            let bounds = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label))
                .and_then(|(_, node)| node.bounds())
                .unwrap();
            let rect = egui::Rect::from_min_max(
                egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
            );
            output.drop_without_applying_deltas();
            rect
        }
    }
    fn target_tree() -> (TargetTreeHarness, EntityId, EntityId, EntityId) {
        let (session, component, group, body) = fixture();
        let context = egui::Context::default();
        context.enable_accesskit();
        (
            TargetTreeHarness {
                context,
                tree: ModelTree::default(),
                session,
                selected: BTreeSet::new(),
                options: TreeOptions::default(),
            },
            component,
            group,
            body,
        )
    }
    #[test]
    fn target_tree_shift_toggles_individual_rows_without_selecting_the_range() {
        let (mut h, component, group, body) = target_tree();
        let component_pos = h.bounds("Cabinet · active").center();
        let body_pos = h.bounds("Side").center();
        h.click(
            component_pos,
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert_eq!(h.selected, BTreeSet::from([component]));
        h.click(
            body_pos,
            egui::Modifiers::SHIFT,
            egui::PointerButton::Primary,
        );
        assert_eq!(h.selected, BTreeSet::from([component, body]));
        assert!(!h.selected.contains(&group));
        h.click(
            component_pos,
            egui::Modifiers::SHIFT,
            egui::PointerButton::Primary,
        );
        assert_eq!(h.selected, BTreeSet::from([body]));
        h.click(
            body_pos,
            egui::Modifiers::COMMAND,
            egui::PointerButton::Primary,
        );
        assert!(h.selected.is_empty());
        h.options.enabled = false;
        h.click(
            body_pos,
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert!(h.selected.is_empty());
        assert!(!h.tree.tool_active());
    }
    #[test]
    fn target_tree_pointer_actions_cannot_change_scope_visibility_or_create_tools() {
        let (mut h, component, _, _) = target_tree();
        let root = h.bounds("Workshop").center();
        // A normal tree double-click activates the root; acquisition must retain
        // the exact prior component scope even when the same gesture is used.
        for _ in 0..2 {
            h.click(root, egui::Modifiers::NONE, egui::PointerButton::Primary);
        }
        assert_eq!(
            h.session.document().hierarchy().active_component,
            Some(component)
        );
        let assembly = h.bounds("Create Assembly").center();
        h.click(
            assembly,
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert!(!h.tree.tool_active());
        let create = h.bounds("Create group").center();
        h.click(create, egui::Modifiers::NONE, egui::PointerButton::Primary);
        assert!(!h.tree.tool_active());
        let body = h.bounds("Side");
        // The visibility control immediately follows this bounded row label.
        h.click(
            egui::pos2(body.right() + 12., body.center().y),
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert!(h.session.document().hierarchy().hidden.is_empty());
        h.click(root, egui::Modifiers::NONE, egui::PointerButton::Secondary);
        let activate = h.bounds("Activate modeling scope").center();
        h.click(
            activate,
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert_eq!(
            h.session.document().hierarchy().active_component,
            Some(component)
        );
        assert!(!h.tree.tool_active());
        assert_eq!(h.session.revision(), 0);
    }
    #[test]
    fn body_history_is_ordered_reference_paths_and_aux_sources_are_local_only() {
        use multicad_core::document::Definition;
        let (session, _, _, body) = fixture();
        let first = Entity::new(
            "Extrude",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.extrude".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let second = Entity::new(
            "Fillet",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.fillet".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![multicad_core::document::Link::new(
                    first.id,
                    "previous-feature",
                )],
                suppressed: false,
            },
        );
        let plane = Entity::new(
            "Datum",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: None,
            },
        );
        let mut part = session.document().get(body).unwrap().clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        *features = vec![first.id, second.id];
        let other = Entity::new("Top", part.data.clone());
        let placed = Entity::new(
            "Placed side",
            EntityData::Instance {
                component: body,
                placement: Default::default(),
                placement_source: None,
            },
        );
        let document = session
            .document()
            .edited(&[
                Mutation::Insert(first.clone()),
                Mutation::Insert(second.clone()),
                Mutation::Insert(plane.clone()),
                Mutation::Replace(part),
                Mutation::Insert(other.clone()),
                Mutation::Insert(placed.clone()),
            ])
            .unwrap();
        let session = Session::new(document).unwrap();
        let before = session.document().clone();
        let mut tree = ModelTree::default();
        let rows = tree.rows(&session).unwrap();
        assert!(!rows.iter().any(|row| row.history_of.is_some()));
        tree.set_expanded(body, true);
        tree.set_expanded(other.id, true);
        let rows = tree.rows(&session).unwrap();
        assert!(!rows.iter().any(|row| row.kind == NodeKind::Plane));
        assert!(
            !rows
                .iter()
                .any(|row| row.kind == NodeKind::Feature && row.history_of.is_none())
        );
        let history: Vec<_> = rows
            .iter()
            .filter(|row| row.history_of == Some(body))
            .map(|row| row.id)
            .collect();
        assert_eq!(history, vec![first.id, second.id]);
        assert!(
            rows.iter()
                .filter(|row| row.history_of == Some(body))
                .all(|row| row.owner == "Workshop")
        );
        let keys: Vec<_> = rows
            .iter()
            .filter(|row| row.id == first.id)
            .map(|row| row.key)
            .collect();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1]);
        assert!(rows.iter().find(|row| row.id == body).unwrap().expandable);
        let placement = rows.iter().find(|row| row.id == placed.id).unwrap();
        assert_eq!(placement.kind, NodeKind::Instance);
        assert!(placement.movable && !placement.destination);
        assert!(
            !placement.expandable,
            "placed copies do not duplicate Part feature history"
        );
        tree.set_expanded(body, false);
        assert!(
            !tree
                .rows(&session)
                .unwrap()
                .iter()
                .any(|row| row.history_of == Some(body))
        );
        tree.set_show_sources(true);
        let rows = tree.rows(&session).unwrap();
        assert!(rows.iter().any(|row| row.id == plane.id));
        assert!(
            !rows
                .iter()
                .any(|row| row.id == first.id && row.history_of.is_none())
        );
        tree.selected_references.insert(first.id);
        tree.locate_source(session.document(), first.id);
        assert!(!tree.selected_references.contains(&first.id));
        assert!(
            tree.rows(&session)
                .unwrap()
                .iter()
                .any(|row| row.id == first.id && row.movable)
        );
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
    }
    #[test]
    fn same_name_identity_component_placement_has_one_default_row_but_remains_inspectable() {
        use multicad_core::document::{Entity, Mutation};
        let (session, component, _, _) = fixture();
        let name = session.document().get(component).unwrap().name.clone();
        let placed = Entity::new(
            name,
            EntityData::Instance {
                component,
                placement: Placement::default(),
                placement_source: None,
            },
        );
        let session = Session::new(
            session
                .document()
                .edited(&[Mutation::Insert(placed.clone())])
                .unwrap(),
        )
        .unwrap();
        let mut tree = ModelTree::default();
        let rows = tree.rows(&session).unwrap();
        assert!(
            rows.iter()
                .any(|row| row.id == component && row.kind == NodeKind::Component)
        );
        assert!(!rows.iter().any(|row| row.id == placed.id));
        tree.set_show_sources(true);
        assert!(
            tree.rows(&session)
                .unwrap()
                .iter()
                .any(|row| row.id == placed.id)
        );
    }

    #[test]
    fn missing_body_history_remains_a_disabled_diagnostic_reference() {
        let (session, _, _, body) = fixture();
        let missing = EntityId::new();
        let mut part = session.document().get(body).unwrap().clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        features.push(missing);
        let document = session
            .document()
            .edited(&[Mutation::Replace(part)])
            .unwrap();
        let session = Session::new(document).unwrap();
        let mut tree = ModelTree::default();
        tree.set_expanded(body, true);
        let row = tree
            .rows(&session)
            .unwrap()
            .iter()
            .find(|row| row.id == missing)
            .unwrap();
        assert_eq!(row.history_of, Some(body));
        assert!(!row.available);
        assert_eq!(row.name, "Missing feature");
    }
    #[test]
    fn resizable_tree_rows_do_not_expand_the_panel_or_starve_the_viewport_over_time() {
        use multicad_core::document::Definition;
        let (session, component, group, body) = fixture();
        let mut component_entity = session.document().get(component).unwrap().clone();
        component_entity.name = "Cabinet with a long descriptive name ".repeat(15);
        let feature = Entity::new(
            "Extrusion with an unusually long history label ".repeat(15),
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.extrude".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let mut body_entity = session.document().get(body).unwrap().clone();
        body_entity.name = "An unusually long body label ".repeat(20);
        let EntityData::Part { features, .. } = &mut body_entity.data else {
            unreachable!()
        };
        features.push(feature.id);
        let mut edits = vec![
            Mutation::Replace(component_entity),
            Mutation::Replace(body_entity),
            Mutation::Insert(feature),
        ];
        let mut state = session.document().hierarchy().clone();
        let mut parent = group;
        for depth in 0..12 {
            let nested = Entity::new(format!("Nested organization {depth}"), EntityData::Group {});
            state.members.insert(parent, vec![nested.id]);
            parent = nested.id;
            edits.push(Mutation::Insert(nested));
        }
        state.members.insert(parent, vec![body]);
        edits.push(Mutation::SetHierarchy(state));
        let document = session.document().edited(&edits).unwrap();
        for pixels_per_point in [1., 2.] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(pixels_per_point);
            let mut session = Session::new(document.clone()).unwrap();
            let mut tree = ModelTree::default();
            for _ in 0..360 {
                let mut left_width = 0.;
                let mut viewport_width = 0.;
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1360., 860.),
                        )),
                        focused: true,
                        ..Default::default()
                    },
                    |ui| {
                        let left = egui::Panel::left("bounded-tree")
                            .default_size(225.)
                            .resizable(true)
                            .show(ui, |ui| {
                                let event = tree.show(
                                    ui,
                                    &mut session,
                                    &BTreeSet::new(),
                                    TreeOptions::default(),
                                );
                                assert!(event.change.is_none());
                            });
                        left_width = left.response.rect.width();
                        egui::Panel::right("bounded-inspector")
                            .default_size(250.)
                            .resizable(true)
                            .show(ui, |ui| {
                                ui.label("Inspector");
                            });
                        egui::CentralPanel::default().show(ui, |ui| {
                            viewport_width = ui.available_width();
                        });
                    },
                )
                .drop_without_applying_deltas();
                assert!(
                    left_width <= 230.,
                    "tree width grew to {left_width} at scale {pixels_per_point}"
                );
                assert!(
                    viewport_width >= 800.,
                    "tree starved viewport: {viewport_width}"
                );
            }
            assert_eq!(session.document(), &document);
            assert_eq!(session.revision(), 0);
        }
    }
    #[test]
    fn resizable_inspector_stays_bounded_through_long_parent_move_and_shared_summary() {
        use crate::viewport::{Geometry, Instance, Interaction, Scene, SceneStamp, Transform};
        use multicad_core::document::{Definition, Placement};
        use multicad_kernel::{MeshOptions, Solid};
        use std::sync::Arc;
        let (session, component, _, body) = fixture();
        let nested = Entity::new(
            "Drawer module with a long descriptive component name",
            EntityData::Assembly { instances: vec![] },
        );
        let material = Entity::new(
            "Birch plywood with a long manufacturing material description",
            EntityData::Material {
                density_kg_m3: Some(700.),
                color_rgba: [140, 170, 190, 255],
                section_hatch: None,
            },
        );
        let producer = Entity::new(
            "Independent native source",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.native-box".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let occurrence = Entity::new(
            "Placed reference with a long ordering description",
            EntityData::Instance {
                component: body,
                placement: Placement {
                    translation_mm: [30., 0., 0.],
                    ..Default::default()
                },
                placement_source: None,
            },
        );
        let mut source_body = session.document().get(body).unwrap().clone();
        source_body.name = "Visible side A with a long descriptive body name".into();
        let EntityData::Part {
            features,
            material: body_material,
            tags,
        } = &mut source_body.data
        else {
            unreachable!()
        };
        features.push(producer.id);
        *body_material = Some(material.id);
        tags.extend([
            "carcass-panel-for-cabinet-A-ready-for-manufacturing-without-finishing".into(),
            "shared-woodworking-tag-with-a-long-unbroken-identifier-that-needs-wrapping".into(),
        ]);
        let mut state = session.document().hierarchy().clone();
        state.members.get_mut(&component).unwrap().push(nested.id);
        state.members.insert(nested.id, vec![occurrence.id]);
        let document = session
            .document()
            .edited(&[
                Mutation::Insert(nested.clone()),
                Mutation::Insert(material),
                Mutation::Insert(producer.clone()),
                Mutation::Insert(occurrence.clone()),
                Mutation::Replace(source_body),
                Mutation::SetHierarchy(state),
            ])
            .unwrap();
        let solid = Solid::box_named([20., 12., 8.], producer.id, Default::default()).unwrap();
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        let geometry = Arc::new(
            Geometry::new(
                producer.id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                solid.topology_snapshot(producer.id, 0).unwrap().elements,
            )
            .unwrap(),
        );
        for pixels_per_point in [1., 2.] {
            for panel_width in [250., 180.] {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(pixels_per_point);
                let mut session = Session::new(document.clone()).unwrap();
                let mut tree = ModelTree::default();
                let mut interaction = Interaction::default();
                let selected = BTreeSet::from([body]);
                for frame in 0..240 {
                    if frame == 160 {
                        let payload = TreeDrag {
                            stamp: (document.id(), 0),
                            entries: vec![body],
                        };
                        let result = tree.drop_entries(
                            &mut session,
                            &payload,
                            occurrence.id,
                            Some(DropPosition::Before),
                        );
                        assert!(result.error.is_none(), "{:?}", result.error);
                        assert_eq!(session.revision(), 1);
                        assert_eq!(
                            Index::new(session.document())
                                .unwrap()
                                .owning_component(body)
                                .unwrap(),
                            Some(nested.id)
                        );
                    }
                    let scene = Scene::new(
                        SceneStamp {
                            document: document.id(),
                            revision: session.revision(),
                            generation: session.revision() + 1,
                        },
                        vec![geometry.clone()],
                        vec![
                            Instance {
                                id: body,
                                geometry: producer.id,
                                transform: Default::default(),
                                color: [140, 170, 190, 255],
                            },
                            Instance {
                                id: occurrence.id,
                                geometry: producer.id,
                                transform: Transform::from_placement(Placement {
                                    translation_mm: [30., 0., 0.],
                                    ..Default::default()
                                })
                                .unwrap(),
                                color: [140, 170, 190, 255],
                            },
                        ],
                    )
                    .unwrap();
                    interaction
                        .select_source_entities(
                            session.document(),
                            session.revision(),
                            &scene,
                            &selected,
                        )
                        .unwrap();
                    let mut width = 0.;
                    let mut viewport_width = 0.;
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1360., 860.),
                            )),
                            focused: true,
                            ..Default::default()
                        },
                        |ui| {
                            egui::Panel::left("inspector-regression-tree")
                                .default_size(225.)
                                .resizable(true)
                                .show(ui, |ui| {
                                    ui.label("Model tree");
                                });
                            let inspector = egui::Panel::right("inspector-regression")
                                .default_size(panel_width)
                                .resizable(true)
                                .show(ui, |ui| {
                                    ui.heading("Inspector");
                                    ui.separator();
                                    egui::ScrollArea::vertical().show(ui, |ui| {
                                        let tool_active = tree.tool_active();
                                        let events = tree.inspector(
                                            ui,
                                            &mut session,
                                            &selected,
                                            TreeOptions::default(),
                                        );
                                        assert!(events.change.is_none());
                                        if !tool_active {
                                            interaction.inspector(ui, session.document());
                                        }
                                    });
                                });
                            width = inspector.response.rect.width();
                            egui::CentralPanel::default().show(ui, |ui| {
                                viewport_width = ui.available_width();
                            });
                        },
                    )
                    .drop_without_applying_deltas();
                    assert!(
                        width <= panel_width + 2.,
                        "Inspector grew to {width} from {panel_width} at frame {frame}, scale {pixels_per_point}"
                    );
                    assert!(
                        viewport_width >= 800.,
                        "Inspector starved viewport: {viewport_width}"
                    );
                }
                assert_eq!(session.revision(), 1);
            }
        }
    }
}
