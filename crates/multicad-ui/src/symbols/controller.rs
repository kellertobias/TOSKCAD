use super::{SymbolAction, SymbolOutcome, SymbolSelection, SymbolTool, forms::Form};
use crate::{
    auxiliary_preview::Schedule,
    egui,
    inspector_actions::{self, Button, Owner},
    viewport::{Scene, SceneBackend, SceneStamp, VisibilityFilter},
};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session, SessionStamp},
    document::EntityId,
    project_file::{ContentHash, ProjectResources},
    symbols::SymbolDefinition,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
struct Scope {
    stamp: SessionStamp,
    resources: ProjectResources,
    catalogue: Arc<EmbeddedCatalogue>,
}
struct EmbeddedCatalogue {
    embedded: Vec<(EntityId, SymbolDefinition)>,
    embedded_pins: BTreeMap<EntityId, ContentHash>,
    embedded_search: BTreeMap<EntityId, super::installed::SearchIndex>,
}
struct Candidate {
    prepared: Prepared,
    source: ContentHash,
    stamp: SceneStamp,
    version: u64,
    selected: Vec<EntityId>,
    pending: bool,
    scene: Option<Arc<Scene>>,
}
struct Handoff {
    document: EntityId,
    revision: u64,
    resources: ProjectResources,
    scene: Arc<Scene>,
}
#[derive(Default)]
pub struct SymbolController {
    pub(super) form: Option<Form>,
    scope: Option<Scope>,
    embedded_cache: Option<(SessionStamp, Arc<EmbeddedCatalogue>)>,
    auto_preview: Schedule,
    backend: Option<Box<dyn SceneBackend>>,
    candidate: Option<Candidate>,
    handoff: Option<Handoff>,
    pub(super) library: Vec<SymbolDefinition>,
    installed: super::installed::Installed,
    library_browser: super::inspector::LibraryBrowser,
    library_pins: Vec<ContentHash>,
    library_search: Vec<super::installed::SearchIndex>,
    actions: VecDeque<SymbolAction>,
    generation: u64,
    library_epoch: u64,
    placement: crate::modeling::gizmo::Interaction,
    placement_source: Option<(Arc<Scene>, multicad_core::document::Placement, EntityId)>,
    placement_display: Option<Arc<Scene>>,
    pub(super) error: Option<String>,
}
impl SymbolController {
    pub fn set_installed_library(&mut self, entries: Arc<Vec<crate::library::LibraryEntry>>) {
        if self.installed.set_entries(entries) {
            self.library_browser.reset_catalogue();
        }
    }
    pub fn set_backend(&mut self, backend: Box<dyn SceneBackend>) {
        self.cancel();
        self.backend = Some(backend);
    }
    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
    pub fn set_library_snapshot(
        &mut self,
        definitions: Vec<SymbolDefinition>,
    ) -> Result<(), String> {
        self.set_library_file_snapshot(definitions, "")
    }
    pub(crate) fn set_library_file_snapshot(
        &mut self,
        definitions: Vec<SymbolDefinition>,
        file_text: &str,
    ) -> Result<(), String> {
        if definitions.len() > 256 {
            return Err("A library snapshot may contain at most 256 definitions".into());
        }
        let mut entries = std::collections::BTreeMap::new();
        for definition in definitions {
            let pin = definition.pin().map_err(|e| e.to_string())?;
            let key = (definition.symbol, definition.revision.clone());
            if entries
                .get(&key)
                .is_some_and(|(previous, _)| *previous != pin)
            {
                return Err(
                    "Two different library contents claim the same symbol and revision".into(),
                );
            }
            entries.insert(key, (pin, definition));
        }
        let (pins, definitions) = entries.into_values().unzip();
        self.library_pins = pins;
        self.library = definitions;
        self.library_browser.reset_catalogue();
        self.library_search = self
            .library
            .iter()
            .map(|definition| {
                crate::library::facets::definition_search_in_file(definition, file_text)
            })
            .collect();
        Ok(())
    }
    pub fn active(&self) -> bool {
        self.form.is_some()
    }
    pub(crate) fn library_placement_intent(&self) -> Option<u64> {
        self.form
            .as_ref()
            .filter(|form| matches!(form.operation, super::forms::Operation::Library))
            .map(|_| self.library_epoch)
    }
    pub(crate) fn library_active(&self) -> bool {
        self.form
            .as_ref()
            .is_some_and(|form| form.tool == SymbolTool::Library)
    }
    pub fn dirty(&self) -> bool {
        self.form.as_ref().is_some_and(Form::authoring)
    }
    pub fn pending(&self) -> bool {
        self.candidate.as_ref().is_some_and(|c| c.pending)
    }
    pub fn label(&self) -> &'static str {
        self.form.as_ref().map_or("Symbols", |f| f.tool.label())
    }
    pub fn begin(
        &mut self,
        kind: SymbolTool,
        session: &Session,
        resources: &ProjectResources,
        _scene: Option<&Arc<Scene>>,
        selection: SymbolSelection,
        _channel: Channel,
    ) -> Result<(), String> {
        let form = Form::new(kind, session.document(), selection)?;
        self.open(form, session, resources)
    }
    pub fn begin_existing(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        _scene: Option<&Arc<Scene>>,
        instance: EntityId,
        _channel: Channel,
    ) -> Result<(), String> {
        self.open(
            Form::existing(session.document(), instance)?,
            session,
            resources,
        )
    }
    fn open(
        &mut self,
        form: Form,
        session: &Session,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        if self.active() {
            return Err("Apply or cancel the active symbol controls first".into());
        }
        if form.authoring() && !self.has_backend() {
            return Err("Native symbol preview is unavailable".into());
        }
        let stamp = session.stamp();
        let catalogue = if let Some((_, catalogue)) = self
            .embedded_cache
            .as_ref()
            .filter(|(cached, _)| *cached == stamp)
        {
            Arc::clone(catalogue)
        } else {
            let embedded = multicad_core::symbols::definitions(session.document())
                .map_err(|error| error.to_string())?;
            let embedded_search = embedded
                .iter()
                .map(|(id, definition)| (*id, super::installed::definition_search(definition)))
                .collect();
            let embedded_pins = embedded
                .iter()
                .map(|(id, definition)| {
                    definition
                        .pin()
                        .map(|pin| (*id, pin))
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<_, _>>()?;
            let catalogue = Arc::new(EmbeddedCatalogue {
                embedded,
                embedded_search,
                embedded_pins,
            });
            self.embedded_cache = Some((stamp, Arc::clone(&catalogue)));
            catalogue
        };
        self.scope = Some(Scope {
            stamp,
            resources: resources.clone(),
            catalogue,
        });
        self.library_epoch = self
            .library_epoch
            .checked_add(1)
            .ok_or("Library request limit reached")?;
        self.form = Some(form);
        self.library_browser = Default::default();
        self.auto_preview.reset();
        self.candidate = None;
        self.handoff = None;
        self.error = None;
        Ok(())
    }
    fn current(&self, session: &Session, resources: &ProjectResources) -> Result<(), String> {
        let scope = self.scope.as_ref().ok_or("Open symbol controls first")?;
        session.validate_stamp(&scope.stamp).map_err(|_| {
            "The source session changed; cancel and reopen these symbol controls".to_string()
        })?;
        if &scope.resources != resources {
            return Err(
                "Source or project resources changed; cancel and reopen the symbol controls".into(),
            );
        }
        Ok(())
    }
    pub(crate) fn tick_auto_preview(
        &mut self,
        ui: &egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        automatic: bool,
    ) {
        let version = self
            .form
            .as_ref()
            .filter(|form| form.authoring())
            .map(|form| form.version);
        let blocked = self.placement.active();
        if self.auto_preview.ready(ui, automatic, version, blocked)
            && !self
                .candidate
                .as_ref()
                .is_some_and(|candidate| Some(candidate.version) == version)
        {
            let _ = self.preview(session, resources, Channel::Ui);
        }
    }

    pub fn preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.invalidate();
        if let Some(form) = &self.form {
            self.auto_preview.attempted(form.version);
        }
        let result = self.request_preview(session, resources, channel);
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }
    fn request_preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.current(session, resources)?;
        let form = self.form.as_mut().ok_or("Open symbol controls first")?;
        let result = form.prepare(session, Actor::local(channel))?;
        if result.prepared.document() == session.document() {
            return Err(
                "This exact definition is already embedded. Select it in Library to place it"
                    .into(),
            );
        }
        let selected = if result.instances.is_empty() {
            vec![result.definition]
        } else {
            result.instances
        };
        let source = ContentHash::document(result.prepared.document())?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Symbol preview generation limit reached")?;
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: self.generation,
        };
        self.backend
            .as_mut()
            .ok_or("Native symbol preview is unavailable")?
            .request_evaluation(
                result.prepared.document(),
                resources,
                stamp,
                &Default::default(),
            )?;
        self.candidate = Some(Candidate {
            prepared: result.prepared,
            source,
            stamp,
            version: form.version,
            selected,
            pending: true,
            scene: None,
        });
        self.error = None;
        Ok(())
    }
    pub fn poll(&mut self, session: &Session, resources: &ProjectResources) {
        if self.active()
            && let Err(error) = self.current(session, resources)
        {
            self.invalidate();
            self.error = Some(error);
        }
        if self.handoff.as_ref().is_some_and(|h| {
            h.document != session.document().id()
                || h.revision != session.revision()
                || &h.resources != resources
        }) {
            self.handoff = None;
        }
        for _ in 0..16 {
            let Some(reply) = self.backend.as_mut().and_then(|b| b.poll()) else {
                break;
            };
            let Some(candidate) = &self.candidate else {
                continue;
            };
            if !candidate.pending || candidate.stamp != reply.stamp {
                continue;
            }
            let result = (|| {
                self.current(session, resources)?;
                let candidate = self.candidate.as_ref().expect("matching candidate");
                session
                    .validate_preview(&candidate.prepared)
                    .map_err(|e| e.to_string())?;
                if self.form.as_ref().map(|f| f.version) != Some(candidate.version) {
                    return Err("Symbol controls changed; preview them again".into());
                }
                let scene = reply.result?;
                super::proof::validate(candidate.prepared.document(), candidate.stamp, &scene)?;
                VisibilityFilter::default().apply(
                    &scene,
                    candidate.prepared.document(),
                    candidate.stamp.revision,
                )
            })();
            let candidate = self.candidate.as_mut().expect("matching candidate");
            candidate.pending = false;
            match result {
                Ok(scene) => {
                    candidate.scene = Some(scene);
                    self.error = None;
                }
                Err(error) => {
                    candidate.scene = None;
                    self.error = Some(error);
                }
            }
        }
    }
    pub fn can_apply(&self, session: &Session, resources: &ProjectResources) -> bool {
        self.dirty()
            && !self.placement.active()
            && self.current(session, resources).is_ok()
            && self.error.is_none()
            && self.candidate.as_ref().is_some_and(|c| {
                !c.pending
                    && c.scene.is_some()
                    && self.form.as_ref().map(|f| f.version) == Some(c.version)
                    && session.validate_preview(&c.prepared).is_ok()
            })
    }
    pub fn apply(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<SymbolOutcome, String> {
        self.current(session, resources)?;
        if !self.can_apply(session, resources) {
            return Err(
                "Wait for a successful native preview of the current symbol controls".into(),
            );
        }
        let candidate = self.candidate.take().expect("checked candidate");
        if ContentHash::document(candidate.prepared.document())? != candidate.source {
            return Err("Symbol candidate content changed; preview again".into());
        }
        let label = self.label();
        let scene = candidate.scene.expect("checked native scene");
        session
            .commit(candidate.prepared)
            .map_err(|e| e.to_string())?;
        self.cancel();
        self.handoff = Some(Handoff {
            document: session.document().id(),
            revision: session.revision(),
            resources: resources.clone(),
            scene,
        });
        Ok(SymbolOutcome::Applied {
            selected: candidate.selected,
            label,
        })
    }
    pub fn display_scene(
        &self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Arc<Scene>> {
        if self.can_apply(session, resources) {
            return self.candidate.as_ref().and_then(|c| c.scene.clone());
        }
        if self.placement.active()
            && self.current(session, resources).is_ok()
            && let Some(candidate) = &self.candidate
            && !candidate.pending
            && self.form.as_ref().map(|form| form.version) == Some(candidate.version)
        {
            return candidate.scene.clone();
        }
        if self.current(session, resources).is_ok()
            && let Some(display) = &self.placement_display
        {
            return Some(display.clone());
        }
        self.handoff
            .as_ref()
            .filter(|h| {
                h.document == session.document().id()
                    && h.revision == session.revision()
                    && &h.resources == resources
            })
            .map(|h| h.scene.clone())
    }
    pub fn canonical_ready(&mut self, session: &Session) {
        if self.handoff.as_ref().is_some_and(|h| {
            h.document == session.document().id() && h.revision <= session.revision()
        }) {
            self.handoff = None;
        }
    }
    fn invalidate(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.cancel();
        }
        self.candidate = None;
        self.placement_source = None;
        self.placement_display = None;
        self.error = None;
    }
    pub fn cancel(&mut self) {
        self.invalidate();
        self.form = None;
        self.auto_preview.reset();
        self.placement = Default::default();
        self.scope = None;
        self.handoff = None;
        self.actions.clear();
    }
    pub fn place_loaded_library(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        self.current(session, resources)?;
        if self.library_placement_intent().is_none() {
            return Err("Open Library before placing a listed component".into());
        }
        if self.library.len() == 1 {
            let content = &self.library[0];
            self.form
                .as_mut()
                .expect("open Library")
                .place_library(session.document(), content)?;
            self.preview(session, resources, Channel::Ui)?;
        }
        Ok(())
    }
    /// Display handles only edit the retained draft. Releasing requests native
    /// validation; committing still requires the explicit Inspector Apply.
    pub(crate) fn placement_viewport(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        viewport: crate::modeling::PlacementViewport<'_>,
    ) -> Result<bool, String> {
        if self
            .form
            .as_mut()
            .and_then(Form::placement_pose_mut)
            .is_none()
        {
            return Ok(false);
        }
        self.current(session, resources)?;
        let parameters = session.parameters();
        let form = self.form.as_mut().expect("placement form");
        let source = form
            .placement_pose_mut()
            .expect("placement pose")
            .numeric()?;
        let gesture = match self.placement.show_with_help(
            ui,
            &viewport,
            &source,
            parameters,
            "drag to adjust the component; release to validate; choose Apply to place",
        ) {
            Ok(gesture) => gesture,
            Err(error) => {
                self.placement = Default::default();
                self.invalidate();
                self.error = Some(error.clone());
                return Err(error);
            }
        };
        if gesture.started {
            self.placement_source = self.candidate.as_ref().and_then(|candidate| {
                let instance = *candidate.selected.first()?;
                let multicad_core::document::EntityData::Instance { placement, .. } =
                    &candidate.prepared.document().get(instance)?.data
                else {
                    return None;
                };
                Some((candidate.scene.clone()?, *placement, instance))
            });
        }
        if let Some(value) = gesture.changed {
            let pose = form.placement_pose_mut().expect("placement pose");
            pose.set_numeric(&value, parameters)?;
            let new_pose = multicad_core::model::RigidPose::from_placement(pose.resolve()?)
                .map_err(|e| e.to_string())?;
            form.changed();
            let base = self.placement_source.take();
            self.invalidate();
            if let Some((scene, old_pose, instance)) = &base {
                let delta = multicad_core::model::RigidPose::from_placement(*old_pose)
                    .and_then(|old| old.inverse())
                    .and_then(|old| old.then(new_pose))
                    .map_err(|e| e.to_string())?;
                self.placement_display = Some(scene.presentation_placement(*instance, delta)?);
            }
            self.placement_source = base;
        }
        if gesture.released && gesture.moved {
            let display = self.placement_display.take();
            self.preview(session, resources, Channel::Ui)?;
            self.placement_display = display;
        }
        Ok(gesture.input_owned)
    }
    pub fn take_action(&mut self) -> Option<SymbolAction> {
        self.actions.pop_front()
    }
    pub(crate) fn show_library_modal(
        &mut self,
        ctx: &egui::Context,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Option<SymbolOutcome> {
        if !self.library_active() {
            return None;
        }
        let current = self.current(session, resources);
        let failure = current
            .as_ref()
            .err()
            .map(String::as_str)
            .or(self.error.as_deref());
        let version = self.form.as_ref()?.version;
        let catalogue = &self.scope.as_ref()?.catalogue;
        let size = ctx.content_rect().size();
        let id = egui::Id::new(("symbol-library", session.document().id()));
        let modal = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).fade_in(false))
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .inner_margin(16)
                    .corner_radius(8),
            )
            .backdrop_color(egui::Color32::from_black_alpha(100))
            .show(ctx, |ui| {
                ui.set_width((size.x - 56.).clamp(1., 760.));
                ui.set_height((size.y - 56.).clamp(1., 540.));
                super::inspector::show_library(
                    self.form.as_mut().expect("open Library"),
                    &mut self.library_browser,
                    ui,
                    session,
                    super::inspector::Catalogue {
                        embedded: &catalogue.embedded,
                        embedded_pins: &catalogue.embedded_pins,
                        embedded_search: &catalogue.embedded_search,
                        library: &self.library,
                        library_pins: &self.library_pins,
                        library_search: &self.library_search,
                        installed: &mut self.installed,
                    },
                    current.is_ok(),
                    failure,
                )
            });
        let close = modal.should_close();
        let (action, error, close_button) = modal.inner;
        if close || close_button {
            self.cancel();
            return Some(SymbolOutcome::Cancelled);
        }
        if self
            .form
            .as_ref()
            .is_some_and(|form| form.version != version)
        {
            self.invalidate();
        }
        if let Some(action) = action {
            self.actions.push_back(action);
        }
        if let Some(error) = error {
            self.error = Some(error);
        }
        if self
            .form
            .as_mut()
            .and_then(Form::placement_pose_mut)
            .is_some()
        {
            let _ = self.preview(session, resources, Channel::Ui);
        }
        None
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        enabled: bool,
    ) -> Option<SymbolOutcome> {
        if self.library_active() {
            return None;
        }
        let current = self.current(session, resources).is_ok();
        let pending = self.pending();
        let version = self.form.as_ref().map(|f| f.version);
        let (action, error) = super::inspector::show(
            self.form.as_mut()?,
            ui,
            session,
            super::inspector::Catalogue {
                embedded: &self.scope.as_ref()?.catalogue.embedded,
                embedded_pins: &self.scope.as_ref()?.catalogue.embedded_pins,
                embedded_search: &self.scope.as_ref()?.catalogue.embedded_search,
                library: &self.library,
                library_pins: &self.library_pins,
                library_search: &self.library_search,
                installed: &mut self.installed,
            },
            enabled && current && (!pending || self.auto_preview.automatic()),
        );
        if self.form.as_ref().map(|f| f.version) != version {
            self.invalidate();
        }
        if let Some(action) = action {
            self.actions.push_back(action);
        }
        if let Some(error) = error {
            self.error = Some(error);
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if pending {
            ui.small("Evaluating exact symbol geometry…");
        }
        let clicks = inspector_actions::row(
            ui,
            Owner::Symbols,
            (!self.auto_preview.automatic()).then(|| {
                Button::new(
                    "Preview",
                    enabled && current && !pending && self.dirty(),
                    "Rebuild the complete candidate before Apply",
                )
            }),
            Button::new(
                "Apply",
                enabled && self.can_apply(session, resources),
                "Commit one native-checked symbol transaction",
            ),
            Button::new("Cancel", true, "Discard these uncommitted symbol controls"),
        );
        if clicks.cancel {
            self.cancel();
            return Some(SymbolOutcome::Cancelled);
        }
        if clicks.preview {
            let _ = self.preview(session, resources, Channel::Ui);
        }
        if clicks.apply {
            match self.apply(session, resources) {
                Ok(outcome) => return Some(outcome),
                Err(error) => self.error = Some(error),
            }
        }
        None
    }
}
