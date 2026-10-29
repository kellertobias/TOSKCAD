//! Sheet-metal inspector: place a rectangular base sheet from sheet stock,
//! or edit an applied sheet part. Thickness comes from the stock; the inside
//! radius and K-factor come from the stock unless overridden here. Sheet
//! stock is defined in Model → Materials.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    materials::{self, LibraryDefinition, StockKind},
    model::{
        self, BodyOutput, ModelPrepared, PanelPlane, SheetBase, SheetDefinition, SheetRequest,
    },
};

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub struct SheetCommand {
    pub editing: bool,
    pub feature: EntityId,
    pub name: String,
    pub sheet: SheetDefinition,
}
impl SheetCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        if self.editing {
            model::prepare_sheet_edit(session, revision, actor, self.feature, &self.sheet)
        } else {
            model::prepare_sheet(
                session,
                revision,
                actor,
                &SheetRequest {
                    feature: self.feature,
                    body: self.sheet.output.body,
                    name: self.name.clone(),
                    sheet: self.sheet.clone(),
                },
            )
        }
        .map_err(|e| e.to_string())
    }
    pub fn edit_id(&self) -> Option<EntityId> {
        self.editing.then_some(self.feature)
    }
}

fn field(value: f64, unit: &'static str) -> DimensionField {
    let d = Dimension::Literal {
        value,
        unit: unit.into(),
    };
    DimensionField::from_dimension(&d, unit).expect("literal field")
}
fn of(d: &Dimension, unit: &'static str) -> Result<DimensionField, String> {
    DimensionField::from_dimension(d, unit)
}
/// Sheet stock entries: (stock, name, thickness, radius, K-factor).
fn sheet_stocks(document: &Document) -> Vec<(EntityId, String, f64, f64, f64)> {
    document
        .entities()
        .filter_map(|e| match materials::read(e).ok().flatten() {
            Some(LibraryDefinition::Stock(s)) => match s.specification {
                StockKind::Sheet {
                    thickness_mm,
                    inside_bend_radius_mm,
                    k_factor,
                    ..
                } => Some((
                    e.id,
                    e.name.clone(),
                    thickness_mm,
                    inside_bend_radius_mm,
                    k_factor,
                )),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub(super) struct SheetPanel {
    stamp: FormStamp,
    existing: Option<EntityId>,
    feature: EntityId,
    body: EntityId,
    name: String,
    stock: Option<EntityId>,
    /// A sketch-region or converted base kept unchanged by this form.
    kept_base: Option<SheetBase>,
    plane: PanelPlane,
    origin: [DimensionField; 3],
    length: DimensionField,
    width: DimensionField,
    reverse: bool,
    radius_override: bool,
    radius: DimensionField,
    k_override: bool,
    k_factor: DimensionField,
    error: Option<String>,
}
impl SheetPanel {
    pub fn new(document: &Document, name: String) -> Self {
        let stocks = sheet_stocks(document);
        let first = stocks.first();
        Self {
            stamp: FormStamp::new(),
            existing: None,
            feature: EntityId::new(),
            body: EntityId::new(),
            name,
            stock: first.map(|s| s.0),
            kept_base: None,
            plane: PanelPlane::Xy,
            origin: [field(0., "mm"), field(0., "mm"), field(0., "mm")],
            length: field(200., "mm"),
            width: field(100., "mm"),
            reverse: false,
            radius_override: false,
            radius: field(first.map_or(1., |s| s.3), "mm"),
            k_override: false,
            k_factor: field(first.map_or(0.4, |s| s.4), "ratio"),
            error: None,
        }
    }
    /// Open an applied sheet part for editing in place.
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let sheet = model::read_sheet(document, feature).map_err(|e| e.to_string())?;
        let mut form = Self::new(document, String::new());
        form.existing = Some(feature);
        form.feature = feature;
        form.body = sheet.output.body;
        form.name = document
            .get(sheet.output.body)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        form.stock = Some(sheet.stock);
        match &sheet.base {
            SheetBase::Rectangle {
                plane,
                origin,
                length,
                width,
            } => {
                form.plane = *plane;
                form.origin = [
                    of(&origin[0], "mm")?,
                    of(&origin[1], "mm")?,
                    of(&origin[2], "mm")?,
                ];
                form.length = of(length, "mm")?;
                form.width = of(width, "mm")?;
            }
            other => form.kept_base = Some(other.clone()),
        }
        form.reverse = sheet.reverse;
        if let Some(r) = &sheet.inside_radius {
            form.radius_override = true;
            form.radius = of(r, "mm")?;
        }
        if let Some(k) = &sheet.k_factor {
            form.k_override = true;
            form.k_factor = of(k, "ratio")?;
        }
        Ok(form)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Base sheet"
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<SheetCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let stock = self
            .stock
            .ok_or("Choose sheet stock; define it in Model → Materials")?;
        let base = match &self.kept_base {
            Some(base) => base.clone(),
            None => SheetBase::Rectangle {
                plane: self.plane,
                origin: [
                    self.origin[0].resolve(document)?,
                    self.origin[1].resolve(document)?,
                    self.origin[2].resolve(document)?,
                ],
                length: self.length.resolve(document)?,
                width: self.width.resolve(document)?,
            },
        };
        Ok(SheetCommand {
            editing: self.existing.is_some(),
            feature: self.feature,
            name: self.name.clone(),
            sheet: SheetDefinition {
                output: BodyOutput { body: self.body },
                stock,
                base,
                reverse: self.reverse,
                inside_radius: self
                    .radius_override
                    .then(|| self.radius.resolve(document))
                    .transpose()?,
                k_factor: self
                    .k_override
                    .then(|| self.k_factor.resolve(document))
                    .transpose()?,
            },
        })
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        let mut changed = false;
        ui.strong(self.label());
        ui.small(
            "A flat sheet-metal Part from sheet stock. The base face is stationary; thickness \
             comes from the stock, and the inside radius and K-factor come from the stock \
             unless you override them.",
        );
        ui.label("Name");
        changed |= ui
            .add_enabled(
                self.existing.is_none(),
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        let stocks = sheet_stocks(document);
        if stocks.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "No sheet stock yet. Define sheet stock in Model → Materials.",
            );
        }
        ui.label("Sheet stock");
        let describe = |s: &(EntityId, String, f64, f64, f64)| {
            format!("{} · {} mm · r {} · K {}", s.1, s.2, s.3, s.4)
        };
        let selected = self
            .stock
            .and_then(|v| stocks.iter().find(|s| s.0 == v))
            .map_or("Choose sheet stock".into(), describe);
        controls::inspector_combo(ui, "sheet-stock")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for s in &stocks {
                    changed |= ui
                        .selectable_value(&mut self.stock, Some(s.0), describe(s))
                        .changed();
                }
            });
        ui.separator();
        if self.kept_base.is_some() {
            ui.small("The base comes from a sketch region or a converted solid and is kept.");
        } else {
            ui.label("Plane");
            controls::inspector_combo(ui, "sheet-plane")
                .selected_text(plane_label(self.plane))
                .show_ui(ui, |ui| {
                    for plane in [PanelPlane::Xy, PanelPlane::Xz, PanelPlane::Yz] {
                        changed |= ui
                            .selectable_value(&mut self.plane, plane, plane_label(plane))
                            .changed();
                    }
                });
            changed |= self
                .length
                .show(ui, "Length (u)", "Base size along the plane's first axis");
            changed |= self
                .width
                .show(ui, "Width (v)", "Base size along the plane's second axis");
            for (axis, f) in ["X", "Y", "Z"].iter().zip(&mut self.origin) {
                changed |= f.show(
                    ui,
                    &format!("Origin {axis}"),
                    "World position of the base sheet's minimum corner",
                );
            }
        }
        changed |= ui
            .checkbox(&mut self.reverse, "Material on the other side")
            .changed();
        changed |= ui
            .checkbox(&mut self.radius_override, "Override inside radius")
            .changed();
        if self.radius_override {
            changed |= self.radius.show(
                ui,
                "Inside radius",
                "Inside bend radius for this part instead of the stock value",
            );
        }
        changed |= ui
            .checkbox(&mut self.k_override, "Override K-factor")
            .changed();
        if self.k_override {
            changed |= self.k_factor.show(
                ui,
                "K-factor",
                "Neutral-axis position as a fraction of thickness, 0–1",
            );
        }
        ui.separator();
        form::preview_status(ui, self.stamp, preview);
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
            self.error = None;
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => match self.request(document, enabled) {
                Ok(request) => Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Sheet(Box::new(request)),
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply)
                if preview.ready(self.stamp) && self.request(document, enabled).is_ok() =>
            {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}
fn plane_label(plane: PanelPlane) -> &'static str {
    match plane {
        PanelPlane::Xy => "XY · thickness up",
        PanelPlane::Xz => "XZ · thickness toward −Y",
        PanelPlane::Yz => "YZ · thickness toward +X",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel},
        document::{Entity, EntityData},
        materials::{Stock, prepare_library},
        model::length,
    };
    use multicad_model::Compiler;

    fn session() -> Session {
        let mut s = Session::new(Document::new("Sheet form").unwrap()).unwrap();
        let steel = Entity::new(
            "DC01",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [180, 184, 190, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: steel.id,
            specification: StockKind::Sheet {
                thickness_mm: 2.,
                length_mm: 2000.,
                width_mm: 1000.,
                inside_bend_radius_mm: 2.,
                k_factor: 0.4,
            },
        })
        .entity(EntityId::new(), "DC01 2.0")
        .unwrap();
        s.commit(prepare_library(&s, 0, Actor::local(Channel::Ui), &[steel, stock]).unwrap())
            .unwrap();
        s
    }

    #[test]
    fn form_places_a_base_sheet_and_edits_its_bend_inputs_in_place() {
        let mut s = session();
        let form = SheetPanel::new(s.document(), "Cover".into());
        assert!(form.stock.is_some());
        let command = form.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), None);
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        let feature = prepared.outputs[0].feature;
        let records = Compiler::default()
            .compile(prepared.prepared.document(), 1)
            .unwrap()
            .sheet_metal(prepared.prepared.document())
            .unwrap();
        assert_eq!(records[0].flat_size_mm, [200., 100.]);
        assert!((records[0].volume_mm3 - 200. * 100. * 2.).abs() < 1e-6);
        s.commit(prepared.prepared).unwrap();

        let mut edit = SheetPanel::existing(s.document(), feature).unwrap();
        assert!(!edit.radius_override);
        edit.radius_override = true;
        edit.radius = field(5., "mm");
        edit.length = field(250., "mm");
        let command = edit.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), Some(feature));
        assert_eq!(command.sheet.inside_radius, Some(length(5.)));
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        let records = Compiler::default()
            .compile(prepared.prepared.document(), 2)
            .unwrap()
            .sheet_metal(prepared.prepared.document())
            .unwrap();
        assert_eq!(records[0].inside_radius_mm, 5.);
        assert_eq!(records[0].flat_size_mm, [250., 100.]);
        // A K-factor outside 0–1 is refused before any geometry exists.
        edit.k_override = true;
        edit.k_factor = field(2., "ratio");
        let command = edit.request(s.document(), true).unwrap();
        assert!(command.prepare(&s, Actor::local(Channel::Ui)).is_err());
    }
}
