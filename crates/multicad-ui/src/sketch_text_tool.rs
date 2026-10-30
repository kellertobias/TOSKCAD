//! Sketch → Common → Text (CAD-142, CAD-149). A click places the Inspector's
//! text (font, size, anchor, line alignment, rotation) with its anchor at the
//! clicked point, through the shared `multicad_solver::prepare_text`
//! transaction (one Undo step). Clicking a letter of placed text, with no tool
//! or with the Text tool, selects that text and opens it here for editing;
//! Apply text is one further transaction. Refusals change nothing.
use super::*;
use multicad_core::constraint_sketch::text::{
    SketchText, TextAlignment, TextAnchor, TextFont, TextOperation, TextStyle, text_at,
};

/// Inspector state of the Text tool; never canonical source.
pub(super) struct TextTool {
    pub draft: TextStyle,
    /// Existing text element being edited, if any.
    pub editing: Option<EntityId>,
}
impl Default for TextTool {
    fn default() -> Self {
        let mut draft = TextStyle::new("Text", 5., [0., 0.]);
        // The anchor defaults to the block center, as the user asked for a
        // text size and its center position.
        draft.anchor = TextAnchor::Center;
        draft.alignment = TextAlignment::Center;
        Self {
            draft,
            editing: None,
        }
    }
}

