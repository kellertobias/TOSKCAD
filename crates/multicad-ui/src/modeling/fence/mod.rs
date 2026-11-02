//! One native source-change form for an entire editable fence run.
mod construction;
#[cfg(test)]
mod tests;
use super::{
    FormStamp, PreviewState, TerminationViewport,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
    outline::{OutlineEditor, OutlineKind, plan_targets},
};
use crate::{controls, egui};
use construction::ConstructionEditor;
use multicad_core::{
    commands::{PreparationSnapshot, Session, SessionStamp},
    document::{Document, EntityData, EntityId},
    model::*,
    parameters::{EvaluatedParameters, Unit},
};
use std::{cell::RefCell, sync::Arc};

struct PositionRow {
    post: FencePostKey,
    offset: DimensionField,
}
#[derive(Clone)]
enum Intent {
    Local(Box<FenceChange>),
    Symbol(crate::symbols::architecture::Command),
}
struct Draft {
    command: Result<Intent, String>,
    layout: Result<Arc<FenceLayout>, String>,
    automatic: Result<Arc<FencePostPlan>, String>,
}
type DraftCache = Option<(SessionStamp, FormStamp, Arc<Draft>)>;
pub(super) struct FencePanel {
    symbol: crate::symbols::architecture::Binding,
    run: EntityId,
    group: EntityId,
    name: String,
    editing: bool,
    selected_type: Option<EntityId>,
    new_type: EntityId,
    type_name: String,
    shared: bool,
    construction: ConstructionEditor,
    outline: OutlineEditor,
    level: Option<EntityId>,
    base_z: DimensionField,
    height_override: bool,
    height: DimensionField,
    spacing_override: bool,
    spacing: DimensionField,
    positions: Vec<PositionRow>,
    selected_post: Option<FencePostKey>,
    stamp: FormStamp,
    source: Option<PreparationSnapshot>,
    cache: RefCell<DraftCache>,
    types: Vec<(EntityId, String)>,
    levels: Vec<(EntityId, String)>,
    users: Vec<String>,
    targets: Vec<[f64; 2]>,
    error: Option<String>,
}
fn field(value: f64) -> DimensionField {
    DimensionField::from_dimension(&length(value), "mm").expect("Millimeters are length units")
}
impl FencePanel {
    pub fn new(
        session: &Session,
        name: String,
        preferred: Option<EntityId>,
    ) -> Result<Self, String> {
        let document = session.document();
        let owned = multicad_core::symbols::owned_entities_in_session(session)
            .map_err(|e| e.to_string())?;
        let selected = preferred
            .filter(|id| read_fence_type(document, *id).is_ok())
            .or_else(|| {
                fence_types(document)
                    .find(|e| !owned.contains(&e.id))
                    .map(|e| e.id)
            });
        let run = EntityId::new();
        let group = EntityId::new();
        let mut symbol = crate::symbols::architecture::Binding::new(
            document,
            multicad_core::symbols::architecture::ArchitectureOutput::Fence { run, group },
        )?;
        if selected.is_none()
            && let Some(id) = symbol.first()
        {
            symbol.select(document, Some(id))?;
        }
        let construction = match selected {
            Some(id) => ConstructionEditor::existing(
                document,
                &read_fence_type(document, id).map_err(|e| e.to_string())?,
            )?,
            None => {
                if let Some(definition) = symbol.snapshot() {
                    let blueprint =
                        multicad_core::symbols::architecture::blueprint_for_definition(definition)
                            .map_err(|e| e.to_string())?
                            .ok_or("Fence interface is missing")?;
                    let construction =
                        read_fence_type(&definition.template, blueprint.construction)
                            .map_err(|e| e.to_string())?;
                    ConstructionEditor::existing(&definition.template, &construction)?
                } else {
                    ConstructionEditor::new(document)?
                }
            }
        };
        let type_name = selected
            .and_then(|id| document.get(id))
            .map_or("Fence construction".into(), |e| e.name.clone());
        let mut panel = Self {
            symbol,
            run,
            group,
            name,
            editing: false,
            selected_type: selected,
            new_type: EntityId::new(),
            type_name,
            shared: false,
            construction,
            outline: OutlineEditor::from_mm(
                OutlineKind::FencePath,
                &[[0., 0.], [4000., 0.]],
                false,
            ),
            level: None,
            base_z: field(0.),
            height_override: false,
            height: field(1800.),
            spacing_override: false,
            spacing: field(2000.),
            positions: Vec::new(),
            selected_post: None,
            stamp: FormStamp::new(),
            source: None,
            cache: RefCell::new(None),
            types: Vec::new(),
            levels: Vec::new(),
            users: Vec::new(),
            targets: Vec::new(),
            error: None,
        };
        panel.sync_source(session);
        Ok(panel)
    }
    pub fn existing(session: &Session, run: EntityId) -> Result<Self, String> {
        let document = session.document();
        let source = read_fence_run(document, run).map_err(|e| e.to_string())?;
        let mut panel = Self::new(
            session,
            document
                .get(run)
                .ok_or("Fence run is missing")?
                .name
                .clone(),
            Some(source.fence_type),
        )?;
        panel.run = run;
        panel.group = source.group;
        panel.editing = true;
        if let Some(binding) = crate::symbols::architecture::Binding::existing(document, run)? {
            panel.symbol = binding;
        }
        panel.symbol.sync_source(session);
        panel.outline = OutlineEditor::new(
            OutlineKind::FencePath,
            &source
                .path
                .vertices
                .iter()
                .map(|v| v.point.clone())
                .collect::<Vec<_>>(),
            source.path.closed,
        )?;
        panel.outline.set_vertex_ids(
            &source
                .path
                .vertices
                .iter()
                .map(|v| v.id)
                .collect::<Vec<_>>(),
        )?;
        panel.outline.sync_source(session);
        panel.level = source.level;
        panel.base_z = DimensionField::from_dimension(&source.path.base_z, "mm")?;
        panel.height_override = source.overrides.post_height.is_some();
        if let Some(value) = source.overrides.post_height {
            panel.height = DimensionField::from_dimension(&value, "mm")?;
        }
        panel.spacing_override = source.overrides.post_spacing.is_some();
        if let Some(value) = source.overrides.post_spacing {
            panel.spacing = DimensionField::from_dimension(&value, "mm")?;
        }
        panel.positions = source
            .post_positions
            .into_iter()
            .map(|p| {
                Ok(PositionRow {
                    post: p.post,
                    offset: DimensionField::from_dimension(&p.offset, "mm")?,
                })
            })
            .collect::<Result<_, String>>()?;
        panel.invalidate();
        Ok(panel)
    }
    pub fn target(document: &Document, id: EntityId) -> Option<EntityId> {
        match &document.get(id)?.data {
            EntityData::Extension { definition, .. } if definition.type_id == FENCE_RUN_KIND => {
                read_fence_run(document, id).ok().map(|_| id)
            }
            EntityData::Feature { definition, .. } if definition.type_id == FENCE_MEMBER_TYPE => {
                read_fence_member(document, id).ok().map(|m| m.run)
            }
            EntityData::Part { features, .. } => features
                .iter()
                .find_map(|feature| read_fence_member(document, *feature).ok().map(|m| m.run)),
            EntityData::Group {} => fence_runs(document)
                .find(|e| read_fence_run(document, e.id).is_ok_and(|r| r.group == id))
                .map(|e| e.id),
            _ => None,
        }
    }
    pub fn select_definition(
        &mut self,
        document: &Document,
        definition: EntityId,
    ) -> Result<(), String> {
        self.symbol.select(document, Some(definition))?;
        self.height_override = false;
        self.spacing_override = false;
        self.shared = false;
        self.invalidate();
        Ok(())
    }
    pub fn preferred_type(&self) -> EntityId {
        self.selected_type.unwrap_or(self.new_type)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Fence"
    }
    fn invalidate(&mut self) {
        self.stamp.changed();
        *self.cache.get_mut() = None;
        self.error = None;
    }
    pub fn sync_source(&mut self, session: &Session) {
        self.symbol.sync_source(session);
        if self
            .source
            .as_ref()
            .is_some_and(|s| s.session().stamp() == session.stamp())
        {
            return;
        }
        let document = session.document();
        self.source = Some(session.preparation_snapshot());
        self.outline.sync_source(session);
        let owned = multicad_core::symbols::owned_entities_in_session(session).unwrap_or_default();
        self.types = fence_types(document)
            .filter(|e| !owned.contains(&e.id))
            .map(|e| (e.id, e.name.clone()))
            .collect();
        self.levels = levels(document).map(|e| (e.id, e.name.clone())).collect();
        if !self.shared
            && let Some(id) = self.selected_type
            && let Ok(value) = read_fence_type(document, id)
            && let Ok(editor) = ConstructionEditor::existing(document, &value)
        {
            self.construction = editor;
            self.type_name = document.get(id).expect("existing type").name.clone();
        }
        self.construction.sync_source(session);
        self.users = self
            .selected_type
            .and_then(|id| fence_type_users(document, id).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| document.get(id).map(|e| e.name.clone()))
            .collect();
        self.targets = plan_targets(document, None);
        for entity in fence_runs(document) {
            if entity.id == self.run {
                continue;
            }
            if let Ok(source) = read_fence_run(document, entity.id) {
                for vertex in source.path.vertices {
                    let x = vertex.point[0].value_in(Unit::Millimeter, Some(session.parameters()));
                    let y = vertex.point[1].value_in(Unit::Millimeter, Some(session.parameters()));
                    if let (Ok(x), Ok(y)) = (x, y) {
                        self.targets.push([x, y]);
                    }
                }
            }
        }
        *self.cache.get_mut() = None;
    }
    fn source_command(
        &self,
        document: &Document,
    ) -> Result<(FenceChange, FenceConstruction, FenceRunDefinition), String> {
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Fence name must contain 1-1000 bytes".into());
        }
        if (self.selected_type.is_none() || self.shared)
            && (self.type_name.trim().is_empty() || self.type_name.len() > 1000)
        {
            return Err("Construction name must contain 1-1000 bytes".into());
        }
        let points = self.outline.dimensions(document)?;
        let ids = self.outline.vertex_ids();
        if points.len() != ids.len() {
            return Err("Path identities do not match the drawn corners; redraw the path".into());
        }
        let construction = if let Some(id) = self.selected_type.filter(|_| !self.shared) {
            read_fence_type(document, id).map_err(|e| e.to_string())?
        } else {
            self.construction.value(document)?
        };
        let source = FenceRunDefinition {
            fence_type: self.preferred_type(),
            path: FencePath {
                vertices: points
                    .into_iter()
                    .zip(ids)
                    .map(|(point, id)| FenceVertex { id: *id, point })
                    .collect(),
                closed: self.outline.closed(),
                base_z: self.base_z.resolve(document)?,
            },
            group: self.group,
            level: self.level,
            overrides: FenceInstanceOverrides {
                post_height: self
                    .height_override
                    .then(|| self.height.resolve(document))
                    .transpose()?,
                post_spacing: self
                    .spacing_override
                    .then(|| self.spacing.resolve(document))
                    .transpose()?,
            },
            post_positions: self
                .positions
                .iter()
                .map(|p| {
                    Ok(FencePostPosition {
                        post: p.post,
                        offset: p.offset.resolve(document)?,
                    })
                })
                .collect::<Result<_, String>>()?,
        };
        source.validate().map_err(|e| e.to_string())?;
        let request = FenceRunRequest {
            id: self.run,
            name: self.name.clone(),
            definition: source.clone(),
        };
        let definition = FenceTypeRequest {
            id: self.preferred_type(),
            name: self.type_name.clone(),
            construction: construction.clone(),
        };
        let command = FenceChange {
            new_type: self.selected_type.is_none().then(|| definition.clone()),
            edit_type: self.shared.then_some(definition),
            run: Some(if self.editing {
                FenceRunChange::Edit(request)
            } else {
                FenceRunChange::Create(request)
            }),
        };
        Ok((command, construction, source))
    }
    fn build_draft(&self, document: &Document, parameters: &EvaluatedParameters) -> Draft {
        if self.symbol.active() {
            return match self.symbol_context(document) {
                Ok(context) => match (
                    self.symbol.command(document, &self.name, context.clone()),
                    self.symbol.fence_plan(document, context),
                ) {
                    (Ok(command), Ok(plan)) => Draft {
                        command: Ok(Intent::Symbol(command)),
                        layout: plan.layout.map(Arc::new),
                        automatic: plan.automatic.map(Arc::new),
                    },
                    (Err(error), _) | (_, Err(error)) => Draft {
                        command: Err(error.clone()),
                        layout: Err(error.clone()),
                        automatic: Err(error),
                    },
                },
                Err(error) => Draft {
                    command: Err(error.clone()),
                    layout: Err(error.clone()),
                    automatic: Err(error),
                },
            };
        }
        match self.source_command(document) {
            Ok((command, construction, source)) => {
                let layout = plan_fence_instance(document, parameters, &construction, &source)
                    .map(Arc::new)
                    .map_err(|e| e.to_string());
                let mut automatic = source;
                automatic.post_positions.clear();
                let automatic =
                    plan_fence_instance_posts(document, parameters, &construction, &automatic)
                        .map(Arc::new)
                        .map_err(|e| e.to_string());
                Draft {
                    command: Ok(Intent::Local(Box::new(command))),
                    layout,
                    automatic,
                }
            }
            Err(error) => Draft {
                command: Err(error.clone()),
                layout: Err(error.clone()),
                automatic: Err(error),
            },
        }
    }
    fn draft(&self, document: &Document) -> Arc<Draft> {
        if let Some(source) = self
            .source
            .as_ref()
            .filter(|s| std::ptr::eq(s.session().document(), document))
        {
            let epoch = source.session().stamp();
            if let Some((old, stamp, value)) = &*self.cache.borrow()
                && *old == epoch
                && *stamp == self.stamp
            {
                return Arc::clone(value);
            }
            let value = Arc::new(self.build_draft(document, source.session().parameters()));
            *self.cache.borrow_mut() = Some((epoch, self.stamp, Arc::clone(&value)));
            return value;
        }
        match multicad_core::parameters::evaluate(document) {
            Ok(parameters) => Arc::new(self.build_draft(document, &parameters)),
            Err(error) => {
                let error = error.to_string();
                Arc::new(Draft {
                    command: Err(error.clone()),
                    layout: Err(error.clone()),
                    automatic: Err(error),
                })
            }
        }
    }
    #[cfg(test)]
    pub fn request(&self, document: &Document, enabled: bool) -> Result<FenceChange, String> {
        if !enabled {
            return Err("Native modeling is unavailable".into());
        }
        let draft = self.draft(document);
        draft.layout.as_ref().map_err(Clone::clone)?;
        match draft.command.clone()? {
            Intent::Local(command) => Ok(*command),
            Intent::Symbol(_) => Err("Use the complete pinned construction transaction".into()),
        }
    }
    pub fn model_request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<ModelRequest, String> {
        if !enabled {
            return Err("Native modeling is unavailable".into());
        }
        let draft = self.draft(document);
        draft.layout.as_ref().map_err(Clone::clone)?;
        match draft.command.clone()? {
            Intent::Local(command) => Ok(ModelRequest::Fence(command)),
            Intent::Symbol(command) => Ok(ModelRequest::SymbolConstruction(Box::new(command))),
        }
    }
    fn symbol_context(
        &self,
        document: &Document,
    ) -> Result<multicad_core::symbols::architecture::ArchitectureContext, String> {
        let ids = self.outline.vertex_ids();
        let points = self.outline.dimensions(document)?;
        if ids.len() != points.len() {
            return Err("Path identities do not match its corners".into());
        }
        Ok(
            multicad_core::symbols::architecture::ArchitectureContext::Fence {
                path: FencePath {
                    vertices: points
                        .into_iter()
                        .zip(ids)
                        .map(|(point, id)| FenceVertex { id: *id, point })
                        .collect(),
                    closed: self.outline.closed(),
                    base_z: self.base_z.resolve(document)?,
                },
                level: self.level,
                post_positions: self
                    .positions
                    .iter()
                    .map(|row| {
                        row.offset
                            .resolve(document)
                            .map(|offset| FencePostPosition {
                                post: row.post,
                                offset,
                            })
                    })
                    .collect::<Result<_, _>>()?,
            },
        )
    }
    fn plane_z(&self) -> f64 {
        let Some(source) = &self.source else {
            return 0.;
        };
        let document = source.session().document();
        let parameters = source.session().parameters();
        let level = self
            .level
            .and_then(|id| resolve_level(document, parameters, id).ok())
            .map_or(0., |l| l.elevation_mm);
        level
            + self
                .base_z
                .resolve(document)
                .ok()
                .and_then(|d| d.value_in(Unit::Millimeter, Some(parameters)).ok())
                .unwrap_or(0.)
    }
    pub fn viewport(&mut self, session: &Session, view: TerminationViewport<'_>) -> bool {
        self.sync_source(session);
        let z = self.plane_z();
        let changed = self
            .outline
            .viewport(session.document(), &view, z, &self.targets);
        if changed {
            self.invalidate();
        }
        changed
    }
    fn choose_type(&mut self, document: &Document, id: EntityId) -> Result<(), String> {
        let source = read_fence_type(document, id).map_err(|e| e.to_string())?;
        self.construction = ConstructionEditor::existing(document, &source)?;
        if let Some(snapshot) = &self.source {
            self.construction.sync_source(snapshot.session());
        }
        self.type_name = document
            .get(id)
            .ok_or("Construction is missing")?
            .name
            .clone();
        self.selected_type = Some(id);
        self.shared = false;
        self.users = fence_type_users(document, id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter_map(|id| document.get(id).map(|e| e.name.clone()))
            .collect();
        Ok(())
    }
    fn post_label(&self, key: FencePostKey) -> String {
        match key {
            FencePostKey::Vertex { .. } => "Path vertex".into(),
            FencePostKey::Interior {
                start,
                end,
                ordinal,
            } => {
                let ids = self.outline.vertex_ids();
                let index = (0..ids.len()).find(|i| {
                    let next = *i + 1;
                    ids[*i] == start
                        && if next < ids.len() {
                            ids[next] == end
                        } else {
                            self.outline.closed() && ids[0] == end
                        }
                });
                match index {
                    Some(i) => format!("Segment {} · interior post {}", i + 1, ordinal),
                    None => format!("Previous segment · post {} (repair)", ordinal),
                }
            }
        }
    }
    fn post_positions(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let draft = self.draft(document);
        let mut changed = false;
        let choices = draft
            .automatic
            .as_ref()
            .ok()
            .map(|p| {
                p.posts
                    .iter()
                    .filter(|p| matches!(p.key, FencePostKey::Interior { .. }))
                    .map(|p| (p.key, self.post_label(p.key)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        egui::CollapsingHeader::new("Adjust individual post positions").show(ui,|ui|{
            ui.small("Offsets are center distances from each segment start. Move path vertices in the viewport.");
            if self.selected_post.is_none_or(|key|!choices.iter().any(|(k,_)|*k==key)){self.selected_post=choices.first().map(|(key,_)|*key);}
            egui::ComboBox::from_id_salt("override-target").selected_text(self.selected_post.map(|key|self.post_label(key)).unwrap_or("No interior posts".into())).show_ui(ui,|ui|{for(key,label)in&choices{ui.selectable_value(&mut self.selected_post,Some(*key),label);}});
            if ui.add_enabled(self.selected_post.is_some_and(|key|!self.positions.iter().any(|p|p.post==key)),egui::Button::new("Adjust selected post")).clicked()&&let Some(key)=self.selected_post&&let Some(plan)=draft.automatic.as_ref().ok(){
                let post=plan.posts.iter().find(|p|p.key==key).expect("selected post");let FencePostKey::Interior{start,..}=key else{unreachable!()};if let Ok(points)=self.outline.positions(document)&&let Some(i)=self.outline.vertex_ids().iter().position(|id|*id==start){let offset=(post.position_mm[0]-points[i][0]).hypot(post.position_mm[1]-points[i][1]);self.positions.push(PositionRow{post:key,offset:field(offset)});changed=true;}
            }
            let labels=self.positions.iter().map(|p|self.post_label(p.post)).collect::<Vec<_>>();let mut remove=None;
            for(index,row)in self.positions.iter_mut().enumerate(){ui.push_id(index,|ui|{ui.label(&labels[index]);egui::Grid::new("post-position").num_columns(2).show(ui,|ui|{changed|=row.offset.show(ui,"Offset along segment","Explicit center distance from the directed segment start");ui.end_row();});if ui.button("Restore automatic position").clicked(){remove=Some(index);}});}
            if let Some(index)=remove{self.positions.remove(index);changed=true;}
        });
        changed
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState,
    ) -> Option<ModelEvent> {
        ui.heading(if self.editing {
            "Edit fence run"
        } else {
            "Draw fence"
        });
        let mut changed = ui.text_edit_singleline(&mut self.name).changed();
        match self.symbol.show_choice(ui, document) {
            Ok(value) => changed |= value,
            Err(error) => self.error = Some(error),
        }
        if self.symbol.active() {
            changed |= self.symbol.show_fields(ui);
        } else {
            let mut selected = self.selected_type;
            egui::ComboBox::from_id_salt("fence-type")
                .selected_text(
                    self.selected_type
                        .and_then(|id| self.types.iter().find(|(key, _)| *key == id))
                        .map_or("New construction", |(_, name)| name.as_str()),
                )
                .width(controls::inspector_width(ui))
                .show_ui(ui, |ui| {
                    for (id, name) in &self.types {
                        ui.selectable_value(&mut selected, Some(*id), name);
                    }
                });
            if selected != self.selected_type
                && let Some(id) = selected
            {
                match self.choose_type(document, id) {
                    Ok(()) => changed = true,
                    Err(error) => self.error = Some(error),
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button("New construction").clicked() {
                    match ConstructionEditor::new(document) {
                        Ok(mut editor) => {
                            if let Some(source) = &self.source {
                                editor.sync_source(source.session());
                            }
                            self.construction = editor;
                            self.selected_type = None;
                            self.new_type = EntityId::new();
                            self.type_name = "Fence construction".into();
                            self.shared = false;
                            changed = true;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if self.selected_type.is_some() && ui.button("Duplicate as separate type").clicked()
                {
                    self.selected_type = None;
                    self.new_type = EntityId::new();
                    self.type_name.push_str(" copy");
                    self.shared = false;
                    changed = true;
                }
            });
            if let Some(id) = self.selected_type {
                if !self.shared && ui.button("Edit shared construction").clicked() {
                    self.shared = true;
                    changed = true;
                }
                if self.shared {
                    ui.small(format!(
                        "Changes also affect {} existing run(s)",
                        self.users.len()
                    ));
                    for name in &self.users {
                        ui.small(name);
                    }
                    if ui.button("Discard construction edits").clicked() {
                        match self.choose_type(document, id) {
                            Ok(()) => changed = true,
                            Err(error) => self.error = Some(error),
                        }
                    }
                }
            }
            if self.selected_type.is_none() || self.shared {
                ui.label("Construction name");
                changed |= ui.text_edit_singleline(&mut self.type_name).changed();
            }
            egui::CollapsingHeader::new("Construction and representative bay")
                .default_open(true)
                .show(ui, |ui| {
                    changed |= self.construction.show(
                        ui,
                        document,
                        self.selected_type.is_none() || self.shared,
                    );
                });
        }
        ui.separator();
        ui.label("Run placement");
        egui::ComboBox::from_id_salt("fence-level")
            .selected_text(
                self.level
                    .and_then(|id| self.levels.iter().find(|(key, _)| *key == id))
                    .map_or("World plane", |(_, name)| name.as_str()),
            )
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut self.level, None, "World plane")
                    .changed();
                for (id, name) in &self.levels {
                    changed |= ui
                        .selectable_value(&mut self.level, Some(*id), name)
                        .changed();
                }
            });
        egui::Grid::new("fence-placement")
            .num_columns(2)
            .show(ui, |ui| {
                changed |=
                    self.base_z
                        .show(ui, "Base above level", "World Z when no level is selected");
                ui.end_row();
            });
        if !self.symbol.active() {
            changed |= ui
                .checkbox(
                    &mut self.height_override,
                    "Override post height for this run",
                )
                .changed();
            if self.height_override {
                egui::Grid::new("fence-height-override")
                    .num_columns(2)
                    .show(ui, |ui| {
                        changed |= self.height.show(
                            ui,
                            "Post height",
                            "Infill keeps its construction dimensions",
                        );
                        ui.end_row();
                    });
            }
            changed |= ui
                .checkbox(&mut self.spacing_override, "Override spacing for this run")
                .changed();
            if self.spacing_override {
                egui::Grid::new("fence-spacing-override")
                    .num_columns(2)
                    .show(ui, |ui| {
                        changed |= self.spacing.show(
                            ui,
                            "Post spacing",
                            "Center pitch before explicit post adjustments",
                        );
                        ui.end_row();
                    });
            }
        }
        changed |= self.outline.show(ui, document, "Horizontal fence plane");
        if changed {
            *self.cache.get_mut() = None;
        }
        if self.post_positions(ui, document) {
            changed = true;
            *self.cache.get_mut() = None;
        }
        let draft = self.draft(document);
        if let Ok(layout) = &draft.layout {
            ui.small(format!(
                "{} posts · {} bays · {} infill members",
                layout.posts.len(),
                layout.bays.len(),
                layout.infills.len()
            ));
            egui::CollapsingHeader::new("Actual bay widths").show(ui, |ui| {
                for (index, bay) in layout.bays.iter().enumerate() {
                    ui.small(format!(
                        "Bay {}: {} center spacing · {} clear",
                        index + 1,
                        crate::preferences::format_mm(bay.center_spacing_mm),
                        crate::preferences::format_mm(bay.clear_width_mm)
                    ));
                }
            });
        } else if let Err(error) = &draft.layout {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        form::preview_status(ui, self.stamp, preview);
        let valid =
            enabled && draft.layout.is_ok() && draft.command.is_ok() && !self.outline.capturing();
        let action = form::actions(ui, valid, changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.invalidate();
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => match self.model_request(document, enabled) {
                Ok(request) => Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request,
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply) if preview.ready(self.stamp) && valid => {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}

pub(crate) fn fence_target(document: &Document, id: EntityId) -> Option<EntityId> {
    FencePanel::target(document, id)
}
