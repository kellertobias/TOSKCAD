//! Sketch → Common → Text (CAD-142). A click places the Inspector's string,
//! cap height, rotation and alignment at the baseline origin through the
//! shared `multicad_solver::prepare_text` transaction (one Undo step). The
//! placed text, or one chosen from the Inspector list, is edited there
//! (string, height, position, rotation, alignment) with Apply text as one
//! further transaction. Refusals change nothing.
use super::*;
use multicad_core::constraint_sketch::text::{SketchText, TextAlignment, TextOperation, TextStyle};

/// Inspector state of the Text tool; never canonical source.
pub(super) struct TextTool {
    pub draft: TextStyle,
    /// Existing text element being edited, if any.
    pub editing: Option<EntityId>,
}
impl Default for TextTool {
    fn default() -> Self {
        Self {
            draft: TextStyle::new("Text", 5., [0., 0.]),
            editing: None,
        }
    }
}

impl SketchEditor {
    fn sketch_texts(&self, session: &Session) -> Vec<SketchText> {
        self.selected
            .and_then(|id| {
                self.read_source(session.document(), id, session.revision())
                    .ok()
            })
            .map(|(source, _, _)| source.sketch.texts)
            .unwrap_or_default()
    }
    pub(super) fn text_tool_ui(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        let draft = &mut self.text_tool.draft;
        ui.horizontal(|ui| {
            ui.label("Text");
            ui.add(egui::TextEdit::singleline(&mut draft.text).desired_width(140.))
                .on_hover_text("One line of printable ASCII or ÄÖÜäöüßéèàç°±µ");
        });
        ui.horizontal(|ui| {
            ui.label("Height");
            ui.add(
                egui::DragValue::new(&mut draft.height_mm)
                    .range(0.1..=10_000.0)
                    .speed(0.1)
                    .suffix(" mm"),
            )
            .on_hover_text("Cap height: the height of capital letters such as H");
        });
        ui.horizontal(|ui| {
            ui.label("Rotation");
            ui.add(
                egui::DragValue::new(&mut draft.rotation_deg)
                    .range(-360.0..=360.0)
                    .speed(1.)
                    .suffix("°"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Align");
            for (alignment, label) in [
                (TextAlignment::Left, "Left"),
                (TextAlignment::Center, "Center"),
                (TextAlignment::Right, "Right"),
            ] {
                ui.selectable_value(&mut draft.alignment, alignment, label);
            }
        });
        ui.small("Font: Hack Regular (bundled). Outlines are exact closed profiles.");
        if let Some(editing) = self.text_tool.editing {
            let draft = &mut self.text_tool.draft;
            ui.horizontal(|ui| {
                ui.label("Position");
                for value in &mut draft.origin_mm {
                    ui.add(egui::DragValue::new(value).speed(0.1).suffix(" mm"));
                }
            })
            .response
            .on_hover_text("Baseline anchor in sketch coordinates");
            ui.small("Editing the selected text · a click places another text");
            ui.horizontal(|ui| {
                if ui.button("Apply text").clicked() {
                    let style = self.text_tool.draft.clone();
                    self.message = match self.apply_text(
                        session,
                        TextOperation::Edit {
                            text: editing,
                            style,
                        },
                    ) {
                        Ok(_) => "Text updated · Undo restores the previous text".into(),
                        Err(error) => error,
                    };
                }
                if ui.button("Remove text").clicked() {
                    self.message =
                        match self.apply_text(session, TextOperation::Remove { text: editing }) {
                            Ok(_) => {
                                self.text_tool.editing = None;
                                "Text removed · Undo restores it".into()
                            }
                            Err(error) => error,
                        };
                }
                if ui.button("New text").clicked() {
                    self.text_tool.editing = None;
                }
            });
        }
        let texts = self.sketch_texts(session);
        if !texts.is_empty() {
            ui.small("Placed texts");
            for text in texts {
                ui.horizontal(|ui| {
                    let selected = self.text_tool.editing == Some(text.id);
                    if ui
                        .selectable_label(
                            selected,
                            format!("\"{}\" · {} mm", text.style.text, text.style.height_mm),
                        )
                        .on_hover_text("Edit this text in the fields above")
                        .clicked()
                    {
                        self.text_tool.editing = Some(text.id);
                        self.text_tool.draft = text.style.clone();
                    }
                });
            }
        }
    }
    /// One solver-checked transaction; failures change nothing.
    pub fn apply_text(
        &mut self,
        session: &mut Session,
        operation: TextOperation,
    ) -> Result<EntityId, String> {
        if !self.editing || self.has_pending_edits() {
            return Err("Open a sketch and apply or discard pending edits first".into());
        }
        let id = self.selected.ok_or("Select a sketch")?;
        let prepared = multicad_solver::prepare_text(
            session,
            session.revision(),
            Actor::local(Channel::Ui),
            id,
            self.evaluated.as_ref(),
            &operation,
        )
        .map_err(|e| e.to_string())?;
        let preview = prepared.edit.preview.ok_or_else(|| {
            format!(
                "The sketch does not solve with this text ({:?}); nothing changed",
                prepared.edit.evaluation.diagnostics.state
            )
        })?;
        session.commit(preview).map_err(|e| e.to_string())?;
        self.revision = session.revision();
        Ok(prepared.text)
    }
    pub(super) fn text_pointer(
        &mut self,
        painter: &egui::Painter,
        session: &mut Session,
        input: draw_modes::ModePointer,
    ) {
        let mut style = self.text_tool.draft.clone();
        style.origin_mm = input.point;
        let ghost = SketchText {
            id: self.draw_preview_id,
            style: style.clone(),
        };
        // Display-only preview samples of the exact quadratic spans.
        if let Ok(contours) = ghost.contours() {
            for contour in contours {
                let mut previous: Option<[f64; 2]> = None;
                for span in contour.poles.windows(3).step_by(2) {
                    for step in 0..=4 {
                        let t = f64::from(step) / 4.;
                        let p = std::array::from_fn(|i| {
                            (1. - t) * (1. - t) * span[0][i]
                                + 2. * t * (1. - t) * span[1][i]
                                + t * t * span[2][i]
                        });
                        if let Some(a) = previous
                            && let Some(segment) = self.camera.segment(
                                input.frame.point(a),
                                input.frame.point(p),
                                input.rect,
                            )
                        {
                            painter.line_segment(segment, Stroke::new(1., Color32::LIGHT_YELLOW));
                        }
                        previous = Some(p);
                    }
                }
            }
        }
        if !input.clicked {
            return;
        }
        self.message = match self.apply_text(
            session,
            TextOperation::Add {
                style: style.clone(),
            },
        ) {
            Ok(id) => {
                self.text_tool.editing = Some(id);
                self.text_tool.draft = style;
                "Text placed · edit it in the Inspector · Undo removes it".into()
            }
            Err(error) => error,
        };
    }
}
