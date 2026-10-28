//! Reusable source hierarchy tree and Inspector tools. Collapse and selection are
//! host/UI state; model edits use the same prepared commands as external callers.
use crate::{
    egui::{self, Color32},
    icons::{self, Icon},
};
use multicad_core::{
    commands::{Actor, Change, Channel, Session},
    document::{Document, EntityData, EntityId},
    hierarchy::{self, Action, ContainerKind, Index},
};
use std::collections::{BTreeMap, BTreeSet};
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
            Self::Body => "Body",
            Self::Sketch => "Sketch",
            Self::Instance => "Instance",
            Self::Plane => "Plane",
            Self::Feature => "Feature",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Self::Root => Icon::Home,
            Self::Component | Self::Body => Icon::Box,
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
    pub owner: String,
    pub local_visible: bool,
    pub effective_visible: bool,
    pub active: bool,
    pub expandable: bool,
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
        document: &Document,
        revision: u64,
        collapsed: &BTreeSet<EntityId>,
        salt: egui::Id,
        show_sources: bool,
    ) -> Result<Self, String> {
        let index = Index::new(document).map_err(|error| error.to_string())?;
        let mut destinations = vec![Destination {
            id: None,
            path: format!("{} (root)", document.name()),
        }];
        for entity in document.entities() {
            if hierarchy::container_kind(&entity.data).is_some() {
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
            let history = match document.get(id).map(|entity| &entity.data) {
                Some(EntityData::Part { features, .. }) => features.as_slice(),
                _ => &[],
            };
            let expandable = !children.is_empty() || !history.is_empty();
            rows.push(TreeRow {
                id,
                depth,
                name,
                kind: node_kind,
                owner,
                local_visible,
                effective_visible,
                active,
                expandable,
                history_of: None,
                available: true,
                key,
            });
            if !collapsed.contains(&id) {
                for (position, feature) in history.iter().copied().enumerate() {
                    let entity = document.get(feature);
                    let available = entity
                        .is_some_and(|entity| matches!(entity.data, EntityData::Feature { .. }));
                    let name = match entity {
                        Some(entity) if available => entity.name.clone(),
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
                        owner,
                        local_visible: true,
                        effective_visible,
                        active: false,
                        expandable: false,
                        history_of: Some(id),
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
    Move {
        document: EntityId,
        revision: u64,
        entries: Vec<EntityId>,
        parent: Option<EntityId>,
        before: Option<EntityId>,
    },
}
impl Tool {
    fn document(&self) -> EntityId {
        match self {
            Self::Create { document, .. }
            | Self::Rename { document, .. }
            | Self::Move { document, .. } => *document,
        }
    }
    fn revision(&self) -> u64 {
        match self {
            Self::Create { revision, .. }
            | Self::Rename { revision, .. }
            | Self::Move { revision, .. } => *revision,
        }
    }
}
pub struct ModelTree {
    salt: egui::Id,
    collapsed: BTreeSet<EntityId>,
    view: Option<TreeView>,
    anchor: Option<egui::Id>,
    tool: Option<Tool>,
    show_sources: bool,
    history_selection: Option<(EntityId, EntityId)>,
    selected_references: BTreeSet<EntityId>,
    scroll_to_source: Option<EntityId>,
}
impl Default for ModelTree {
    fn default() -> Self {
        Self {
            salt: egui::Id::new(EntityId::new()),
            collapsed: BTreeSet::new(),
            view: None,
            anchor: None,
            tool: None,
            show_sources: false,
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
                ContainerKind::Component => "Component",
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
    pub fn begin_move(&mut self, session: &Session, entries: Vec<EntityId>) -> Result<(), String> {
        if entries.is_empty() {
            return Err("Select model entries to move".into());
        }
        if entries
            .iter()
            .any(|id| self.selected_references.contains(id))
        {
            return Err("History rows reference a feature. Select its source entry explicitly before changing membership".into());
        }
        for id in &entries {
            if !session
                .document()
                .get(*id)
                .is_some_and(|entity| hierarchy::is_model_entry(&entity.data))
            {
                return Err("The selection contains an unsupported model entry".into());
            }
        }
        self.tool = Some(Tool::Move {
            document: session.document().id(),
            revision: session.revision(),
            entries,
            parent: session.document().hierarchy().active_component,
            before: None,
        });
        Ok(())
    }
    fn ensure_view(&mut self, session: &Session) -> Result<(), String> {
        let stamp = (session.document().id(), session.revision());
        if self.view.as_ref().is_none_or(|view| view.stamp != stamp) {
            self.view = Some(TreeView::build(
                session.document(),
                session.revision(),
                &self.collapsed,
                self.salt,
                self.show_sources,
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
            self.view = None;
        }
    }
    fn locate_source(&mut self, document: &Document, id: EntityId) {
        self.set_show_sources(true);
        self.history_selection = None;
        self.selected_references.remove(&id);
        self.scroll_to_source = Some(id);
        self.collapsed.remove(&document.id());
        if let Ok(index) = Index::new(document) {
            let mut next = index.parent(id).ok().flatten();
            while let Some(parent) = next {
                self.collapsed.remove(&parent);
                next = index.parent(parent).ok().flatten();
            }
        }
        self.view = None;
    }
    pub fn set_expanded(&mut self, id: EntityId, expanded: bool) {
        if expanded {
            self.collapsed.remove(&id);
        } else {
            self.collapsed.insert(id);
        }
        self.view = None;
    }
    fn execute(&mut self, session: &mut Session, revision: u64, action: Action) -> TreeEvents {
        let mut events = TreeEvents::default();
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
        let mut moving = None;
        let mut toggle = None;
        ui.horizontal(|ui| {
            ui.strong("Model");
            let response = ui
                .add_enabled_ui(options.enabled, |ui| {
                    icons::compact_button(ui, Icon::New, "Create component or group")
                })
                .inner;
            egui::Popup::menu(&response).show(|ui| {
                for (kind, label) in [
                    (ContainerKind::Component, "New component"),
                    (ContainerKind::Group, "New group"),
                ] {
                    if ui
                        .add_enabled(options.enabled, egui::Button::new(label))
                        .clicked()
                    {
                        create = Some((kind, session.document().hierarchy().active_component));
                        ui.close();
                    }
                }
            });
        });
        let mut show_sources = self.show_sources;
        if ui.checkbox(&mut show_sources, "Show sources").on_hover_text("Show planes and feature sources in their actual owning containers; body history rows remain references").changed() {
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
                    let (row_rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(0.), 26.), egui::Sense::hover());
                    let mut row_ui = ui.new_child(egui::UiBuilder::new().max_rect(row_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                    row_ui.set_clip_rect(row_rect.intersect(ui.clip_rect()));
                    row_ui.push_id(row.key, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space((row.depth as f32 * 10.).min(80.).min((row_rect.width() - 100.).max(0.)));
                            if row.expandable {
                                let collapsed = self.collapsed.contains(&row.id);
                                if ui.add_sized([16., 24.], egui::Button::new(if collapsed { ">" } else { "v" }).frame(false))
                                    .on_hover_text("Expand or collapse this container without editing the document").clicked() {
                                    toggle = Some((row.id, collapsed));
                                }
                            } else {
                                ui.add_space(16.);
                            }
                            let icon = ui.add_enabled_ui(options.enabled && row.available, |ui| {
                                icons::mini_button(ui, row.kind.icon(), &format!("Select {} {}", row.kind.label(), row.name))
                            }).inner;
                            let trailing = ui.spacing().interact_size.y.max(24.);
                            let width = (ui.available_width() - trailing - 2. * ui.spacing().item_spacing.x).max(0.);
                            let label = if row.active { format!("{} · active", row.name) } else { row.name.clone() };
                            let color = if row.effective_visible { ui.visuals().text_color() } else { Color32::GRAY };
                            let response = flat_row_label(ui, width, &label, options.enabled && row.available, selected.contains(&row.id), color);
                            let role = if row.history_of.is_some() { "Feature used by body (reference only)" } else { row.kind.label() };
                            response.clone().on_hover_text(format!("{}\nOwning component: {}\nCmd/Ctrl-click toggles; Shift-click selects a range", role, row.owner));
                            if response.clicked() || icon.clicked() {
                                let modifiers = crate::selection_input::click_modifiers(ui, if response.clicked() { &response } else { &icon });
                                let mut next = if modifiers.command { selected.clone() } else { BTreeSet::new() };
                                if modifiers.shift && let Some(start) = self.anchor.and_then(|id| rows.iter().position(|row| row.key == id)) {
                                    next.extend(rows[start.min(row_index)..=start.max(row_index)].iter().filter(|row| row.available).map(|row| row.id));
                                } else if !modifiers.command || !next.remove(&row.id) {
                                    next.insert(row.id);
                                }
                                self.selected_references.retain(|id| next.contains(id));
                                if !modifiers.command {
                                    self.selected_references.clear();
                                }
                                if modifiers.shift && let Some(start) = self.anchor.and_then(|key| rows.iter().position(|row| row.key == key)) {
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
                            if response.double_clicked() && options.enabled {
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
                            let visibility = ui.add_enabled(options.enabled && !matches!(row.kind, NodeKind::Plane | NodeKind::Feature),
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
                            response.context_menu(|ui| {
                                if row.history_of.is_some() {
                                    ui.label("Feature used by body");
                                    ui.small("History is a reference, not container ownership. Inspect its source to edit membership.");
                                    return;
                                }
                                ui.add_enabled_ui(options.enabled && row.available, |ui| {
                                    if matches!(row.kind, NodeKind::Root | NodeKind::Component)
                                        && ui.button("Activate modeling scope").clicked() {
                                        action = Some(Action::SetActive { component: if row.kind == NodeKind::Root { None } else { Some(row.id) } });
                                        ui.close();
                                    }
                                    if row.kind.container() {
                                        for (kind, label) in [(ContainerKind::Component, "New child component"), (ContainerKind::Group, "New child group")] {
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
                                    if row.kind != NodeKind::Root && ui.button("Move to container…").clicked() {
                                        moving = Some(if selected.contains(&row.id) { selected.iter().copied().collect() } else { vec![row.id] });
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
                                });
                            });
                        });
                    });
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
        if let Some(ids) = moving
            && let Err(error) = self.begin_move(session, ids)
        {
            events.error = Some(error);
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
                        ui.strong(match kind { ContainerKind::Component => "New component", ContainerKind::Group => "New group" });
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)).on_hover_text("Name for the new model container");
                        parent_choice(ui, "create-parent", parent, &view.destinations);
                    }
                    Tool::Rename { name, .. } => {
                        ui.strong("Rename container");
                        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)).on_hover_text("New component or group name; identity and references remain stable");
                    }
                    Tool::Move { entries, parent, before, .. } => {
                        ui.strong(format!("Move {} model entries", entries.len()));
                        ui.label("Geometry and source identities are retained. A group preserves its enclosing component scope.");
                        let prior = *parent;
                        parent_choice(ui, "move-parent", parent, &view.destinations);
                        if prior != *parent { *before = None; }
                        let key = parent.unwrap_or(session.document().id());
                        let siblings = session.document().hierarchy().members.get(&key).cloned().unwrap_or_default();
                        let before_label = before.and_then(|id| session.document().get(id).map(|entity| entity.name.clone())).unwrap_or_else(|| "Append at end".into());
                        crate::controls::inspector_combo(ui, "move-before")
                            .selected_text(&before_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(before, None, "Append at end");
                                for id in siblings {
                                    if !entries.contains(&id) && let Some(entity) = session.document().get(id) {
                                        ui.selectable_value(before, Some(id), format!("Before {}", entity.name));
                                    }
                                }
                            }).response.on_hover_text(format!("{before_label}\nOrder relative to an explicit member; normalize legacy membership before reordering its instances"));
                    }
                }
                if !current {
                    ui.colored_label(Color32::LIGHT_RED, "The document changed. Cancel and reopen this tool to review the current state.");
                }
                ui.horizontal_wrapped(|ui| {
                    apply = ui.add_enabled(current, egui::Button::new("Apply")).on_hover_text("Commit one undoable hierarchy transaction").clicked();
                    cancel = ui.button("Cancel").on_hover_text("Discard this source-neutral form").clicked();
                });
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
        ui.strong(if root {
            session.document().name()
        } else {
            &entity.unwrap().name
        });
        ui.label(node_kind.label());
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
            if let Some(EntityData::Instance { component, .. }) = entity.map(|entity| &entity.data)
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
        if let Some(EntityData::Part { material, tags, .. }) = entity.map(|entity| &entity.data) {
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
        let history_body = self
            .history_selection
            .filter(|(feature, _)| *feature == id)
            .and_then(|(_, body)| session.document().get(body))
            .map(|body| body.name.clone());
        if let Some(body) = &history_body {
            ui.label(format!("Feature used by body: {body}"));
            ui.small(
                "This history row references the source; it does not change its owning component.",
            );
            if ui
                .button("Show source entry")
                .on_hover_text("Reveal this feature under its actual owning container")
                .clicked()
            {
                self.locate_source(session.document(), id);
            }
        }
        let mut action = None;
        let mut rename = false;
        let mut moving = false;
        ui.add_enabled_ui(enabled, |ui| {
            if matches!(node_kind, NodeKind::Root | NodeKind::Component) && ui.button("Activate modeling scope")
                .on_hover_text("New source geometry belongs to this component; nested components retain their own scope").clicked() {
                action = Some(Action::SetActive { component: if root { None } else { Some(id) } });
            }
            if matches!(node_kind, NodeKind::Component | NodeKind::Group) {
                rename = ui.button("Rename…").on_hover_text("Rename this container without changing its identity").clicked();
            }
            if !root && history_body.is_none() && selected.iter().all(|id| !self.selected_references.contains(id)) {
                moving = ui.button("Move to container…").on_hover_text("Choose an explicit parent and ordering position; this does not transform geometry").clicked();
            }
            if node_kind == NodeKind::Sketch && options.allow_open_sketch
                && ui.button("Open selected sketch").on_hover_text("Open this sketch in the Sketch workspace").clicked() {
                events.open_sketch = Some(id);
            }
        });
        if rename && let Err(error) = self.begin_rename(session, id) {
            events.error = Some(error);
        }
        if moving && let Err(error) = self.begin_move(session, selected.iter().copied().collect()) {
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
            Tool::Move {
                entries,
                parent,
                before,
                ..
            } => Action::Move {
                entries,
                parent,
                before,
            },
        };
        let events = self.execute(session, revision, action);
        if events.error.is_none() {
            self.tool = None;
        }
        events
    }
}
fn flat_row_label(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    enabled: bool,
    selected: bool,
    color: Color32,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, 24.),
        if enabled {
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
        tree.begin_move(&session, vec![body]).unwrap();
        if let Some(Tool::Move { parent, .. }) = &mut tree.tool {
            *parent = None;
        }
        let events = tree.apply_tool(&mut session);
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
        tree.begin_move(&session, vec![component]).unwrap();
        if let Some(Tool::Move { parent, .. }) = &mut tree.tool {
            *parent = Some(group);
        }
        let before = session.document().clone();
        assert!(tree.apply_tool(&mut session).error.is_some());
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
        let document = session
            .document()
            .edited(&[
                Mutation::Insert(first.clone()),
                Mutation::Insert(second.clone()),
                Mutation::Insert(plane.clone()),
                Mutation::Replace(part),
                Mutation::Insert(other.clone()),
            ])
            .unwrap();
        let session = Session::new(document).unwrap();
        let before = session.document().clone();
        let mut tree = ModelTree::default();
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
            rows.iter()
                .any(|row| row.id == first.id && row.history_of.is_none())
        );
        tree.selected_references.insert(first.id);
        assert!(tree.begin_move(&session, vec![body, first.id]).is_err());
        tree.locate_source(session.document(), first.id);
        assert!(!tree.selected_references.contains(&first.id));
        assert!(tree.begin_move(&session, vec![first.id]).is_ok());
        tree.cancel_tool();
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
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
                tree.begin_move(&session, vec![body]).unwrap();
                for frame in 0..240 {
                    if frame == 80 {
                        let Some(Tool::Move { parent, before, .. }) = &mut tree.tool else {
                            panic!("move form")
                        };
                        *parent = Some(nested.id);
                        *before = Some(occurrence.id);
                    }
                    if frame == 160 {
                        let result = tree.apply_tool(&mut session);
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
                                            interaction.quick_actions(ui, &mut session, true);
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
