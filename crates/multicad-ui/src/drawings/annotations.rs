//! Page annotation editing. Every change stays in the page draft until Apply,
//! which commits the whole page as one undoable edit. Annotations never edit
//! model parameters or features.
use super::*;
use crate::controls;
use multicad_core::{
    drawings::{
        Annotation, AnnotationEvaluation, AnnotationKind, DimensionMeasure, DraftGeometry,
        LinearDirection, SheetTransform, WeldSide, WeldSymbol,
    },
    inspection::{self, GeometryReference},
};

/// How a model dimension is made from a pinned measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinDimension {
    Horizontal,
    Vertical,
    Aligned,
    RadiusOfA,
    DiameterOfA,
    Angle,
}

fn fresh(reference: &GeometryReference) -> GeometryReference {
    let mut copy = reference.clone();
    copy.intent.id = EntityId::new();
    copy
}

impl DrawingEditor {
    pub fn selected_annotation(&self) -> Option<EntityId> {
        self.selected_annotation
    }
    fn page_center(&self) -> Result<[f64; 2], String> {
        let page = self.draft.as_ref().ok_or("Add a page first")?;
        let [w, h] = page.settings.dimensions();
        Ok([w * 0.35, h * 0.35])
    }
    pub(super) fn add_annotation(
        &mut self,
        session: &Session,
        tool: DrawingTool,
    ) -> Result<(), String> {
        let [x, y] = self.page_center()?;
        let kind = match tool {
            DrawingTool::Dimension => {
                let pin = inspection::pins(session.document())
                    .next()
                    .ok_or("Pin a measurement with Measure first; dimensions use its persistent references")?;
                return self.add_pin_dimension(session, pin, PinDimension::Aligned);
            }
            DrawingTool::Leader => AnnotationKind::Leader {
                points_mm: vec![[x, y], [x + 15., y - 10.]],
                text: "Note".into(),
            },
            DrawingTool::Weld => AnnotationKind::Weld {
                points_mm: vec![[x, y], [x + 12., y - 12.]],
                symbol: WeldSymbol::Fillet,
                side: WeldSide::Arrow,
                size: "a3".into(),
                length: String::new(),
                all_around: false,
                field: false,
                tail: String::new(),
            },
            DrawingTool::Table => AnnotationKind::Table {
                position_mm: [x, y],
                title: "Schedule".into(),
                columns: vec!["Item".into(), "Qty".into()],
                rows: vec![vec![String::new(), String::new()]],
                widths_mm: vec![30., 12.],
                row_height_mm: 5.,
                schedule: None,
            },
            DrawingTool::DraftLine => AnnotationKind::Draft {
                geometry: DraftGeometry::Line {
                    a: [x, y],
                    b: [x + 30., y],
                },
                line_width_mm: 0.35,
            },
            DrawingTool::DraftCircle => AnnotationKind::Draft {
                geometry: DraftGeometry::Circle {
                    center: [x, y],
                    radius_mm: 10.,
                },
                line_width_mm: 0.35,
            },
            DrawingTool::DraftArc => AnnotationKind::Draft {
                geometry: DraftGeometry::Arc {
                    center: [x, y],
                    radius_mm: 10.,
                    start_deg: 0.,
                    sweep_deg: 90.,
                },
                line_width_mm: 0.35,
            },
            DrawingTool::Hatch => AnnotationKind::Draft {
                geometry: DraftGeometry::Hatch {
                    boundary: vec![[x, y], [x + 20., y], [x + 20., y + 15.], [x, y + 15.]],
                    angle_deg: 45.,
                    spacing_mm: 2.,
                },
                line_width_mm: 0.18,
            },
            DrawingTool::DraftDimension => AnnotationKind::DraftDimension {
                a: [x, y],
                b: [x + 30., y],
                offset_mm: 6.,
                precision: 1,
            },
            _ => return Err("Not an annotation tool".into()),
        };
        self.push_annotation(kind)
    }
    fn push_annotation(&mut self, kind: AnnotationKind) -> Result<(), String> {
        let page = self.draft.as_mut().ok_or("Add a page first")?;
        let id = EntityId::new();
        page.annotations.push(Annotation { id, kind });
        self.selected_annotation = Some(id);
        self.selected_view = None;
        Ok(())
    }
    /// A model dimension from a pinned measurement's persistent references,
    /// in the selected (or first) view.
    pub fn add_pin_dimension(
        &mut self,
        session: &Session,
        pin: EntityId,
        how: PinDimension,
    ) -> Result<(), String> {
        let pin = inspection::read(session.document(), pin).map_err(|e| e.to_string())?;
        let page = self.draft.as_ref().ok_or("Add a page first")?;
        let view = self
            .selected_view
            .filter(|id| page.views.iter().any(|v| v.id == *id))
            .or_else(|| page.views.first().map(|v| v.id))
            .ok_or("Add a model view first")?;
        let (a, b) = (&pin.query.a, &pin.query.b);
        let measure = match how {
            PinDimension::Horizontal | PinDimension::Vertical | PinDimension::Aligned => {
                DimensionMeasure::Linear {
                    a: fresh(a),
                    b: fresh(b),
                    direction: match how {
                        PinDimension::Horizontal => LinearDirection::Horizontal,
                        PinDimension::Vertical => LinearDirection::Vertical,
                        _ => LinearDirection::Aligned,
                    },
                }
            }
            PinDimension::RadiusOfA => DimensionMeasure::Radius { edge: fresh(a) },
            PinDimension::DiameterOfA => DimensionMeasure::Diameter { edge: fresh(a) },
            PinDimension::Angle => DimensionMeasure::Angle {
                a: fresh(a),
                b: fresh(b),
            },
        };
        self.push_annotation(AnnotationKind::Dimension {
            view,
            measure: Box::new(measure),
            offset_mm: 10.,
            precision: 1,
            reference: false,
        })
    }
    pub fn transform_selected(&mut self, transform: SheetTransform) -> Result<(), String> {
        let id = self.selected_annotation.ok_or("Select an annotation")?;
        let page = self.draft.as_mut().ok_or("Add a page first")?;
        let annotation = page
            .annotations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or("The annotation is missing")?;
        multicad_core::drawings::transform(&mut annotation.kind, transform)
    }
    /// Copy the selected annotation with a new identity, offset on the sheet.
    pub fn copy_selected(&mut self, offset_mm: [f64; 2]) -> Result<(), String> {
        let id = self.selected_annotation.ok_or("Select an annotation")?;
        let page = self.draft.as_ref().ok_or("Add a page first")?;
        let mut kind = page
            .annotations
            .iter()
            .find(|a| a.id == id)
            .ok_or("The annotation is missing")?
            .kind
            .clone();
        if let AnnotationKind::Dimension { offset_mm: o, .. } = &mut kind {
            *o += offset_mm[1];
        } else {
            multicad_core::drawings::transform(&mut kind, SheetTransform::translation(offset_mm))?;
        }
        self.push_annotation(kind)
    }
    pub fn remove_selected(&mut self) -> Result<(), String> {
        let id = self
            .selected_annotation
            .take()
            .ok_or("Select an annotation")?;
        let page = self.draft.as_mut().ok_or("Add a page first")?;
        page.annotations.retain(|a| a.id != id);
        Ok(())
    }
    pub(super) fn annotations_inspector(&mut self, ui: &mut egui::Ui, session: &Session) {
        let Some(page) = self.draft.as_ref() else {
            return;
        };
        if page.annotations.is_empty() {
            return;
        }
        ui.separator();
        ui.strong("Annotations");
        let names: Vec<_> = page
            .annotations
            .iter()
            .map(|a| {
                let name = match &a.kind {
                    AnnotationKind::Dimension { .. } => "Dimension",
                    AnnotationKind::Note { .. } => "Note",
                    AnnotationKind::Leader { .. } => "Leader",
                    AnnotationKind::Weld { .. } => "Weld symbol",
                    AnnotationKind::Table { .. } => "Table",
                    AnnotationKind::Draft { .. } => "Sheet geometry",
                    AnnotationKind::DraftDimension { .. } => "Sheet dimension",
                };
                (a.id, name)
            })
            .collect();
        ui.horizontal_wrapped(|ui| {
            for (index, (id, name)) in names.iter().enumerate() {
                if ui
                    .selectable_label(
                        self.selected_annotation == Some(*id),
                        format!("{} {}", name, index + 1),
                    )
                    .on_hover_text("Edit this annotation")
                    .clicked()
                {
                    self.selected_annotation = Some(*id);
                }
            }
        });
        let Some(id) = self.selected_annotation else {
            return;
        };
        let evaluation = self
            .preview_matches(session)
            .then(|| {
                self.feed
                    .result
                    .as_ref()
                    .and_then(|r| r.annotations.get(&id).cloned())
            })
            .flatten();
        let views: Vec<_> = page.views.iter().map(|v| (v.id, v.name.clone())).collect();
        let Some(page) = self.draft.as_mut() else {
            return;
        };
        let Some(annotation) = page.annotations.iter_mut().find(|a| a.id == id) else {
            self.selected_annotation = None;
            return;
        };
        let point = |ui: &mut egui::Ui, label: &str, p: &mut [f64; 2]| {
            ui.horizontal_wrapped(|ui| {
                ui.label(label);
                for v in p {
                    ui.add(egui::DragValue::new(v).speed(0.5).suffix(" mm"))
                        .on_hover_text("Paper position from the page's top left");
                }
            });
        };
        match &mut annotation.kind {
            AnnotationKind::Dimension {
                view,
                measure,
                offset_mm,
                precision,
                reference,
            } => {
                ui.label(match measure.as_ref() {
                    DimensionMeasure::Linear { direction, .. } => match direction {
                        LinearDirection::Horizontal => "Horizontal model distance",
                        LinearDirection::Vertical => "Vertical model distance",
                        LinearDirection::Aligned => "Aligned model distance",
                    },
                    DimensionMeasure::Radius { .. } => "Model radius",
                    DimensionMeasure::Diameter { .. } => "Model diameter",
                    DimensionMeasure::Angle { .. } => "Model angle",
                });
                controls::inspector_combo(ui, "dimension-view")
                    .selected_text(
                        views
                            .iter()
                            .find(|(v, _)| v == view)
                            .map_or("Missing view", |(_, n)| n.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for (v, name) in &views {
                            ui.selectable_value(view, *v, name);
                        }
                    });
                ui.add(
                    egui::DragValue::new(offset_mm)
                        .speed(0.5)
                        .range(-500.0..=500.)
                        .prefix("Offset ")
                        .suffix(" mm"),
                )
                .on_hover_text("Paper distance of the dimension line from the geometry");
                ui.add(
                    egui::DragValue::new(precision)
                        .range(0..=6)
                        .prefix("Decimals "),
                );
                ui.checkbox(reference, "Reference dimension (parentheses)")
                    .on_hover_text(
                        "Non-driving reference value; drawing dimensions never drive the model",
                    );
                match evaluation {
                    Some(AnnotationEvaluation::Ready { graphic }) => {
                        ui.small(format!(
                            "Current model value: {} {}",
                            graphic.text, graphic.unit
                        ));
                    }
                    Some(AnnotationEvaluation::Detached { message }) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("Detached: {message}"));
                    }
                    None => {
                        ui.small("Waiting for the native drawing preview…");
                    }
                }
            }
            AnnotationKind::Note { position_mm, text } => {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                point(ui, "Position", position_mm);
            }
            AnnotationKind::Leader { points_mm, text } => {
                ui.text_edit_singleline(text);
                for (i, p) in points_mm.iter_mut().enumerate() {
                    point(ui, if i == 0 { "Arrow" } else { "Point" }, p);
                }
            }
            AnnotationKind::Weld {
                points_mm,
                symbol,
                side,
                size,
                length,
                all_around,
                field,
                tail,
            } => {
                controls::inspector_combo(ui, "weld-symbol")
                    .selected_text(format!("{symbol:?}"))
                    .show_ui(ui, |ui| {
                        for s in [
                            WeldSymbol::Fillet,
                            WeldSymbol::SquareGroove,
                            WeldSymbol::VGroove,
                            WeldSymbol::BevelGroove,
                            WeldSymbol::Plug,
                            WeldSymbol::Spot,
                            WeldSymbol::Seam,
                        ] {
                            ui.selectable_value(symbol, s, format!("{s:?}"));
                        }
                    });
                controls::inspector_combo(ui, "weld-side")
                    .selected_text(format!("{side:?} side"))
                    .show_ui(ui, |ui| {
                        for s in [WeldSide::Arrow, WeldSide::Other, WeldSide::Both] {
                            ui.selectable_value(side, s, format!("{s:?} side"));
                        }
                    });
                for (label, value) in [("Size", size), ("Length", length), ("Tail", tail)] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.text_edit_singleline(value);
                    });
                }
                ui.checkbox(all_around, "Weld all around");
                ui.checkbox(field, "Field weld");
                for (i, p) in points_mm.iter_mut().enumerate() {
                    point(ui, if i == 0 { "Arrow" } else { "Point" }, p);
                }
            }
            AnnotationKind::Table {
                position_mm,
                title,
                columns,
                rows,
                widths_mm,
                row_height_mm,
                schedule,
            } => {
                ui.text_edit_singleline(title).on_hover_text("Table title");
                point(ui, "Top left", position_mm);
                ui.add(
                    egui::DragValue::new(row_height_mm)
                        .range(2.0..=50.)
                        .prefix("Row height ")
                        .suffix(" mm"),
                );
                if let Some(spec) = schedule {
                    ui.small(format!(
                        "{} rows × {} columns from the {} schedule. Refresh replaces the cells; edit the model, not the table.",
                        rows.len(),
                        columns.len(),
                        spec.label()
                    ));
                } else {
                    egui::Grid::new(("table-cells", id)).show(ui, |ui| {
                        for (c, w) in columns.iter_mut().zip(widths_mm.iter_mut()) {
                            ui.vertical(|ui| {
                                ui.add(egui::TextEdit::singleline(c).desired_width(60.));
                                ui.add(egui::DragValue::new(w).range(2.0..=500.).suffix(" mm"));
                            });
                        }
                        ui.end_row();
                        for row in rows.iter_mut() {
                            for cell in row.iter_mut() {
                                ui.add(egui::TextEdit::singleline(cell).desired_width(60.));
                            }
                            ui.end_row();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Add row").clicked() && rows.len() < 256 {
                            rows.push(vec![String::new(); columns.len()]);
                        }
                        if ui.button("Remove row").clicked() {
                            rows.pop();
                        }
                        if ui.button("Add column").clicked() && columns.len() < 32 {
                            columns.push(String::new());
                            widths_mm.push(20.);
                            rows.iter_mut().for_each(|r| r.push(String::new()));
                        }
                    });
                }
            }
            AnnotationKind::Draft {
                geometry,
                line_width_mm,
            } => {
                ui.add(
                    egui::DragValue::new(line_width_mm)
                        .range(0.05..=2.)
                        .speed(0.01)
                        .prefix("Line width ")
                        .suffix(" mm"),
                );
                match geometry {
                    DraftGeometry::Line { a, b } => {
                        point(ui, "Start", a);
                        point(ui, "End", b);
                    }
                    DraftGeometry::Circle { center, radius_mm } => {
                        point(ui, "Center", center);
                        ui.add(
                            egui::DragValue::new(radius_mm)
                                .range(0.01..=1000.)
                                .prefix("Radius ")
                                .suffix(" mm"),
                        );
                    }
                    DraftGeometry::Arc {
                        center,
                        radius_mm,
                        start_deg,
                        sweep_deg,
                    } => {
                        point(ui, "Center", center);
                        ui.add(
                            egui::DragValue::new(radius_mm)
                                .range(0.01..=1000.)
                                .prefix("Radius ")
                                .suffix(" mm"),
                        );
                        ui.add(egui::DragValue::new(start_deg).prefix("Start ").suffix("°"));
                        ui.add(
                            egui::DragValue::new(sweep_deg)
                                .range(-360.0..=360.)
                                .prefix("Sweep ")
                                .suffix("°"),
                        );
                    }
                    DraftGeometry::Hatch {
                        boundary,
                        angle_deg,
                        spacing_mm,
                    } => {
                        ui.add(egui::DragValue::new(angle_deg).prefix("Angle ").suffix("°"));
                        ui.add(
                            egui::DragValue::new(spacing_mm)
                                .range(0.2..=50.)
                                .prefix("Spacing ")
                                .suffix(" mm"),
                        );
                        for p in boundary.iter_mut() {
                            point(ui, "Corner", p);
                        }
                    }
                }
            }
            AnnotationKind::DraftDimension {
                a,
                b,
                offset_mm,
                precision,
            } => {
                point(ui, "From", a);
                point(ui, "To", b);
                ui.add(
                    egui::DragValue::new(offset_mm)
                        .speed(0.5)
                        .prefix("Offset ")
                        .suffix(" mm"),
                );
                ui.add(
                    egui::DragValue::new(precision)
                        .range(0..=6)
                        .prefix("Decimals "),
                );
                ui.small("Reads paper length divided by the page scale.");
            }
        }
        let movable = !matches!(annotation.kind, AnnotationKind::Dimension { .. });
        let key = ui.id().with(("annotation-move", id));
        let mut step: [f64; 3] = ui.data(|d| d.get_temp(key)).unwrap_or([5., 0., 15.]);
        let mut action = None;
        if movable {
            ui.horizontal_wrapped(|ui| {
                ui.label("Move / rotate");
                ui.add(egui::DragValue::new(&mut step[0]).suffix(" mm X"));
                ui.add(egui::DragValue::new(&mut step[1]).suffix(" mm Y"));
                ui.add(egui::DragValue::new(&mut step[2]).suffix("°"));
            });
        }
        ui.horizontal_wrapped(|ui| {
            if movable
                && ui
                    .button("Move")
                    .on_hover_text("Translate by X / Y on the sheet")
                    .clicked()
            {
                action = Some(0);
            }
            if movable
                && ui
                    .button("Rotate")
                    .on_hover_text("Rotate about the first point; lengths are preserved")
                    .clicked()
            {
                action = Some(1);
            }
            if ui
                .button("Copy")
                .on_hover_text("Duplicate with a new identity, offset on the sheet")
                .clicked()
            {
                action = Some(2);
            }
            if crate::icons::compact_button(ui, Icon::Trash, "Remove annotation").clicked() {
                action = Some(3);
            }
        });
        ui.data_mut(|d| d.insert_temp(key, step));
        let about = annotation
            .anchor_points()
            .first()
            .copied()
            .unwrap_or_default();
        let result = match action {
            Some(0) => self.transform_selected(SheetTransform::translation([step[0], step[1]])),
            Some(1) => self.transform_selected(SheetTransform {
                translate_mm: [0.; 2],
                rotate_deg: step[2],
                about_mm: about,
            }),
            Some(2) => self.copy_selected([step[0].max(1.), step[1]]),
            Some(3) => self.remove_selected(),
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
    pub(super) fn pin_dimension_controls(&mut self, ui: &mut egui::Ui, session: &Session) {
        let pins: Vec<_> = inspection::pins(session.document()).collect();
        if pins.is_empty() || self.draft.as_ref().is_none_or(|p| p.views.is_empty()) {
            return;
        }
        ui.separator();
        ui.small("Add a model dimension from a pinned measurement");
        for pin in &pins {
            let name = session
                .document()
                .get(*pin)
                .map_or("Measurement".to_string(), |e| e.name.clone());
            ui.horizontal_wrapped(|ui| {
                ui.label(&name);
                for (label, how) in [
                    ("Horizontal", PinDimension::Horizontal),
                    ("Vertical", PinDimension::Vertical),
                    ("Aligned", PinDimension::Aligned),
                    ("R of A", PinDimension::RadiusOfA),
                    ("Ø of A", PinDimension::DiameterOfA),
                    ("Angle", PinDimension::Angle),
                ] {
                    if ui.small_button(label).clicked()
                        && let Err(error) = self.add_pin_dimension(session, *pin, how)
                    {
                        self.error = Some(error);
                    }
                }
            });
        }
    }
}
