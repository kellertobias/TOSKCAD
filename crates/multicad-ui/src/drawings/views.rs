use super::*;
use multicad_core::{
    document::{Document, EntityData},
    drawings::{DrawingView, Scale, ViewEvaluation, ViewKind, ViewSource},
    references::ReferenceFrame,
};
impl DrawingEditor {
    pub fn set_backend_factory(&mut self, factory: DrawingBackendFactory) {
        self.feed.set_factory(factory);
        self.preview_input = None;
    }
    pub fn has_backend_factory(&self) -> bool {
        self.feed.has_factory()
    }
    pub(super) fn preview_matches(&self, session: &Session) -> bool {
        self.preview_input
            .as_ref()
            .is_some_and(|(doc, rev, book, draft)| {
                *doc == session.document().id()
                    && *rev == session.revision()
                    && *book == self.book
                    && draft == &self.draft
            })
    }
    fn candidate_document(&self, session: &Session) -> Result<Document, String> {
        let (Some(id), Some(draft)) = (self.book, self.draft.as_ref()) else {
            return Ok(session.document().clone());
        };
        let mut book = drawings::read(session.document(), id).map_err(|e| e.to_string())?;
        let page = book
            .pages
            .iter_mut()
            .find(|p| p.id == draft.id)
            .ok_or("Drawing page is missing")?;
        *page = draft.clone();
        let edits =
            drawings::replace_edits(session.document(), id, &book).map_err(|e| e.to_string())?;
        session.document().edited(&edits).map_err(|e| e.to_string())
    }
    pub(super) fn tick_views(&mut self, ctx: &egui::Context, session: &Session) {
        let has_views = self.draft.as_ref().is_some_and(|p| !p.views.is_empty());
        if !has_views {
            self.feed.clear();
            self.preview_input = None;
            return;
        }
        if !self.preview_matches(session) {
            self.preview_input = Some((
                session.document().id(),
                session.revision(),
                self.book,
                self.draft.clone(),
            ));
            match self.candidate_document(session).and_then(|document| {
                self.feed
                    .schedule(document, session.revision(), ctx.input(|i| i.time))
            }) {
                Ok(()) => {}
                Err(error) => {
                    self.feed.clear();
                    self.feed.error = Some(error);
                }
            }
        }
        self.feed.tick(ctx.input(|i| i.time));
        if self.feed.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
    pub(super) fn changed_views_ready(&self, session: &Session) -> bool {
        if self.draft.as_ref().map(|p| &p.views) == self.original.as_ref().map(|p| &p.views) {
            return true;
        }
        self.preview_matches(session)
            && self.feed.result.as_ref().is_some_and(|result| {
                self.draft.as_ref().is_some_and(|p| {
                    p.views.iter().all(|v| {
                        matches!(result.views.get(&v.id), Some(ViewEvaluation::Ready { .. }))
                    })
                })
            })
    }
    pub(super) fn add_view(&mut self, kind: ViewKind) -> Result<(), String> {
        let page = self.draft.as_mut().ok_or("Add a page first")?;
        let id = EntityId::new();
        page.views.push(DrawingView {
            id,
            name: format!("View {}", page.views.len() + 1),
            source: ViewSource::Model { entities: vec![] },
            frame: ReferenceFrame {
                origin_mm: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
            kind,
            position_mm: [
                page.settings.dimensions()[0] * 0.5,
                page.settings.dimensions()[1] * 0.4,
            ],
            scale: page.settings.scale,
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.01,
        });
        self.selected_view = Some(id);
        Ok(())
    }
    pub(super) fn fit_view(&mut self, id: EntityId, session: &Session) -> Result<(), String> {
        if !self.preview_matches(session) {
            return Err("Wait for the current native drawing preview".into());
        }
        let result = self
            .feed
            .result
            .as_ref()
            .ok_or("Wait for the current native drawing preview")?;
        let Some(ViewEvaluation::Ready { vectors, .. }) = result.views.get(&id) else {
            return Err("Resolve this view before fitting it".into());
        };
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in vectors.iter().flat_map(|v| &v.preview_mm) {
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        let page = self.draft.as_mut().ok_or("Select a page")?;
        let [w, h] = page.settings.dimensions();
        let margin = page.settings.margin_mm + 8.;
        let bottom = if page.settings.title_block {
            h - page.settings.margin_mm - 40.
        } else {
            h - margin
        };
        let available = [w - 2. * margin, bottom - margin];
        if available.iter().any(|v| *v <= 0.) {
            return Err("Paper has no space for a view".into());
        }
        let view = page
            .views
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or("View is missing")?;
        if let ViewKind::Detail {
            center_mm,
            radius_mm,
        } = view.kind
        {
            min = [center_mm[0] - radius_mm, center_mm[1] - radius_mm];
            max = [center_mm[0] + radius_mm, center_mm[1] + radius_mm];
        }
        let needed = ((max[0] - min[0]) / available[0])
            .max((max[1] - min[1]) / available[1])
            .ceil()
            .max(1.);
        if needed > 100000. {
            return Err("View needs a larger sheet or a manually configured scale".into());
        }
        let denominator = needed as u32;
        let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
        for i in 0..3 {
            view.frame.origin_mm[i] += view.frame.u[i] * center[0] + view.frame.v[i] * center[1];
        }
        if let ViewKind::Detail { center_mm, .. } = &mut view.kind {
            center_mm[0] -= center[0];
            center_mm[1] -= center[1];
        }
        view.scale = Scale {
            numerator: 1,
            denominator,
        };
        view.position_mm = [w * 0.5, (margin + bottom) * 0.5];
        Ok(())
    }
    pub(super) fn views_inspector(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        if ui
            .button("Page settings")
            .on_hover_text("Edit paper, title block and text notes")
            .clicked()
        {
            self.selected_view = None;
            return;
        }
        let Some(page) = self.draft.as_mut() else {
            return;
        };
        ui.heading("Drawing views");
        for view in &page.views {
            if ui
                .selectable_label(self.selected_view == Some(view.id), &view.name)
                .on_hover_text("Edit this linked model view")
                .clicked()
            {
                self.selected_view = Some(view.id);
            }
        }
        let Some(view) = page
            .views
            .iter_mut()
            .find(|v| Some(v.id) == self.selected_view)
        else {
            self.selected_view = None;
            return;
        };
        let id = view.id;
        ui.separator();
        ui.label("View name");
        ui.text_edit_singleline(&mut view.name)
            .on_hover_text("Name of this view; source identity stays unchanged");
        ui.strong("Model sources");
        if let ViewSource::Model { entities } = &mut view.source {
            let mut chosen: std::collections::BTreeSet<_> = entities.iter().copied().collect();
            egui::ScrollArea::vertical().id_salt("view-sources").max_height(150.).show(ui,|ui|{for e in session.document().entities(){if matches!(e.data,EntityData::Part{..}|EntityData::Instance{..}|EntityData::Assembly{..}|EntityData::Group{..}){let mut selected=chosen.contains(&e.id);if ui.checkbox(&mut selected,&e.name).on_hover_text("Include this current model source independently of viewport visibility").changed(){if selected{chosen.insert(e.id);}else{chosen.remove(&e.id);}}}}});
            *entities = chosen.into_iter().collect();
        } else {
            ui.label("This view uses a registered source producer.");
        }
        ui.label("Orientation");
        crate::controls::inspector_combo(ui, "view-direction")
            .selected_text("Choose standard view…")
            .show_ui(ui, |ui| {
                for (label, u, v) in [
                    ("Top (+Z)", [1., 0., 0.], [0., 1., 0.]),
                    ("Bottom (−Z)", [-1., 0., 0.], [0., 1., 0.]),
                    ("Front (−Y)", [1., 0., 0.], [0., 0., 1.]),
                    ("Back (+Y)", [-1., 0., 0.], [0., 0., 1.]),
                    ("Right (+X)", [0., 1., 0.], [0., 0., 1.]),
                    ("Left (−X)", [0., -1., 0.], [0., 0., 1.]),
                    (
                        "Isometric",
                        [2f64.sqrt().recip(), -2f64.sqrt().recip(), 0.],
                        [6f64.sqrt().recip(), 6f64.sqrt().recip(), 2. / 6f64.sqrt()],
                    ),
                ] {
                    if ui.button(label).clicked() {
                        view.frame.u = u;
                        view.frame.v = v;
                        ui.close();
                    }
                }
            });
        ui.label("View origin X / Y / Z (model mm)");
        ui.horizontal_wrapped(|ui| {
            for x in &mut view.frame.origin_mm {
                ui.add(egui::DragValue::new(x).speed(1.))
                    .on_hover_text("World-space origin of the projection frame");
            }
        });
        ui.label("Paper position X / Y (mm)");
        ui.horizontal_wrapped(|ui| {
            for x in &mut view.position_mm {
                ui.add(egui::DragValue::new(x).speed(1.))
                    .on_hover_text("View anchor measured from the page's top left");
            }
        });
        ui.label("View scale (paper : model)");
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::DragValue::new(&mut view.scale.numerator).range(1..=100000))
                .on_hover_text("Independent view scale numerator");
            ui.label(":");
            ui.add(egui::DragValue::new(&mut view.scale.denominator).range(1..=100000))
                .on_hover_text("Independent view scale denominator");
        });
        ui.checkbox(&mut view.hidden_lines, "Show hidden edges")
            .on_hover_text("Generate native occluded edges and display them dashed");
        ui.checkbox(&mut view.tangent_edges, "Show tangent edges")
            .on_hover_text("Include smooth edges between adjacent surfaces");
        match &mut view.kind {
            ViewKind::Projection => {
                ui.label("Orthographic projection");
            }
            ViewKind::Section { offset_mm } => {
                ui.label("Section offset (model mm)");
                ui.add(egui::DragValue::new(offset_mm).speed(1.))
                    .on_hover_text(
                        "Cut along the view normal and retain the material behind the plane",
                    );
            }
            ViewKind::Detail {
                center_mm,
                radius_mm,
            } => {
                ui.label("Detail center X / Y and radius (model mm)");
                ui.horizontal_wrapped(|ui| {
                    for v in center_mm {
                        ui.add(egui::DragValue::new(v).speed(1.))
                            .on_hover_text("Detail center in the projection plane");
                    }
                    ui.add(egui::DragValue::new(radius_mm).range(0.001..=1e7))
                        .on_hover_text("Circular detail radius before view scaling");
                });
            }
        };
        let remove = crate::icons::compact_button(ui, Icon::Trash, "Remove view").clicked();
        let fit=ui.button("Fit view to page").on_hover_text("Use current native vector bounds to center this view and choose a fitting independent scale").clicked();
        if remove {
            page.views.retain(|v| v.id != id);
            self.selected_view = page.views.first().map(|v| v.id);
        }
        if fit && let Err(error) = self.fit_view(id, session) {
            self.error = Some(error);
        }
        if let Some(error) = &self.feed.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.feed.pending() {
            ui.label("Generating native drawing…");
        }
        if self.preview_matches(session)
            && let Some(result) = &self.feed.result
            && let Some(evaluation) = result.views.get(&id)
        {
            match evaluation {
                ViewEvaluation::Ready { vectors, warnings } => {
                    ui.small(format!("{} native vectors", vectors.len()));
                    for warning in warnings {
                        ui.colored_label(egui::Color32::YELLOW, warning);
                    }
                }
                ViewEvaluation::Unresolved { message } => {
                    ui.colored_label(egui::Color32::LIGHT_RED, message);
                }
            }
        }
        self.page_actions(ui, session);
    }
    pub(super) fn page_actions(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        if let Err(error) = self.validate_draft() {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.dirty() && self.revision != Some(session.revision()) {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "The source changed. Discard this stale page draft before editing again.",
            );
        }
        if !self.changed_views_ready(session) {
            ui.small("A changed view needs a successful current native preview before Apply.");
        }
        ui.separator();
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.enabled(DrawingTool::Apply, session),
                    egui::Button::new("Apply page"),
                )
                .on_hover_text("Commit the whole page draft as one undoable source edit")
                .clicked()
            {
                action = Some(DrawingTool::Apply);
            }
            if ui
                .add_enabled(self.dirty(), egui::Button::new("Discard edits"))
                .on_hover_text("Restore the saved page source and cancel pending edits")
                .clicked()
            {
                action = Some(DrawingTool::Discard);
            }
        });
        if let Some(tool) = action
            && let Err(error) = self.invoke(session, tool, Channel::Ui)
        {
            self.error = Some(error);
        }
    }
}
