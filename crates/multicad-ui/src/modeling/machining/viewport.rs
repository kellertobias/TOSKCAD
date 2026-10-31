//! Visual placement for the machining form: the receiving panel faces,
//! their local axes and the resolved cuts are drawn over the scene, and a
//! click on a visible panel face adds that panel and places the hole,
//! pocket, groove or shelf-pin row at the snapped face point. The Inspector
//! fields are the same values, so either can edit them.
use super::{Kind, MachiningPanel, Side, field};
use crate::{
    egui,
    modeling::{
        TerminationViewport,
        panel_face::{self, FaceHit},
    },
};
use multicad_core::{
    document::{Document, EntityId},
    model::{self, MachiningFace, PanelAxis, PanelEdgeName, RowEdge},
    parameters::{EvaluatedParameters, Unit},
};

/// Screen distance within which a click selects an existing hole.
const SELECT_PX: f32 = 8.;

impl MachiningPanel {
    pub fn picking(&self) -> bool {
        self.place
    }
    /// The panels the current values cut, with the face each one is cut on,
    /// or why the values are refused for it.
    pub(super) fn resolved(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
    ) -> Vec<(EntityId, Result<model::ResolvedMachining, String>)> {
        let request = self.request(document, true);
        self.targets
            .iter()
            .map(|&t| {
                let r = match &request {
                    Ok(c) => model::resolve_machining(document, parameters, &c.machining, t)
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.clone()),
                };
                (t, r)
            })
            .collect()
    }
    /// Paint the targets and cuts, track the hovered face and apply a click.
    /// Returns true when a click changed the form.
    pub fn viewport(
        &mut self,
        document: &Document,
        revision: u64,
        view: TerminationViewport<'_>,
    ) -> bool {
        let Ok(parameters) = multicad_core::parameters::evaluate(document) else {
            return false;
        };
        self.paint(document, &parameters, &view);
        if !self.place {
            self.hover = None;
            return false;
        }
        self.hover = panel_face::hover(document, revision, &view);
        let Some(hit) = self.hover else {
            return false;
        };
        let at = self.snapped(document, &hit);
        panel_face::outline(&view, &hit.frame, panel_face::HOVER);
        panel_face::axes(&view, &hit.frame);
        panel_face::offsets(&view, &hit.frame, at);
        let p = panel_face::on_screen(&view, &hit.frame, at);
        view.painter
            .circle_stroke(p, 4., egui::Stroke::new(2., panel_face::HOVER));
        let name = document.get(hit.part).map_or("", |e| e.name.as_str());
        panel_face::label(
            &view,
            p + egui::vec2(8., -8.),
            &format!(
                "{name} · {} · ({:.1}, {:.1})",
                panel_face::face_label(hit.face),
                at[0],
                at[1]
            ),
            panel_face::HOVER,
        );
        if !view.clicked {
            return false;
        }
        let pointer = view.pointer.unwrap_or(p);
        let changed = self.click(document, &parameters, hit, at, |frame, q| {
            panel_face::on_screen(&view, frame, q).distance(pointer)
        });
        if changed {
            self.stamp.changed();
            self.error = None;
        }
        changed
    }
    /// Face points snap to whole millimeters and the face edges and center
    /// lines.
    fn snapped(&self, document: &Document, hit: &FaceHit) -> [f64; 2] {
        let _ = document;
        panel_face::snap(hit.at, hit.frame.size, 1., 5.)
    }
    fn paint(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        view: &TerminationViewport<'_>,
    ) {
        for (k, (part, resolved)) in self.resolved(document, parameters).into_iter().enumerate() {
            let name = document.get(part).map_or(String::new(), |e| e.name.clone());
            match resolved {
                Ok(r) => {
                    panel_face::outline(view, &r.frame, panel_face::TARGET);
                    if k == 0 && self.hover.is_none() {
                        panel_face::axes(view, &r.frame);
                    }
                    panel_face::label(
                        view,
                        panel_face::on_screen(view, &r.frame, [0., r.frame.size[1]]),
                        &format!("{name} · {}", panel_face::face_label(r.face)),
                        panel_face::TARGET,
                    );
                    for (i, h) in r.holes.iter().enumerate() {
                        let color = if self.kind == Kind::Drill && self.selected_hole == Some(i) {
                            panel_face::HOVER
                        } else {
                            panel_face::CUT
                        };
                        panel_face::circle(view, &r.frame, h.at, h.diameter_mm, color);
                    }
                    for x in &r.recesses {
                        panel_face::polygon(view, &r.frame, &x.outline, panel_face::CUT);
                    }
                }
                Err(error) => {
                    let face = match self.side {
                        Side::Bottom => MachiningFace::Bottom,
                        Side::Edge => MachiningFace::Edge { edge: self.edge },
                        _ => MachiningFace::Top,
                    };
                    if let Some(frame) = panel_face::frame(document, parameters, part, face) {
                        panel_face::outline(view, &frame, panel_face::INVALID);
                        panel_face::label(
                            view,
                            panel_face::on_screen(view, &frame, [0., frame.size[1]]),
                            &format!("{name}: {error}"),
                            panel_face::INVALID,
                        );
                    }
                }
            }
        }
    }
    /// Apply a face click: add the panel as a target, take its face, then
    /// place the feature at `at` in that face's coordinates. `distance`
    /// measures the screen distance from the pointer to a face point.
    pub(super) fn click(
        &mut self,
        document: &Document,
        parameters: &EvaluatedParameters,
        hit: FaceHit,
        at: [f64; 2],
        distance: impl Fn(&model::MachiningFaceFrame, [f64; 2]) -> f32,
    ) -> bool {
        self.pick_note = None;
        let name = document
            .get(hit.part)
            .map_or("This panel".into(), |e| e.name.clone());
        if !self.targets.contains(&hit.part) {
            if self.existing.is_some() {
                self.pick_note = Some(format!(
                    "{name} is not a target of this operation; its target panels are fixed"
                ));
                return false;
            }
            self.targets.push(hit.part);
        }
        let drilling = matches!(self.kind, Kind::Drill | Kind::Hardware);
        match (self.side, hit.face) {
            (_, MachiningFace::Edge { edge }) if drilling => {
                self.side = Side::Edge;
                self.edge = edge;
            }
            (_, MachiningFace::Edge { .. }) => {
                self.pick_note = Some(format!(
                    "{} are cut into a top, bottom or inside face; click the face of {name}",
                    self.kind_noun()
                ));
                return true;
            }
            // Inside keeps choosing each panel's inner face; top and bottom
            // share the panel (u, v) coordinates, so the point is the same.
            (Side::Inside, _) => {}
            (_, face) => {
                self.side = if face == MachiningFace::Bottom {
                    Side::Bottom
                } else {
                    Side::Top
                };
            }
        }
        let size = hit.frame.size;
        match self.kind {
            Kind::Drill => {
                let mut holes = self.hole_points();
                let near = holes
                    .iter()
                    .position(|h| distance(&hit.frame, *h) <= SELECT_PX);
                match (near, self.selected_hole) {
                    (Some(i), _) => {
                        self.selected_hole = Some(i);
                        return true;
                    }
                    (None, Some(i)) if i < holes.len() => holes[i] = at,
                    _ => holes.push(at),
                }
                self.holes = holes
                    .iter()
                    .map(|[u, v]| format!("{}, {}", num(*u), num(*v)))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            Kind::Pocket => self.center = [field(at[0]), field(at[1])],
            Kind::Hardware => self.at = [field(at[0]), field(at[1])],
            Kind::Groove => {
                let width = self
                    .width
                    .resolve(document)
                    .ok()
                    .and_then(|d| d.value_in(Unit::Millimeter, Some(parameters)).ok())
                    .unwrap_or(0.);
                let across = if self.along == PanelAxis::U {
                    at[1]
                } else {
                    at[0]
                };
                self.offset = field((across - width / 2.).max(0.));
            }
            Kind::Rabbet => {
                let d = [at[0], size[0] - at[0], at[1], size[1] - at[1]];
                let i = (0..4).min_by(|a, b| d[*a].total_cmp(&d[*b])).unwrap_or(0);
                self.edge = [
                    PanelEdgeName::UStart,
                    PanelEdgeName::UEnd,
                    PanelEdgeName::VStart,
                    PanelEdgeName::VEnd,
                ][i];
            }
            Kind::ShelfPins => {
                let (c, across) = match self.pin_along {
                    PanelAxis::U => (at[1], size[1]),
                    PanelAxis::V => (at[0], size[0]),
                };
                if let Some((from, offset)) = self.pin_rows.get_mut(self.active_row) {
                    *offset = field(match from {
                        RowEdge::Start => c,
                        RowEdge::End => across - c,
                    });
                }
            }
        }
        true
    }
    fn kind_noun(&self) -> &'static str {
        match self.kind {
            Kind::Groove => "Grooves",
            Kind::Rabbet => "Rabbets",
            Kind::Pocket => "Pockets",
            Kind::ShelfPins => "Shelf-pin rows",
            Kind::Drill | Kind::Hardware => "Holes",
        }
    }
    /// The literal hole centers of the hole list; unreadable lines are kept
    /// out, as the Inspector already reports them.
    pub(super) fn hole_points(&self) -> Vec<[f64; 2]> {
        self.holes
            .lines()
            .filter_map(|line| {
                let v: Vec<f64> = line
                    .split(',')
                    .map(|p| p.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .ok()?;
                match v.as_slice() {
                    [u, w] => Some([*u, *w]),
                    _ => None,
                }
            })
            .collect()
    }
    /// Inspector controls of the System 32 shelf-pin rows.
    pub(super) fn show_shelf_pins(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        ui.small("Rows of blind shelf-pin holes on each target's face, every pitch along the row. Opposite sides targeted together get the same rows, so a shelf sits level on matching pins.");
        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(&mut self.pin_along, PanelAxis::V, "Rows along width (v)")
                .changed();
            changed |= ui
                .radio_value(&mut self.pin_along, PanelAxis::U, "Rows along length (u)")
                .changed();
        });
        let mut remove = None;
        for (i, (from, offset)) in self.pin_rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.active_row, i, format!("Row {}", i + 1))
                    .on_hover_text("A face click in the viewport moves this row");
                changed |= ui
                    .selectable_value(from, RowEdge::Start, "from start")
                    .changed();
                changed |= ui
                    .selectable_value(from, RowEdge::End, "from end")
                    .changed();
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
            changed |= offset.show(
                ui,
                "Row offset",
                "Distance from the chosen edge across the rows",
            );
        }
        if let Some(i) = remove.filter(|_| self.pin_rows.len() > 1) {
            self.pin_rows.remove(i);
            self.active_row = self.active_row.min(self.pin_rows.len() - 1);
            changed = true;
        }
        if self.pin_rows.len() < model::MAX_PIN_ROWS && ui.button("Add row").clicked() {
            self.pin_rows
                .push((RowEdge::Start, field(super::SYSTEM32_ROW_OFFSET_MM)));
            changed = true;
        }
        ui.horizontal(|ui| {
            ui.label("Pitch");
            if ui
                .button(format!("System 32 ({} mm)", model::SYSTEM32_PITCH_MM))
                .clicked()
            {
                self.pitch = field(model::SYSTEM32_PITCH_MM);
                changed = true;
            }
        });
        changed |= self
            .pitch
            .show(ui, "Pitch", "Distance between hole centers along a row");
        changed |= self.pin_start.show(
            ui,
            "Start",
            "First hole center from the start edge along the row",
        );
        changed |= self.pin_end.show(
            ui,
            "End limit",
            "No hole center closer than this to the end edge",
        );
        let mut fixed = self.pin_count.is_some();
        if ui.checkbox(&mut fixed, "Fixed hole count").changed() {
            self.pin_count = fixed.then_some(self.pin_count.unwrap_or(10));
            changed = true;
        }
        if let Some(count) = &mut self.pin_count {
            changed |= ui
                .add(egui::DragValue::new(count).range(1..=model::MAX_HOLES as u32))
                .changed();
        }
        changed |= self.diameter.show(ui, "Diameter", "Hole diameter");
        changed |= self.depth.show(ui, "Depth", "Blind hole depth");
        if let Ok(parameters) = multicad_core::parameters::evaluate(document) {
            for (part, r) in self.resolved(document, &parameters) {
                let name = document.get(part).map_or(String::new(), |e| e.name.clone());
                if let Ok(r) = r {
                    ui.small(format!(
                        "{name}: {} holes on the {}",
                        r.holes.len(),
                        panel_face::face_label(r.face)
                    ));
                }
            }
            let carcasses: std::collections::BTreeSet<_> = self
                .targets
                .iter()
                .filter_map(|t| model::panel_feature(document, *t))
                .filter_map(|f| model::read_panel(document, f).ok())
                .filter_map(|p| match p.source {
                    model::PanelSource::Carcass { carcass, .. } => Some(carcass),
                    _ => None,
                })
                .collect();
            if let Some(levels) = self.request(document, true).ok().and_then(|_| {
                carcasses
                    .first()
                    .and_then(|c| model::shelf_support_offsets(document, &parameters, *c).ok())
            }) && !levels.is_empty()
            {
                ui.small(format!(
                    "Applied shelf support levels (interior bottom to shelf underside): {}",
                    levels
                        .iter()
                        .take(12)
                        .map(|l| format!("{l:.1}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        changed
    }
}

fn num(v: f64) -> String {
    let r = (v * 1000.).round() / 1000.;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}
