//! Explicit Cabinet controls. Each row keeps its canonical shelf slot while
//! the shared typed definition drives Preview, Apply and the viewport guide.
use super::*;
use crate::modeling::{joinery_fields::DowelFields, joint::TenonFields};
use multicad_core::{
    model::{CabinetJoinery, CabinetOptions, CabinetPins, CabinetShelf, ShelfSupport},
    parameters::{self, EvaluatedParameters},
};

#[derive(Clone, Debug)]
struct ShelfFields {
    key: u32,
    underside: DimensionField,
    depth: DimensionField,
    stock: Option<EntityId>,
    adjustable: bool,
    level: u32,
}
#[derive(Clone, Debug)]
struct PinFields {
    diameter: DimensionField,
    depth: DimensionField,
    start: DimensionField,
    end: DimensionField,
    front: DimensionField,
    rear: DimensionField,
}
impl PinFields {
    fn of(p: &CabinetPins) -> Result<Self, String> {
        Ok(Self {
            diameter: of(&p.diameter, "mm")?,
            depth: of(&p.depth, "mm")?,
            start: of(&p.start, "mm")?,
            end: of(&p.end_clearance, "mm")?,
            front: of(&p.front_inset, "mm")?,
            rear: of(&p.rear_inset, "mm")?,
        })
    }
    fn spec(&self, d: &Document) -> Result<CabinetPins, String> {
        Ok(CabinetPins {
            diameter: self.diameter.resolve(d)?,
            depth: self.depth.resolve(d)?,
            start: self.start.resolve(d)?,
            end_clearance: self.end.resolve(d)?,
            front_inset: self.front.resolve(d)?,
            rear_inset: self.rear.resolve(d)?,
        })
    }
    fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = self.diameter.show(
            ui,
            "Pin diameter",
            "The shelf underside rests one radius above a hole center",
        );
        changed |= self.depth.show(
            ui,
            "Pin receiving depth",
            "Blind drilling into each cabinet side",
        );
        changed |= self.start.show(
            ui,
            "First hole above bottom",
            "Hole center measured from the bottom panel top",
        );
        changed |= self.end.show(
            ui,
            "Top hole clearance",
            "From the top panel underside to the final hole center",
        );
        changed |= self.front.show(
            ui,
            "Front support row",
            "From each adjustable shelf's front edge",
        );
        changed |= self.rear.show(
            ui,
            "Rear support row",
            "From each adjustable shelf's rear edge",
        );
        changed
    }
}
#[derive(Clone, Debug)]
pub(super) struct CabinetFields {
    pub(super) toe: DimensionField,
    pub(super) stock: Option<EntityId>,
    shelves: Vec<ShelfFields>,
    domino: bool,
    dowels: DowelFields,
    tenon: TenonFields,
    pins: PinFields,
}
pub(super) fn value(
    field: &DimensionField,
    d: &Document,
    p: &EvaluatedParameters,
) -> Result<f64, String> {
    field
        .resolve(d)?
        .value_in(multicad_core::parameters::Unit::Millimeter, Some(p))
}
impl CabinetFields {
    pub(super) fn new(stock: Option<EntityId>, thickness: f64) -> Self {
        Self {
            toe: field(100., "mm"),
            stock,
            shelves: vec![ShelfFields {
                key: 0,
                underside: field(360., "mm"),
                depth: field(500., "mm"),
                stock: None,
                adjustable: false,
                level: 0,
            }],
            domino: false,
            dowels: DowelFields::fitted(thickness),
            tenon: TenonFields::default(),
            pins: PinFields::of(&CabinetPins::default()).expect("literal pin settings"),
        }
    }
    pub(super) fn of(c: &CabinetOptions) -> Result<Self, String> {
        let mut out = Self::new(Some(c.shelf_stock), 18.);
        out.toe = of(&c.toe_clearance, "mm")?;
        out.shelves = c
            .shelves
            .iter()
            .map(|s| {
                Ok(ShelfFields {
                    key: s.key,
                    underside: of(&s.underside, "mm")?,
                    depth: of(&s.depth, "mm")?,
                    stock: s.stock,
                    adjustable: matches!(s.support, ShelfSupport::System32 { .. }),
                    level: match s.support {
                        ShelfSupport::System32 { level } => level,
                        _ => 0,
                    },
                })
            })
            .collect::<Result<_, String>>()?;
        match &c.joinery {
            CabinetJoinery::Dowels(spec) => out.dowels = DowelFields::of(spec)?,
            CabinetJoinery::Domino(spec) => {
                out.domino = true;
                out.tenon = TenonFields::of(spec)?;
            }
        }
        out.pins = PinFields::of(&c.pins)?;
        Ok(out)
    }
    pub(super) fn spec(&self, d: &Document) -> Result<CabinetOptions, String> {
        Ok(CabinetOptions {
            toe_clearance: self.toe.resolve(d)?,
            shelf_stock: self.stock.ok_or("Choose the default shelf stock")?,
            shelves: self
                .shelves
                .iter()
                .map(|s| {
                    Ok(CabinetShelf {
                        key: s.key,
                        underside: s.underside.resolve(d)?,
                        depth: s.depth.resolve(d)?,
                        stock: s.stock,
                        support: if s.adjustable {
                            ShelfSupport::System32 { level: s.level }
                        } else {
                            ShelfSupport::Fixed
                        },
                    })
                })
                .collect::<Result<_, String>>()?,
            joinery: if self.domino {
                CabinetJoinery::Domino(Box::new(self.tenon.spec(d)?))
            } else {
                CabinetJoinery::Dowels(Box::new(self.dowels.spec(d)?))
            },
            pins: self.pins.spec(d)?,
        })
    }
    fn next_position(
        &self,
        d: &Document,
        p: &EvaluatedParameters,
        stocks: &[(EntityId, String, f64)],
        body_t: f64,
        height: f64,
    ) -> Result<f64, String> {
        let c = self.spec(d)?;
        let thickness = stocks
            .iter()
            .find(|(id, ..)| *id == c.shelf_stock)
            .map(|(_, _, t)| *t)
            .ok_or("Choose default shelf stock")?;
        let mut occupied = vec![];
        for shelf in &c.shelves {
            let z = c
                .underside_mm(shelf, p, body_t)
                .map_err(|e| e.to_string())?;
            let t = stocks
                .iter()
                .find(|(id, ..)| *id == shelf.stock.unwrap_or(c.shelf_stock))
                .map(|(_, _, t)| *t)
                .ok_or("A shelf stock is missing")?;
            occupied.push((z, z + t));
        }
        occupied.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cursor = value(&self.toe, d, p)? + body_t;
        let ceiling = height - body_t;
        let mut largest = (0., 0.);
        for (lo, hi) in occupied.into_iter().chain([(ceiling, ceiling)]) {
            if lo - cursor > largest.1 - largest.0 {
                largest = (cursor, lo);
            }
            cursor = cursor.max(hi);
        }
        if largest.1 - largest.0 < thickness + 10. {
            return Err(
                "No free gap for another shelf; change the height or existing positions".into(),
            );
        }
        Ok((largest.0 + largest.1 - thickness) / 2.)
    }
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        d: &Document,
        stocks: &[(EntityId, String, f64)],
        body_t: Option<f64>,
        height: Option<f64>,
        available_depth: Option<f64>,
    ) -> bool {
        let mut changed = false;
        ui.strong("Individual shelves");
        ui.label("Default shelf material / stock");
        changed |= stock_combo(ui, "default-shelf-stock", stocks, &mut self.stock, false).0;
        ui.small(
            "New shelves inherit this stock. Explicit overrides keep their material and thickness.",
        );
        let parameters = parameters::evaluate(d).ok();
        let stock_name = |id: Option<EntityId>| {
            id.and_then(|id| stocks.iter().find(|(s, ..)| *s == id))
                .map(|(_, n, t)| format!("{n} · {t} mm"))
                .unwrap_or_else(|| "Choose stock".into())
        };
        let defaults = stock_name(self.stock);
        let grid = parameters.as_ref().and_then(|p| {
            Some((
                value(&self.toe, d, p).ok()?
                    + body_t?
                    + value(&self.pins.start, d, p).ok()?
                    + value(&self.pins.diameter, d, p).ok()? / 2.,
                value(&self.pins.end, d, p).ok()?,
            ))
        });
        let mut remove = None;
        for shelf in &mut self.shelves {
            ui.push_id(shelf.key, |ui| {
                let title = format!(
                    "Shelf {} · {}",
                    shelf.key + 1,
                    if shelf.adjustable {
                        "System 32"
                    } else {
                        "Fixed"
                    }
                );
                egui::CollapsingHeader::new(title)
                    .default_open(true)
                    .show(ui, |ui| {
                        let previous = shelf.adjustable;
                        let effective = if shelf.adjustable {
                            grid.map(|(first, _)| {
                                first + shelf.level as f64 * model::SYSTEM32_PITCH_MM
                            })
                        } else {
                            parameters
                                .as_ref()
                                .and_then(|p| value(&shelf.underside, d, p).ok())
                        };
                        ui.label("Support");
                        ui.horizontal_wrapped(|ui| {
                            changed |= ui
                                .radio_value(&mut shelf.adjustable, false, "Fixed")
                                .changed();
                            changed |= ui
                                .radio_value(&mut shelf.adjustable, true, "System 32 adjustable")
                                .changed();
                        });
                        if previous != shelf.adjustable {
                            if shelf.adjustable {
                                if let (Some(z), Some((first, _))) = (effective, grid) {
                                    shelf.level =
                                        ((z - first) / model::SYSTEM32_PITCH_MM).round().max(0.)
                                            as u32;
                                }
                            } else if let Some(z) = effective {
                                shelf.underside = field(z, "mm");
                            }
                        }
                        if shelf.adjustable {
                            ui.horizontal(|ui| {
                                ui.label("Support level");
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut shelf.level).range(0..=1_000_000),
                                    )
                                    .changed();
                            });
                            if let Some((first, _)) = grid {
                                ui.small(format!(
                                    "Underside from floor: {} · 32 mm pitch",
                                    cabinet_edit::format_mm(
                                        first + shelf.level as f64 * model::SYSTEM32_PITCH_MM
                                    )
                                ));
                            }
                        } else {
                            changed |= shelf.underside.show(
                                ui,
                                "Underside from floor",
                                "Local floor datum to this shelf underside",
                            );
                        }
                        changed |= shelf.depth.show(
                            ui,
                            "Shelf depth",
                            "Finished front-to-back dimension; never shortened silently",
                        );
                        ui.label("Shelf material / stock");
                        controls::inspector_combo(ui, "shelf-stock")
                            .selected_text(shelf.stock.map_or_else(
                                || format!("Default · {defaults}"),
                                |id| stock_name(Some(id)),
                            ))
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut shelf.stock,
                                        None,
                                        format!("Default · {defaults}"),
                                    )
                                    .changed();
                                for (id, name, t) in stocks {
                                    changed |= ui
                                        .selectable_value(
                                            &mut shelf.stock,
                                            Some(*id),
                                            format!("{name} · {t} mm"),
                                        )
                                        .changed();
                                }
                            });
                        if ui.small_button("Remove shelf").clicked() {
                            remove = Some(shelf.key);
                        }
                    });
            });
        }
        if let Some(key) = remove {
            self.shelves.retain(|s| s.key != key);
            changed = true;
        }
        let new_position = match (parameters.as_ref(), body_t, height) {
            (Some(p), Some(t), Some(h)) => self.next_position(d, p, stocks, t, h),
            _ => Err("Enter valid cabinet dimensions and stock first".into()),
        };
        let can_add = self.shelves.len() < model::MAX_SHELVES
            && new_position.is_ok()
            && available_depth.is_some_and(|d| d > 0.);
        if ui
            .add_enabled(can_add, egui::Button::new("Add shelf"))
            .on_disabled_hover_text(new_position.as_ref().err().map_or(
                "The cabinet has reached its shelf limit or has no usable depth",
                String::as_str,
            ))
            .clicked()
        {
            let key = (0..model::MAX_SHELVES as u32)
                .find(|k| !self.shelves.iter().any(|s| s.key == *k))
                .expect("free shelf slot");
            self.shelves.push(ShelfFields {
                key,
                underside: field(new_position.expect("enabled valid gap"), "mm"),
                depth: field(
                    available_depth.expect("enabled valid depth").min(500.),
                    "mm",
                ),
                stock: None,
                adjustable: false,
                level: 0,
            });
            changed = true;
        }
        ui.separator();
        ui.strong("Fixed connections");
        ui.small("Side, top, bottom and fixed-shelf connections. The back retains its inset or overlay construction.");
        ui.horizontal(|ui| {
            changed |= ui.radio_value(&mut self.domino, false, "Dowels").changed();
            changed |= ui.radio_value(&mut self.domino, true, "Domino").changed();
        });
        egui::CollapsingHeader::new("Connection sizes and spacing").show(ui, |ui| {
            if self.domino {
                changed |= self.tenon.show(ui);
            } else {
                changed |= self.dowels.show(ui);
            }
        });
        if self.shelves.iter().any(|s| s.adjustable) {
            egui::CollapsingHeader::new("System 32 support rows · 32 mm pitch").show(ui, |ui| {
                changed |= self.pins.show(ui);
            });
        }
        changed
    }
}
