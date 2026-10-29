//! Drawing guide fields of the Levels & stairs panel: floor plans repeated on
//! levels, section lines and elevations, with page generation on Apply.
use super::dimension::DimensionField;
use crate::egui;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::{self, GuideDefinition, GuideKind, Scale},
    model::{self, length},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GuideView {
    Plan,
    Section,
    Elevation,
}
pub(super) struct GuideFields {
    pub view: GuideView,
    pub cut_height: DimensionField,
    pub view_depth: DimensionField,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub limit_depth: bool,
    pub depth: DimensionField,
    pub direction_deg: f64,
    pub levels: BTreeSet<EntityId>,
    pub sources: BTreeSet<EntityId>,
    pub scale: u32,
    pub generate: bool,
}
fn mm(value: f64) -> Result<DimensionField, String> {
    DimensionField::from_dimension(&length(value), "mm")
}
fn number(d: &multicad_core::constraint_sketch::Dimension) -> Result<f64, String> {
    match d {
        multicad_core::constraint_sketch::Dimension::Literal { value, .. } => Ok(*value),
        multicad_core::constraint_sketch::Dimension::Expression { expression } => Err(format!(
            "Section point {} is an expression; edit it through the API",
            expression.text
        )),
    }
}
fn shown(document: &Document) -> Vec<(EntityId, String)> {
    document
        .entities()
        .filter(|e| {
            matches!(
                e.data,
                EntityData::Part { .. } | EntityData::Instance { .. }
            )
        })
        .map(|e| (e.id, e.name.clone()))
        .collect()
}
impl GuideFields {
    pub fn new(document: &Document) -> Result<Self, String> {
        Ok(Self {
            view: GuideView::Plan,
            cut_height: mm(1000.)?,
            view_depth: mm(1200.)?,
            start: [0., 0.],
            end: [5000., 0.],
            limit_depth: false,
            depth: mm(3000.)?,
            direction_deg: 90.,
            levels: model::levels(document).map(|e| e.id).collect(),
            sources: shown(document).into_iter().map(|(id, _)| id).collect(),
            scale: 50,
            generate: true,
        })
    }
    pub fn load(&mut self, document: &Document, id: EntityId) -> Result<(), String> {
        let guide = drawings::read_guide(document, id).map_err(|e| e.to_string())?;
        match &guide.kind {
            GuideKind::Plan {
                cut_height,
                view_depth,
            } => {
                self.view = GuideView::Plan;
                self.cut_height = DimensionField::from_dimension(cut_height, "mm")?;
                self.view_depth = DimensionField::from_dimension(view_depth, "mm")?;
            }
            GuideKind::Section { start, end, depth } => {
                self.view = GuideView::Section;
                self.start = [number(&start[0])?, number(&start[1])?];
                self.end = [number(&end[0])?, number(&end[1])?];
                self.limit_depth = depth.is_some();
                if let Some(d) = depth {
                    self.depth = DimensionField::from_dimension(d, "mm")?;
                }
            }
            GuideKind::Elevation { direction_deg } => {
                self.view = GuideView::Elevation;
                self.direction_deg = *direction_deg;
            }
        }
        self.levels = guide.levels.into_iter().collect();
        self.sources = guide.sources.into_iter().collect();
        self.scale = guide.scale.denominator;
        Ok(())
    }
    pub fn definition(&self, document: &Document) -> Result<GuideDefinition, String> {
        let kind = match self.view {
            GuideView::Plan => GuideKind::Plan {
                cut_height: self.cut_height.resolve(document)?,
                view_depth: self.view_depth.resolve(document)?,
            },
            GuideView::Section => GuideKind::Section {
                start: self.start.map(length),
                end: self.end.map(length),
                depth: self
                    .limit_depth
                    .then(|| self.depth.resolve(document))
                    .transpose()?,
            },
            GuideView::Elevation => GuideKind::Elevation {
                direction_deg: self.direction_deg,
            },
        };
        // Keep document order so the generated pages follow the levels.
        let guide = GuideDefinition {
            kind,
            levels: model::levels(document)
                .map(|e| e.id)
                .filter(|id| self.levels.contains(id))
                .collect(),
            sources: shown(document)
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| self.sources.contains(id))
                .collect(),
            scale: Scale {
                numerator: 1,
                denominator: self.scale,
            },
        };
        guide.validate().map_err(|e| e.to_string())?;
        Ok(guide)
    }
    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            for (view, label) in [
                (GuideView::Plan, "Floor plans"),
                (GuideView::Section, "Section"),
                (GuideView::Elevation, "Elevation"),
            ] {
                changed |= ui.radio_value(&mut self.view, view, label).changed();
            }
        });
        match self.view {
            GuideView::Plan => {
                changed |= self.cut_height.show(
                    ui,
                    "Cut height",
                    "Height of the horizontal cut above each level's floor",
                );
                changed |= self.view_depth.show(
                    ui,
                    "View depth",
                    "How far below the cut uncut geometry is drawn",
                );
            }
            GuideView::Section => {
                for (label, point) in [
                    ("Line start (x, y mm)", &mut self.start),
                    ("Line end (x, y mm)", &mut self.end),
                ] {
                    ui.label(label);
                    ui.horizontal(|ui| {
                        for v in point.iter_mut() {
                            changed |= ui.add(egui::DragValue::new(v).speed(10.)).changed();
                        }
                    });
                }
                ui.small("The view looks across the line toward its left side; the cut is limited to the line.");
                changed |= ui
                    .checkbox(&mut self.limit_depth, "Limit the depth behind the cut")
                    .changed();
                if self.limit_depth {
                    changed |= self.depth.show(ui, "Depth", "Shown behind the cut plane");
                }
            }
            GuideView::Elevation => {
                ui.horizontal(|ui| {
                    ui.label("Looking toward °");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.direction_deg).range(-360.0..=360.0))
                        .changed();
                })
                .response
                .on_hover_text("0 looks toward +X, 90 toward +Y");
            }
        }
        ui.label(match self.view {
            GuideView::Plan => "Levels (one plan each)",
            _ => "Levels (limit the section height; optional)",
        });
        for level in model::levels(document) {
            let mut on = self.levels.contains(&level.id);
            if ui.checkbox(&mut on, &level.name).changed() {
                changed = true;
                if on {
                    self.levels.insert(level.id);
                } else {
                    self.levels.remove(&level.id);
                }
            }
        }
        ui.label("Shown Parts and Instances");
        egui::ScrollArea::vertical()
            .id_salt("guide-sources")
            .max_height(140.)
            .show(ui, |ui| {
                for (id, name) in shown(document) {
                    let mut on = self.sources.contains(&id);
                    if ui.checkbox(&mut on, name).changed() {
                        changed = true;
                        if on {
                            self.sources.insert(id);
                        } else {
                            self.sources.remove(&id);
                        }
                    }
                }
            });
        ui.horizontal(|ui| {
            ui.label("Scale 1 :");
            changed |= ui
                .add(egui::DragValue::new(&mut self.scale).range(1..=10000))
                .changed();
        });
        changed |= ui
            .checkbox(
                &mut self.generate,
                "Generate or update the drawing pages on Apply",
            )
            .changed();
        ui.small("Generated views stay linked: level and guide edits move their cut.");
        changed
    }
}
