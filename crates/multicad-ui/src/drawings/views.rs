use super::*;
use multicad_core::{
    document::{Document, EntityData},
    drawings::{DrawingView, SectionRange, SectionWindow, ViewEvaluation, ViewKind, ViewSource},
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
    pub(super) fn candidate_document(&self, session: &Session) -> Result<Document, String> {
        let (Some(id), Some(draft)) = (self.book, self.draft.as_ref()) else {
            return Ok(session.document().clone());
        };
        // A saved page already belongs to this validated document revision.
        // Keep its shared dependency order and content hash for the worker.
        if self.revision == Some(session.revision()) && !self.dirty() {
            return Ok(session.document().clone());
        }
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
            if self.validate_draft().is_err() {
                self.feed.clear();
                self.preview_input = None;
                return;
            }
            self.preview_input = Some((
                session.document().id(),
                session.revision(),
                self.book,
                self.draft.clone(),
            ));
            match self.candidate_document(session).and_then(|document| {
                self.feed.schedule_page(
                    document,
                    session.revision(),
                    ctx.input(|i| i.time),
                    self.draft.as_ref().expect("page with views").id,
                )
            }) {
                Ok(()) => {}
                Err(error) => {
                    self.feed.clear();
                    self.feed.error = Some(error);
                }
            }
        }
        if self.feed.tick(ctx.input(|i| i.time)) {
            ctx.request_repaint();
        }
        if self.feed.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
    pub(super) fn changed_views_ready(&self, session: &Session) -> bool {
        if self
            .draft
            .as_ref()
            .is_some_and(|page| page.views.is_empty())
        {
            return true;
        }
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
            guide: None,
            equipment_categories: None,
        });
        self.selected_view = Some(id);
        Ok(())
    }
    /// Start picking a section line in `parent`, a projection on this page.
    pub(super) fn begin_section_pick(&mut self, parent: EntityId) -> Result<(), String> {
        let page = self.draft.as_ref().ok_or("Add a page first")?;
        match page.views.iter().find(|v| v.id == parent).map(|v| v.kind) {
            Some(ViewKind::Projection) => {
                self.section_pick = Some((parent, None));
                Ok(())
            }
            Some(_) => Err("Pick section lines in a projection view".into()),
            None => Err("The parent view is missing".into()),
        }
    }
    pub(super) fn cancel_section_pick(&mut self) {
        self.section_pick = None;
    }
    /// Record a picked paper point. The second point adds a section view whose
    /// cut runs through the picked line, limited to that segment.
    pub(super) fn pick_section_point(&mut self, paper_mm: [f64; 2]) -> Result<(), String> {
        let Some((parent_id, first)) = self.section_pick else {
            return Ok(());
        };
        let Some(first) = first else {
            self.section_pick = Some((parent_id, Some(paper_mm)));
            return Ok(());
        };
        self.section_pick = None;
        let page = self.draft.as_mut().ok_or("Add a page first")?;
        let parent = page
            .views
            .iter()
            .find(|v| v.id == parent_id)
            .ok_or("The parent view is missing")?
            .clone();
        let (frame, window) = drawings::section_from_line(
            &parent,
            parent.model_point(first),
            parent.model_point(paper_mm),
        )
        .ok_or("Pick two distinct points for the section line")?;
        let id = EntityId::new();
        let letter = page
            .views
            .iter()
            .filter(|v| matches!(v.kind, ViewKind::Section { .. }))
            .count();
        let letter = char::from(b'A' + (letter % 26) as u8);
        let [w, _] = page.settings.dimensions();
        page.views.push(DrawingView {
            id,
            name: format!("Section {letter}-{letter}"),
            frame,
            kind: ViewKind::Section {
                offset_mm: 0.,
                range: SectionRange::Unlimited,
                window,
            },
            position_mm: [
                (parent.position_mm[0] + w * 0.35).min(w * 0.85),
                parent.position_mm[1],
            ],
            ..parent
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
        let layout = drawings::layout_page(page, 1, 1).map_err(|e| e.to_string())?;
        let view = page
            .views
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or("View is missing")?;
        let fit = drawings::fit_view(view, &layout, min, max, self.fit_mode)?;
        let center = fit.center_mm;
        for i in 0..3 {
            view.frame.origin_mm[i] += view.frame.u[i] * center[0] + view.frame.v[i] * center[1];
        }
        if let ViewKind::Detail { center_mm, .. } = &mut view.kind {
            center_mm[0] -= center[0];
            center_mm[1] -= center[1];
        }
        view.scale = fit.scale;
        view.position_mm = fit.position_mm;
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
        let page_id = page.id;
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
        let picking = self.section_pick.is_some_and(|(parent, _)| parent == id);
        let mut pick = None;
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
        custom_direction(ui, view);
        ui.label("View origin X / Y / Z (model mm)");
        ui.push_id(("drawing-view-origin", page_id, id), |ui| {
            ui.horizontal_wrapped(|ui| {
                for (axis, x) in view.frame.origin_mm.iter_mut().enumerate() {
                    ui.push_id(axis, |ui| {
                        ui.add(egui::DragValue::new(x).speed(1.))
                            .on_hover_text("World-space origin of the projection frame");
                    });
                }
            });
        });
        ui.label("Paper position X / Y (mm)");
        ui.push_id(("drawing-view-paper-position", page_id, id), |ui| {
            ui.horizontal_wrapped(|ui| {
                for (axis, x) in view.position_mm.iter_mut().enumerate() {
                    ui.push_id(axis, |ui| {
                        ui.add(egui::DragValue::new(x).speed(1.))
                            .on_hover_text("View anchor measured from the page's top left");
                    });
                }
            });
        });
        ui.label("View scale (paper : model)");
        crate::controls::inspector_combo(ui, "view-standard-scale")
            .selected_text(format!("Scale {}", drawings::scale_text(view.scale)))
            .show_ui(ui, |ui| {
                for scale in drawings::standard_scales() {
                    if ui
                        .selectable_label(view.scale == scale, drawings::scale_text(scale))
                        .on_hover_text("Use this standard drawing scale; geometry stays exact")
                        .clicked()
                    {
                        view.scale = scale;
                        ui.close();
                    }
                }
            });
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
        if view.kind == ViewKind::Projection && view.frame.normal()[2] > 1. - 1e-8 {
            ui.collapsing("Equipment plan categories", |ui| {
                use multicad_core::equipment::Category;
                for category in Category::ALL {
                    let mut visible = view
                        .equipment_categories
                        .as_ref()
                        .is_none_or(|set| set.contains(&category));
                    if ui.checkbox(&mut visible, category.label()).changed() {
                        let set = view
                            .equipment_categories
                            .get_or_insert_with(|| Category::ALL.into_iter().collect());
                        if visible {
                            set.insert(category);
                        } else {
                            set.remove(&category);
                        }
                    }
                }
                ui.small("ToskCAD generic v1 symbols; equipment footprints retain model scale.");
            });
        }
        match &mut view.kind {
            ViewKind::Projection => {
                ui.label("Orthographic projection");
                pick = if picking {
                    ui.small("Click the start, then the end of the section line on the page.");
                    ui.button("Cancel section line")
                        .on_hover_text("Stop picking without adding a section")
                        .clicked()
                        .then_some(false)
                } else {
                    ui.button("Pick section line")
                        .on_hover_text("Click two points in this view to add a finite section through them; it looks toward the left of the picked direction")
                        .clicked()
                        .then_some(true)
                };
            }
            ViewKind::Section {
                offset_mm,
                range,
                window,
            } => section_controls(ui, offset_mm, range, window),
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
        let mut fit = false;
        ui.horizontal_wrapped(|ui| {
            fit = ui
                .button("Fit view to page")
                .on_hover_text("Center this view in the free paper beside the title block and choose its scale")
                .clicked();
            crate::controls::inspector_combo(ui, "view-fit-mode")
                .selected_text(self.fit_mode.label())
                .show_ui(ui, |ui| {
                    for mode in drawings::FitMode::ALL {
                        ui.selectable_value(&mut self.fit_mode, mode, mode.label())
                            .on_hover_text(match mode {
                                drawings::FitMode::Standard => "Largest standard scale that fits; small parts are enlarged (2:1 … 100:1)",
                                drawings::FitMode::Exact => "Tightest whole-number ratio that fits",
                                drawings::FitMode::KeepScale => "Retain the current scale, e.g. 1:1, and only center the view",
                            });
                    }
                });
        });
        ui.small(format!("Printed as \"{}\"", drawings::caption_text(view)));
        if remove {
            page.views.retain(|v| v.id != id);
            self.selected_view = page.views.first().map(|v| v.id);
        }
        if fit && let Err(error) = self.fit_view(id, session) {
            self.error = Some(error);
        }
        match pick {
            Some(true) => {
                if let Err(error) = self.begin_section_pick(id) {
                    self.error = Some(error);
                }
            }
            Some(false) => self.cancel_section_pick(),
            None => {}
        }
        if self.feed.pending() {
            ui.label("Generating native drawing…");
        }
        if self.preview_matches(session)
            && let Some(result) = &self.feed.result
            && let Some(evaluation) = result.views.get(&id)
        {
            match evaluation {
                ViewEvaluation::Ready {
                    vectors, warnings, ..
                } => {
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

/// An exterior elevation or projection along any direction: the camera looks
/// along `direction`, with world Z (or Y when looking vertically) kept up.
pub(super) fn frame_for_direction(direction: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
    let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !length.is_finite() || length < 1e-9 {
        return None;
    }
    let n = direction.map(|v| -v / length);
    let up = if n[2].abs() > 0.999 {
        [0., 1., 0.]
    } else {
        [0., 0., 1.]
    };
    let along = up[0] * n[0] + up[1] * n[1] + up[2] * n[2];
    let v = [
        up[0] - along * n[0],
        up[1] - along * n[1],
        up[2] - along * n[2],
    ];
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    let v = v.map(|x| x / norm);
    let u = [
        v[1] * n[2] - v[2] * n[1],
        v[2] * n[0] - v[0] * n[2],
        v[0] * n[1] - v[1] * n[0],
    ];
    Some((u, v))
}
fn custom_direction(ui: &mut egui::Ui, view: &mut DrawingView) {
    let id = ui.id().with(("view-look", view.id));
    let mut look = ui
        .data(|d| d.get_temp::<[f64; 3]>(id))
        .unwrap_or([0., 1., 0.]);
    ui.label("Custom look direction X / Y / Z");
    ui.horizontal_wrapped(|ui| {
        for v in &mut look {
            ui.add(egui::DragValue::new(v).speed(0.05).range(-1.0..=1.0))
                .on_hover_text("Direction the camera looks along, in world coordinates");
        }
        if ui
            .button("Apply")
            .on_hover_text("Orient this exterior view along the custom direction, world Z up")
            .clicked()
            && let Some((u, v)) = frame_for_direction(look)
        {
            view.frame.u = u;
            view.frame.v = v;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, look));
}
fn section_controls(
    ui: &mut egui::Ui,
    offset_mm: &mut f64,
    range: &mut SectionRange,
    window: &mut SectionWindow,
) {
    ui.label("Section offset (model mm)");
    ui.add(egui::DragValue::new(offset_mm).speed(1.))
        .on_hover_text("Cut plane position along the view normal from the view origin");
    ui.label("Shown behind the cut");
    let label = match range {
        SectionRange::Unlimited => "Everything behind",
        SectionRange::Depth { .. } => "Finite depth",
        SectionRange::SectionOnly => "Section only",
    };
    crate::controls::inspector_combo(ui, "section-range")
        .selected_text(label)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(
                    matches!(range, SectionRange::Unlimited),
                    "Everything behind",
                )
                .on_hover_text("Show all retained material behind the cut plane")
                .clicked()
            {
                *range = SectionRange::Unlimited;
            }
            if ui
                .selectable_label(matches!(range, SectionRange::Depth { .. }), "Finite depth")
                .on_hover_text(
                    "Show uncut geometry only within a depth behind the cut; clip at the far limit",
                )
                .clicked()
                && !matches!(range, SectionRange::Depth { .. })
            {
                *range = SectionRange::Depth { depth_mm: 100. };
            }
            if ui
                .selectable_label(matches!(range, SectionRange::SectionOnly), "Section only")
                .on_hover_text("Show only the exact plane intersection and its hatching")
                .clicked()
            {
                *range = SectionRange::SectionOnly;
            }
        });
    if let SectionRange::Depth { depth_mm } = range {
        ui.label("View depth behind the cut (model mm)");
        ui.add(egui::DragValue::new(depth_mm).speed(1.).range(0.001..=1e7))
            .on_hover_text("Far limit of the viewing interval, measured from the cut plane");
    }
    for (axis, limits, hint) in [
        (
            "U (across)",
            &mut window.u_mm,
            "Limit the section line along the view's right axis",
        ),
        (
            "V (height)",
            &mut window.v_mm,
            "Limit the section along the view's up axis",
        ),
    ] {
        let mut limited = limits.is_some();
        if ui
            .checkbox(&mut limited, format!("Limit section {axis}"))
            .on_hover_text(hint)
            .changed()
        {
            *limits = limited.then_some([-50., 50.]);
        }
        if let Some([lo, hi]) = limits {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(lo).speed(1.))
                    .on_hover_text("Window start from the view origin (model mm)");
                ui.label("…");
                ui.add(egui::DragValue::new(hi).speed(1.))
                    .on_hover_text("Window end from the view origin (model mm)");
            });
        }
    }
    ui.small(match range {
        SectionRange::Unlimited => "Cut faces are hatched; all material behind is drawn.".to_string(),
        SectionRange::Depth { depth_mm } => format!(
            "Cut faces are hatched; geometry within {depth_mm} mm behind is drawn and clipped at the far limit."
        ),
        SectionRange::SectionOnly => "Only the cut faces and their hatching are drawn.".into(),
    });
}

#[cfg(test)]
mod direction_tests {
    use super::frame_for_direction;
    #[test]
    fn custom_directions_give_right_handed_frames_with_world_up() {
        let near = |a: [f64; 3], b: [f64; 3]| {
            assert!(
                a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12),
                "{a:?} != {b:?}"
            )
        };
        // Looking along +Y matches the Front (−Y) preset.
        let (u, v) = frame_for_direction([0., 1., 0.]).unwrap();
        near(u, [1., 0., 0.]);
        near(v, [0., 0., 1.]);
        // Looking straight down keeps +Y up, like the Top preset.
        let (u, v) = frame_for_direction([0., 0., -2.]).unwrap();
        near(u, [1., 0., 0.]);
        near(v, [0., 1., 0.]);
        // An oblique elevation stays orthonormal and looks along the input.
        let (u, v) = frame_for_direction([1., 1., -0.5]).unwrap();
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let d = 1.5;
        near(n, [-1. / d, -1. / d, 0.5 / d]);
        assert!((u[0] * v[0] + u[1] * v[1] + u[2] * v[2]).abs() < 1e-12);
        assert!(frame_for_direction([0.; 3]).is_none());
    }
}
