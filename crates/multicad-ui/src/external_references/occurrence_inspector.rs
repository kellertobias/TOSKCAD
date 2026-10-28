use super::{
    controller::{Controller, Proposal, Purpose},
    occurrence_form::{Consumer, geometry_label, occurrence_label, source_history},
    queries::Task,
};
use crate::{
    controls, egui,
    inspector_actions::{self, Button, Owner},
    viewport::PickFilter,
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::EntityId,
    project_file::ProjectResources,
    references::ReferenceReport,
    sketch_external::{self, Behavior, QueryMethod},
};
impl Controller {
    pub(super) fn occurrence_inspector(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        enabled: bool,
        resources: &ProjectResources,
    ) -> Option<Proposal> {
        controls::inspector_layout(ui);
        ui.strong("External references");
        let pending = self.query_pending();
        let mut load = None;
        let mut results_changed = false;
        let mut geometry_changed = false;
        let mut choice = None;
        if let Some(active) = &mut self.active
            && let Some(form) = &mut active.form
        {
            ui.label(if matches!(active.purpose, Purpose::Repair(_)) {
                "Replace source or exact result"
            } else {
                "Choose source geometry and its exact result"
            });
            ui.add_enabled_ui(enabled && !pending, |ui| {
                let old_consumer = form.consumer.clone();

                let selected = match &form.consumer {
                    Consumer::Unset => "Choose sketch occurrence".into(),
                    Consumer::World => "World / definition plane".into(),
                    Consumer::Occurrence(c) => form.labels.get(c.component,&c.path).map_or_else(|| "Missing sketch occurrence".into(),|label|label.compact(ui)),
                };
                ui.label("Receiving sketch context");
                controls::inspector_combo(ui, "external-consumer-context").selected_text(selected).show_ui(ui, |ui| {
                    if ui.selectable_value(&mut form.consumer, Consumer::World, "World / definition plane").on_hover_text("Use the sketch's authored plane in world coordinates").clicked() { ui.close(); }
                    for context in &form.contexts {
                        let Some(name)=form.labels.get(context.component,&context.path) else {continue;};
                        let (label,help)=(&name.full,&name.help);
                        if ui.selectable_value(&mut form.consumer, Consumer::Occurrence(context.clone()), label).on_hover_text(help).clicked() { ui.close(); }
                    }
                }).response.on_hover_text("Choose the exact placed Part whose local coordinates contain this sketch. Repeated placements are separate choices");
                if form.consumer != old_consumer { results_changed = true; }

                ui.label("Source occurrence");
                let mut next_source = None;
                controls::inspector_combo(ui, "external-source-occurrence").selected_text(form.source.as_ref().map_or_else(|| "Choose source occurrence".into(), |s| form.labels.get(s.component,&s.path).map_or_else(|| "Missing source occurrence".into(),|label|label.compact(ui)))).show_ui(ui, |ui| {
                    for source in &form.sources {
                        let Some(name)=form.labels.get(source.component,&source.path) else {continue;};
                        let (label,help)=(&name.full,&name.help);
                        if ui.selectable_label(form.source.as_ref().is_some_and(|s| s.component == source.component && s.path == source.path), label).on_hover_text(help).clicked() {
                            next_source = Some(source.clone());
                            ui.close();
                        }
                    }
                }).response.on_hover_text("Choose the placed copy to reference before picking nested geometry. Repeated copies have distinct path labels");
                if let Some(source) = next_source { geometry_changed |= form.choose_source(&source); }
                if let Some(source) = &mut form.source {
                    let prior = source.source;
                    ui.label("Source history");
                    controls::inspector_combo(ui, "external-source-history").selected_text(if source.source == source.component { "Current Part · follows its final feature".into() } else { format!("Retained feature · {}", session.document().get(source.source).map_or("Missing", |e| e.name.as_str())) }).show_ui(ui, |ui| {
                        for id in source_history(session.document(), source) {
                            let label = if id == source.component { "Current Part · follows its final feature".into() } else { format!("Retained feature · {}", session.document().get(id).map_or("Missing", |e| e.name.as_str())) };
                            if ui.selectable_value(&mut source.source, id, label).clicked() { ui.close(); }
                        }
                    }).response.on_hover_text("Current Part follows new features. For a reference within the same Part, choose an earlier retained feature to avoid a dependency cycle");
                    if source.source != prior { geometry_changed = true; }
                    if self.native.backend.is_some() && ui.add_enabled(!self.native.pending(), egui::Button::new("Load source geometry")).on_hover_text("Inspect current native source topology even when the downstream model cannot rebuild").clicked() { load = Some(source.source); }
                }
                let prior_method = form.method;
                controls::inspector_combo(ui, "external-occurrence-method").selected_text(query_method_label(form.method)).show_ui(ui, |ui| {
                    for method in [QueryMethod::Coplanar, QueryMethod::Project, QueryMethod::Intersect] {
                        if ui.selectable_value(&mut form.method, method, query_method_label(method)).clicked() { ui.close(); }
                    }
                }).response.on_hover_text("Coplanar preserves geometry already in the plane. Project places curves onto the plane. Intersect finds finite crossings or section curves");
                if prior_method != form.method {
                    geometry_changed = true;
                    active.filter = PickFilter::Edges;
                }
                controls::inspector_combo(ui, "external-occurrence-filter").selected_text(active.filter.label()).show_ui(ui, |ui| {
                    let filters = if form.method == QueryMethod::Intersect { [PickFilter::Edges, PickFilter::Faces] } else { [PickFilter::Edges, PickFilter::Vertices] };
                    for filter in filters {
                        if ui.selectable_value(&mut active.filter, filter, filter.label()).clicked() { geometry_changed = true; ui.close(); }
                    }
                }).response.on_hover_text("Select the native geometry family to pick in the viewer");
                controls::inspector_combo(ui, "external-occurrence-behavior").selected_text(super::inspector::behavior_label(active.behavior)).show_ui(ui, |ui| {
                    for behavior in [Behavior::ReferenceOnly, Behavior::Derived] { if ui.selectable_value(&mut active.behavior, behavior, super::inspector::behavior_label(behavior)).clicked() { ui.close(); } }
                }).response.on_hover_text("Reference geometry is a construction aid. Derived geometry can close a solid profile; both stay associative");
                ui.checkbox(&mut active.include_hidden, "Pick occluded geometry").on_hover_text("Permit explicitly hidden native faces, edges or vertices behind visible surfaces");
            });
            if geometry_changed {
                form.clear_geometry();
                active.selected = None;
                active.request = None;
            } else if results_changed {
                form.clear_results();
            }
            if let Some(catalog) = &form.catalog {
                if catalog.coincident && catalog.query.method == QueryMethod::Intersect {
                    ui.colored_label(egui::Color32::LIGHT_YELLOW, "The source coincides with the sketch plane; choose a distinct finite source or projection method.");
                }
                for diagnostic in &catalog.diagnostics {
                    ui.colored_label(egui::Color32::LIGHT_YELLOW, diagnostic);
                }
                if catalog.results.is_empty() {
                    ui.label("No finite native results for this source and plane.");
                } else {
                    ui.label(format!("{} exact result(s)", catalog.results.len()));
                    let selected = catalog
                        .results
                        .iter()
                        .find(|c| form.selected.as_ref() == Some(&c.selector))
                        .map(|c| geometry_label(&c.geometry))
                        .unwrap_or_else(|| "Choose exact result".into());
                    controls::inspector_combo(ui, "external-exact-result").selected_text(selected).show_ui(ui, |ui| {
                        for (index, result) in catalog.results.iter().enumerate() {
                            if ui.add_enabled(!result.tangent && !(catalog.coincident && catalog.query.method == QueryMethod::Intersect) && !pending, egui::Button::selectable(form.selected.as_ref() == Some(&result.selector), format!("{} · {}", index + 1, geometry_label(&result.geometry)))).on_hover_text(if result.tangent { "Tangency has no unique persistent crossing. Choose another finite source/result" } else { "Use this exact result. Choose the branch you want when the source produces several results" }).clicked() { choice = Some(result.selector.clone()); ui.close(); }
                        }
                    });
                    ui.small("One result per reference. Capture additional references to complete a loop.");
                }
            } else if form.intent.is_some() {
                ui.small("Source captured. Find exact native results before choosing one.");
            } else {
                ui.small("Pick an exact edge, vertex or face in the viewer. Choose a source occurrence first for nested copies.");
            }
        }
        if geometry_changed || results_changed {
            self.queries.cancel();
            self.hover = None;
            self.error = None;
        }
        if let Some(choice) = choice
            && let Some(active) = &mut self.active
            && let Some(form) = &mut active.form
        {
            if let Err(error) = form.select_result(&choice, active.behavior) {
                self.error = Some(error);
            } else {
                self.error = None;
            }
        }
        self.update_occurrence_view(session);
        if let Some(view) = self.scene()
            && !view.is_full_model()
        {
            let full = self
                .context
                .as_ref()
                .and_then(|c| c.scene.as_ref())
                .is_some_and(|s| s.is_full_model());
            ui.colored_label(
                egui::Color32::LIGHT_YELLOW,
                if full {
                    "Selected occurrence source view"
                } else {
                    "Source definition inspection · incomplete model"
                },
            );
        }
        if pending {
            ui.small("Finding exact reference geometry…");
        }
        let label = if self
            .active
            .as_ref()
            .and_then(|a| a.form.as_ref())
            .is_some_and(|f| f.catalog.is_none())
        {
            "Find exact results"
        } else {
            "Preview reference"
        };
        let clicked = inspector_actions::row(
            ui,
            Owner::External,
            Some(Button::new(
                label,
                enabled && self.can_preview(session),
                "Query exact native geometry, select a result, then prepare the consumer sketch",
            )),
            Button::new(
                "Apply",
                false,
                "Apply becomes available after a valid sketch preview and complete model check",
            ),
            Button::new(
                "Cancel",
                true,
                "Discard this uncommitted reference operation",
            ),
        );
        if clicked.cancel {
            self.cancel();
            return None;
        }
        let result = if let Some(source) = load {
            self.request_source(session, source, resources)
                .map(|()| None)
        } else if clicked.preview {
            self.request_preview(session, resources, Channel::Ui)
        } else {
            Ok(None)
        };
        match result {
            Ok(proposal) => {
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                proposal
            }
            Err(error) => {
                self.error = Some(error.clone());
                ui.colored_label(egui::Color32::LIGHT_RED, error);
                None
            }
        }
    }
    pub(super) fn occurrence_rows(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        sketch: EntityId,
        report: Option<&ReferenceReport>,
        enabled: bool,
        resources: &ProjectResources,
    ) {
        let bindings = match sketch_external::occurrence_bindings(session.document(), sketch) {
            Ok(bindings) => bindings,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        for binding in bindings {
            ui.push_id(binding.query.reference, |ui| {
                ui.separator();
                let paths = vec![binding.query.source.path.clone()];
                let (label, help) = occurrence_label(session.document(), binding.query.source.component, &binding.query.source.path, &paths);
                ui.label(label).on_hover_text(help);
                ui.small(query_method_label(binding.query.method));
                if let Some(error) = report.filter(|r| r.revision == session.revision()).and_then(|r| r.external_errors.get(&binding.query.reference)) { ui.colored_label(egui::Color32::LIGHT_YELLOW, format!("Reference needs repair: {error}")); }
                else if report.filter(|r| r.revision == session.revision()).is_some_and(|r| r.external_results.contains_key(&binding.query.reference)) { ui.small("Current native occurrence result is available"); }
                else { ui.small("Awaiting current native occurrence evaluation"); }
                ui.add_enabled_ui(enabled && !self.query_pending(), |ui| {
                    let mut behavior = binding.behavior;
                    controls::inspector_combo(ui, "occurrence-binding-behavior").selected_text(super::inspector::behavior_label(behavior)).show_ui(ui, |ui| {
                        for value in [Behavior::ReferenceOnly, Behavior::Derived] { if ui.selectable_value(&mut behavior, value, super::inspector::behavior_label(value)).clicked() { ui.close(); } }
                    });
                    if behavior != binding.behavior {
                        let mut input = binding.clone(); input.behavior = behavior;
                        if let Err(error) = self.queries.begin(session, resources, Actor::local(Channel::Ui), Task::Prepare { input, repair: true }) { self.error = Some(error); }
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Repair reference").on_hover_text("Choose a replacement occurrence, source or exact result; compatible output identities remain unchanged").clicked() && let Err(error) = self.begin_occurrence_row(session, binding.clone()) { self.error = Some(error); }
                        if ui.button("Remove reference").on_hover_text("Prepare an atomic native-checked removal. Dependent geometry and constraints must be handled explicitly").clicked() && let Err(error) = self.remove_occurrence(session, resources, binding.query.reference) { self.error = Some(error); }
                    });
                });
            });
        }
    }
}
fn query_method_label(method: QueryMethod) -> &'static str {
    match method {
        QueryMethod::Coplanar => "Coplanar projection",
        QueryMethod::Project => "Project onto sketch plane",
        QueryMethod::Intersect => "Plane intersection",
    }
}
