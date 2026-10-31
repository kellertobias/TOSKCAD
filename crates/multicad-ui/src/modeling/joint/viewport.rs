//! Visual pairing for the joint form: click panel A, then panel B, on their
//! visible faces. For loose tenons the viewport draws both receiving
//! panels, the insertion direction, every rounded mortise and the tenons
//! resolved from the current values, or the reason they are refused.
use super::{JointPanel, Kind};
use crate::{
    egui,
    modeling::{TerminationViewport, panel_face},
};
use multicad_core::{
    document::{Document, EntityId},
    model::{self, MachiningFace},
    parameters::EvaluatedParameters,
};

impl JointPanel {
    pub fn picking(&self) -> bool {
        self.place
    }
    /// Paint the pairs, track the hovered panel and apply a click. Returns
    /// true when a click changed the pairs.
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
            return false;
        }
        let Some(hit) = panel_face::hover(document, revision, &view) else {
            return false;
        };
        let slot = self.next_slot();
        panel_face::outline(&view, &hit.frame, panel_face::HOVER);
        let name = document.get(hit.part).map_or("", |e| e.name.as_str());
        let p = panel_face::on_screen(&view, &hit.frame, hit.at);
        panel_face::label(
            &view,
            p + egui::vec2(8., -8.),
            &format!(
                "Panel {} · {name} · {}",
                if slot.1 == 0 { "A" } else { "B" },
                panel_face::face_label(hit.face)
            ),
            panel_face::HOVER,
        );
        if !view.clicked {
            return false;
        }
        let changed = self.pick(document, hit.part);
        if changed {
            self.stamp.changed();
            self.error = None;
        }
        changed
    }
    /// The (pair, side) a click fills next.
    fn next_slot(&self) -> (usize, usize) {
        for (i, pair) in self.pairs.iter().enumerate() {
            if let Some(k) = pair.iter().position(Option::is_none) {
                return (i, k);
            }
        }
        (self.pairs.len(), 0)
    }
    /// Put a clicked panel into the next open pair slot.
    pub(super) fn pick(&mut self, document: &Document, part: EntityId) -> bool {
        self.pick_note = None;
        let name = document.get(part).map_or(String::new(), |e| e.name.clone());
        if self.existing.is_some() {
            self.pick_note = Some(
                "An applied joint keeps its panel pairs; delete it and create another to change them"
                    .into(),
            );
            return false;
        }
        let (panels, refused) = panel_face::selected_panels(document, [part]);
        if panels.is_empty() {
            self.pick_note = refused.into_iter().next();
            return false;
        }
        let (i, k) = self.next_slot();
        if i == self.pairs.len() {
            self.pairs.push([None, None]);
        }
        if self.pairs[i][1 - k] == Some(part) {
            let other = if k == 0 { "B" } else { "A" };
            self.pick_note = Some(format!("{name} is already panel {other} of this pair"));
            return false;
        }
        self.pairs[i][k] = Some(part);
        true
    }
    fn paint(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        view: &TerminationViewport<'_>,
    ) {
        let spec = match (self.kind, self.tenon.spec(document)) {
            (Kind::LooseTenon, Ok(spec)) => Some(spec),
            _ => None,
        };
        for pair in &self.pairs {
            for (k, part) in pair.iter().enumerate() {
                let Some(part) = part else { continue };
                if let Some(frame) =
                    panel_face::frame(document, parameters, *part, MachiningFace::Top)
                {
                    let name = document
                        .get(*part)
                        .map_or(String::new(), |e| e.name.clone());
                    panel_face::outline(view, &frame, panel_face::TARGET);
                    panel_face::label(
                        view,
                        panel_face::on_screen(view, &frame, [0., frame.size[1]]),
                        &format!("{} · {name}", if k == 0 { "A" } else { "B" }),
                        panel_face::TARGET,
                    );
                }
            }
            let (Some(spec), [Some(a), Some(b)]) = (&spec, pair) else {
                continue;
            };
            let pair = model::PanelJointPair { a: *a, b: *b };
            match model::resolve_loose_tenon(document, parameters, spec, &pair) {
                Ok((_, layout)) => {
                    for m in &layout.face_mortises {
                        panel_face::polygon(view, &layout.face_frame, &m.outline, panel_face::CUT);
                    }
                    for m in &layout.edge_mortises {
                        panel_face::polygon(view, &layout.edge_frame, &m.outline, panel_face::CUT);
                    }
                    for t in &layout.tenons {
                        let at = |k: f64| {
                            let p = [0, 1, 2].map(|i| t.center[i] + t.axis[i] * k);
                            let s = view
                                .camera
                                .project([p[0] as f32, p[1] as f32, p[2] as f32], view.rect);
                            egui::pos2(s[0], s[1])
                        };
                        // The insertion direction: face panel into edge panel.
                        view.painter.arrow(
                            at(-t.into_face_mm),
                            at(t.into_edge_mm) - at(-t.into_face_mm),
                            egui::Stroke::new(2., panel_face::HOVER),
                        );
                    }
                }
                Err(error) => {
                    if let Some(frame) =
                        panel_face::frame(document, parameters, *a, MachiningFace::Top)
                    {
                        panel_face::outline(view, &frame, panel_face::INVALID);
                        panel_face::label(
                            view,
                            panel_face::on_screen(view, &frame, [0., 0.]),
                            &error.to_string(),
                            panel_face::INVALID,
                        );
                    }
                }
            }
        }
    }
}