const SELECTED: Color32 = Color32::from_rgb(255, 176, 64);

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
    /// Open a placed text in the Text Inspector for editing.
    fn open_text(&mut self, text: &SketchText) {
        self.text_tool.editing = Some(text.id);
        self.text_tool.draft = text.style.clone();
        self.message = format!(
            "Text \"{}\" selected · edit it in the Inspector · Apply text commits",
            text.style.text.replace('\n', " ⏎ ")
        );
    }
    /// Click selection: a click on a glyph outline (`curve` is one of a
    /// text's derived contours) or inside a letter selects that text, switches
    /// to the Text tool and opens it for editing. Returns whether it did.
    pub(super) fn select_text_at(
        &mut self,
        session: &Session,
        curve: Option<EntityId>,
        point: Option<[f64; 2]>,
    ) -> bool {
        let texts = self.sketch_texts(session);
        let hit = curve
            .and_then(|curve| texts.iter().find(|t| t.owns_contour(curve)).map(|t| t.id))
            .or_else(|| point.and_then(|point| text_at(&texts, point)));
        let Some(text) = hit.and_then(|id| texts.into_iter().find(|t| t.id == id)) else {
            return false;
        };
        if let Err(error) = self.set_tool(DrawTool::Text) {
            self.message = error;
            return true;
        }
        self.open_text(&text);
        true
    }
    pub(super) fn text_tool_ui(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        let draft = &mut self.text_tool.draft;
        ui.label("Text");
        ui.add(
            egui::TextEdit::multiline(&mut draft.text)
                .desired_rows(2)
                .desired_width(180.),
        )
        .on_hover_text("Printable ASCII or ÄÖÜäöüßéèàç°±µ · Enter starts a new line");
        ui.horizontal(|ui| {
            ui.label("Font");
            egui::ComboBox::from_id_salt("sketch-text-font")
                .selected_text(draft.font.label())
                .show_ui(ui, |ui| {
                    for font in TextFont::ALL {
                        ui.selectable_value(&mut draft.font, font, font.label());
                    }
                });
        })
        .response
        .on_hover_text(
            "Bundled vector outline fonts: Cantarell (proportional, OFL) or Hack (monospaced)",
        );
        ui.horizontal(|ui| {
            ui.label("Size");
            ui.add(
                egui::DragValue::new(&mut draft.height_mm)
                    .range(0.1..=10_000.0)
                    .speed(0.1)
                    .suffix(" mm"),
            )
            .on_hover_text("Cap height: the height of capital letters such as H");
        });
        ui.horizontal(|ui| {
            ui.label("Anchor");
            egui::Grid::new("sketch-text-anchor")
                .spacing([2., 2.])
                .show(ui, |ui| {
                    for (index, anchor) in TextAnchor::ALL.into_iter().enumerate() {
                        ui.selectable_value(&mut draft.anchor, anchor, "◼")
                            .on_hover_text(anchor.label());
                        if index % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
            ui.small(draft.anchor.label());
        })
        .response
        .on_hover_text("The point of the text block that sits at the anchor position");
        ui.horizontal(|ui| {
            ui.label("Align");
            for (alignment, label) in [
                (TextAlignment::Left, "Left"),
                (TextAlignment::Center, "Center"),
                (TextAlignment::Right, "Right"),
            ] {
                ui.selectable_value(&mut draft.alignment, alignment, label);
            }
        })
        .response
        .on_hover_text("Alignment of each line inside the text block");
        ui.horizontal(|ui| {
            ui.label("Rotation");
            ui.add(
                egui::DragValue::new(&mut draft.rotation_deg)
                    .range(-360.0..=360.0)
                    .speed(1.)
                    .suffix("°"),
            )
            .on_hover_text("Counterclockwise about the anchor");
        });
        ui.small("Outlines are exact closed profiles. A click places the anchor.");
        if let Some(editing) = self.text_tool.editing {
            let draft = &mut self.text_tool.draft;
            ui.horizontal(|ui| {
                ui.label("Position");
                for value in &mut draft.position_mm {
                    ui.add(egui::DragValue::new(value).speed(0.1).suffix(" mm"));
                }
            })
            .response
            .on_hover_text("Anchor position in sketch coordinates");
            ui.small("Editing the selected text · click empty space to place another");
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
            ui.small("Placed texts · or click a letter in the view");
            for text in texts {
                ui.horizontal(|ui| {
                    let selected = self.text_tool.editing == Some(text.id);
                    if ui
                        .selectable_label(
                            selected,
                            format!(
                                "\"{}\" · {} mm",
                                text.style.text.replace('\n', " ⏎ "),
                                text.style.height_mm
                            ),
                        )
                        .on_hover_text("Edit this text in the fields above")
                        .clicked()
                    {
                        self.open_text(&text);
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
    /// Display-only outline and anchor marker of one text.
    fn paint_text(
        &self,
        painter: &egui::Painter,
        input: &draw_modes::ModePointer,
        text: &SketchText,
        color: Color32,
    ) {
        let Ok(contours) = text.contours() else {
            return;
        };
        for contour in contours {
            for pair in contour.samples(4).windows(2) {
                if let Some(segment) = self.camera.segment(
                    input.frame.point(pair[0]),
                    input.frame.point(pair[1]),
                    input.rect,
                ) {
                    painter.line_segment(segment, Stroke::new(1., color));
                }
            }
        }
        let anchor = self.camera.project(
            input.frame.point(text.anchor_mm()).map(|v| v as f32),
            input.rect,
        );
        painter.circle_stroke(Pos2::new(anchor[0], anchor[1]), 4., Stroke::new(1.5, color));
    }
    pub(super) fn text_pointer(
        &mut self,
        painter: &egui::Painter,
        session: &mut Session,
        input: draw_modes::ModePointer,
    ) {
        let texts = self.sketch_texts(session);
        if let Some(editing) = self
            .text_tool
            .editing
            .and_then(|id| texts.iter().find(|t| t.id == id))
        {
            self.paint_text(painter, &input, editing, SELECTED);
        }
        // A letter under the pointer is selected by a click, not overlaid.
        if let Some(hovered) = text_at(&texts, input.point) {
            if input.clicked
                && let Some(text) = texts.iter().find(|t| t.id == hovered)
            {
                self.open_text(text);
            }
            return;
        }
        let mut style = self.text_tool.draft.clone();
        style.position_mm = input.point;
        let ghost = SketchText {
            id: self.draw_preview_id,
            style: style.clone(),
        };
        self.paint_text(painter, &input, &ghost, Color32::LIGHT_YELLOW);
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
