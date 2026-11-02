//! Cabinet inspector: build or edit a parametric carcass from an envelope, or
//! place a rectangular panel from panel stock with grain, banding and
//! oversize. Panel stock is defined in Model → Materials.
use super::{
    FormStamp, PreviewState,
    cabinet_edit::{self, CabinetGuide},
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
mod fields;
use crate::{controls, egui};
use fields::CabinetFields;
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    materials::{self, LibraryDefinition, StockKind},
    model::{
        self, Back, Band, BodyOutput, CarcassDefinition, CarcassRequest, Construction, GrainAxis,
        ModelPrepared, PanelDefinition, PanelEdges, PanelPlane, PanelRequest, PanelSource,
        ShelfLayout, Shelves, angle, length,
    },
};

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub enum CabinetCommand {
    Carcass {
        existing: Option<EntityId>,
        request: Box<CarcassRequest>,
    },
    Panel {
        editing: bool,
        feature: EntityId,
        name: String,
        panel: Box<PanelDefinition>,
    },
}
impl CabinetCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        match self {
            Self::Carcass {
                existing: Some(id),
                request,
            } => model::prepare_carcass_edit(session, revision, actor, *id, &request.definition)
                .map(|p| p.model),
            Self::Carcass {
                existing: None,
                request,
            } => model::prepare_carcass(session, revision, actor, request).map(|p| p.model),
            Self::Panel {
                editing: true,
                feature,
                panel,
                ..
            } => model::prepare_panel_edit(session, revision, actor, *feature, panel),
            Self::Panel {
                editing: false,
                feature,
                name,
                panel,
            } => model::prepare_panel(
                session,
                revision,
                actor,
                &PanelRequest {
                    feature: *feature,
                    body: panel.output.body,
                    name: name.clone(),
                    panel: (**panel).clone(),
                },
            ),
        }
        .map_err(|e| e.to_string())
    }
    /// Panel edits keep their feature; carcass edits add or remove panels and
    /// run as source changes like levels.
    pub fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::Panel {
                editing: true,
                feature,
                ..
            } => Some(*feature),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Carcass,
    Panel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackKind {
    None,
    Inset,
    Overlay,
}

fn field(value: f64, unit: &'static str) -> DimensionField {
    let d = if unit == "deg" {
        angle(value)
    } else {
        length(value)
    };
    DimensionField::from_dimension(&d, unit).expect("literal field")
}
fn of(d: &Dimension, unit: &'static str) -> Result<DimensionField, String> {
    DimensionField::from_dimension(d, unit)
}
/// Panel stock entries: (stock, name, thickness).
fn panel_stocks(document: &Document) -> Vec<(EntityId, String, f64)> {
    document
        .entities()
        .filter_map(|e| match materials::read(e).ok().flatten() {
            Some(LibraryDefinition::Stock(s)) => match s.specification {
                StockKind::Panel { thickness_mm, .. } => Some((e.id, e.name.clone(), thickness_mm)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}
fn stock_combo(
    ui: &mut egui::Ui,
    id: &str,
    stocks: &[(EntityId, String, f64)],
    value: &mut Option<EntityId>,
) -> bool {
    let selected = value
        .and_then(|v| stocks.iter().find(|(s, ..)| *s == v))
        .map_or("Choose panel stock".into(), |(_, n, t)| {
            format!("{n} · {t} mm")
        });
    let mut changed = false;
    controls::inspector_combo(ui, id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (s, n, t) in stocks {
                changed |= ui
                    .selectable_value(value, Some(*s), format!("{n} · {t} mm"))
                    .changed();
            }
        });
    changed
}

pub(super) struct CabinetPanel {
    stamp: FormStamp,
    kind: Kind,
    /// The carcass or panel feature being edited.
    existing: Option<EntityId>,
    feature: EntityId,
    body: EntityId,
    name: String,
    stock: Option<EntityId>,
    origin: [DimensionField; 3],
    // Carcass
    rotation: DimensionField,
    width: DimensionField,
    height: DimensionField,
    depth: DimensionField,
    construction: Construction,
    back: BackKind,
    back_stock: Option<EntityId>,
    recess: DimensionField,
    offsets: bool,
    shelf_count: u32,
    shelf_offsets: String,
    setback: DimensionField,
    clearance: DimensionField,
    options: Option<CabinetFields>,
    group: bool,
    // Both: front band on carcasses, chosen edges on panels.
    banded: bool,
    band: DimensionField,
    allowance: DimensionField,
    // Panel
    plane: PanelPlane,
    length: DimensionField,
    panel_width: DimensionField,
    grain: GrainAxis,
    edges: [bool; 4],
    oversize: DimensionField,
    error: Option<String>,
    /// Display-only envelope guide for (stamp, source revision).
    guide: Option<(FormStamp, u64, Option<CabinetGuide>)>,
}
impl CabinetPanel {
    pub fn new(document: &Document, name: String) -> Self {
        let stocks = panel_stocks(document);
        let thinnest = stocks
            .iter()
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|s| s.0);
        let thickest = stocks
            .iter()
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .map(|s| s.0);
        Self {
            stamp: FormStamp::new(),
            kind: Kind::Carcass,
            existing: None,
            feature: EntityId::new(),
            body: EntityId::new(),
            name,
            stock: thickest,
            origin: [field(0., "mm"), field(0., "mm"), field(0., "mm")],
            rotation: field(0., "deg"),
            width: field(600., "mm"),
            height: field(720., "mm"),
            depth: field(560., "mm"),
            construction: Construction::SidesFullHeight,
            back: if thinnest.is_some() && thinnest != thickest {
                BackKind::Inset
            } else {
                BackKind::None
            },
            back_stock: thinnest,
            recess: field(10., "mm"),
            offsets: false,
            shelf_count: 1,
            shelf_offsets: "250, 450".into(),
            setback: field(20., "mm"),
            clearance: field(1., "mm"),
            options: None,
            group: true,
            banded: true,
            band: field(2., "mm"),
            allowance: field(0., "mm"),
            plane: PanelPlane::Xy,
            length: field(800., "mm"),
            panel_width: field(400., "mm"),
            grain: GrainAxis::U,
            edges: [false; 4],
            oversize: field(0., "mm"),
            error: None,
            guide: None,
        }
    }
    pub fn cabinet(document: &Document, name: String) -> Self {
        let mut form = Self::new(document, name);
        let thickness = panel_stocks(document)
            .iter()
            .find(|(id, ..)| Some(*id) == form.stock)
            .map_or(18., |(_, _, t)| *t);
        form.options = Some(CabinetFields::new(form.stock, thickness));
        form
    }
    pub fn panel(document: &Document, name: String) -> Self {
        let mut form = Self::new(document, name);
        form.kind = Kind::Panel;
        form
    }
    /// Viewport dimensions of the carcass being drafted, planned exactly as
    /// Apply would plan it; `None` for panels or an invalid draft.
    pub fn guide(&mut self, document: &Document, revision: u64) -> Option<&CabinetGuide> {
        if self
            .guide
            .as_ref()
            .is_none_or(|(stamp, rev, _)| *stamp != self.stamp || *rev != revision)
        {
            let guide = match self.request(document, true) {
                Ok(CabinetCommand::Carcass { request, .. }) => {
                    cabinet_edit::carcass_guide(document, &request.definition).ok()
                }
                _ => None,
            };
            self.guide = Some((self.stamp, revision, guide));
        }
        self.guide.as_ref().and_then(|(.., g)| g.as_ref())
    }
    /// Open an applied panel: a carcass panel opens its carcass.
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let panel = model::read_panel(document, feature).map_err(|e| e.to_string())?;
        let name = |id| document.get(id).map(|e| e.name.clone()).unwrap_or_default();
        let mut form = Self::new(document, String::new());
        match &panel.source {
            PanelSource::Carcass { carcass, .. } => {
                let c = model::read_carcass(document, *carcass).map_err(|e| e.to_string())?;
                form.kind = Kind::Carcass;
                form.existing = Some(*carcass);
                form.name = name(*carcass);
                form.stock = Some(c.stock);
                form.options = c.cabinet.as_ref().map(CabinetFields::of).transpose()?;
                form.origin = [
                    of(&c.origin[0], "mm")?,
                    of(&c.origin[1], "mm")?,
                    of(&c.origin[2], "mm")?,
                ];
                form.rotation = of(&c.rotation, "deg")?;
                form.width = of(&c.width, "mm")?;
                form.height = of(&c.height, "mm")?;
                form.depth = of(&c.depth, "mm")?;
                form.construction = c.construction;
                (form.back, form.back_stock) = match &c.back {
                    Back::None => (BackKind::None, form.back_stock),
                    Back::Inset { stock, recess } => {
                        form.recess = of(recess, "mm")?;
                        (BackKind::Inset, Some(*stock))
                    }
                    Back::Overlay { stock } => (BackKind::Overlay, Some(*stock)),
                };
                match &c.shelves.layout {
                    ShelfLayout::Even { count } => {
                        form.offsets = false;
                        form.shelf_count = *count;
                    }
                    ShelfLayout::Offsets { offsets } => {
                        form.offsets = true;
                        form.shelf_offsets = offsets
                            .iter()
                            .map(|d| match d {
                                Dimension::Literal { value, .. } => value.to_string(),
                                Dimension::Expression { expression } => expression.text.clone(),
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                    }
                }
                form.setback = of(&c.shelves.setback, "mm")?;
                form.clearance = of(&c.shelves.clearance, "mm")?;
                form.group = c.group.is_some();
                form.banded = c.front_band.is_some();
                if let Some(b) = &c.front_band {
                    form.band = of(&b.thickness, "mm")?;
                    form.allowance = of(b.allowance.as_ref().unwrap_or(&length(0.)), "mm")?;
                }
            }
            PanelSource::Rectangle {
                stock,
                plane,
                origin,
                length: l,
                width: w,
            } => {
                form.kind = Kind::Panel;
                form.existing = Some(feature);
                form.feature = feature;
                form.body = panel.output.body;
                form.name = name(panel.output.body);
                form.stock = Some(*stock);
                form.plane = *plane;
                form.origin = [
                    of(&origin[0], "mm")?,
                    of(&origin[1], "mm")?,
                    of(&origin[2], "mm")?,
                ];
                form.length = of(l, "mm")?;
                form.panel_width = of(w, "mm")?;
                form.grain = panel.grain;
                let bands = panel.edges.bands();
                form.edges = bands.map(|b| b.is_some());
                form.banded = bands.iter().any(Option::is_some);
                if let Some(b) = bands.into_iter().flatten().next() {
                    form.band = of(&b.thickness, "mm")?;
                    form.allowance = of(b.allowance.as_ref().unwrap_or(&length(0.)), "mm")?;
                }
                if let Some(o) = &panel.oversize {
                    form.oversize = of(o, "mm")?;
                }
            }
            PanelSource::WoodBox { .. } => {
                return Err("Open the Wood Box recipe to edit this generated face".into());
            }
            PanelSource::Profile { .. } => {
                return Err("Sketch-region panels are edited through the MCP API for now".into());
            }
        }
        Ok(form)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        match self.kind {
            Kind::Carcass if self.options.is_some() => "Cabinet",
            Kind::Carcass => "Carcass",
            Kind::Panel => "Panel",
        }
    }
    fn band(&self, document: &Document) -> Result<Band, String> {
        Ok(Band {
            thickness: self.band.resolve(document)?,
            allowance: Some(self.allowance.resolve(document)?),
        })
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<CabinetCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let stock = self
            .stock
            .ok_or("Choose panel stock; define it in Model → Materials")?;
        let origin = [
            self.origin[0].resolve(document)?,
            self.origin[1].resolve(document)?,
            self.origin[2].resolve(document)?,
        ];
        match self.kind {
            Kind::Carcass => {
                let back_stock = || self.back_stock.ok_or("Choose the back panel stock");
                let layout = if self.options.is_some() {
                    ShelfLayout::Even { count: 0 }
                } else if self.offsets {
                    ShelfLayout::Offsets {
                        offsets: self
                            .shelf_offsets
                            .split(',')
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| {
                                s.trim().parse::<f64>().map(length).map_err(|_| {
                                    format!("Shelf offset \"{}\" is not a number", s.trim())
                                })
                            })
                            .collect::<Result<_, _>>()?,
                    }
                } else {
                    ShelfLayout::Even {
                        count: self.shelf_count,
                    }
                };
                let definition = CarcassDefinition {
                    cabinet: self
                        .options
                        .as_ref()
                        .map(|o| o.spec(document))
                        .transpose()?,
                    origin,
                    rotation: self.rotation.resolve(document)?,
                    width: self.width.resolve(document)?,
                    height: self.height.resolve(document)?,
                    depth: self.depth.resolve(document)?,
                    stock,
                    construction: self.construction,
                    back: match self.back {
                        BackKind::None => Back::None,
                        BackKind::Inset => Back::Inset {
                            stock: back_stock()?,
                            recess: self.recess.resolve(document)?,
                        },
                        BackKind::Overlay => Back::Overlay {
                            stock: back_stock()?,
                        },
                    },
                    shelves: Shelves {
                        layout,
                        setback: self.setback.resolve(document)?,
                        clearance: self.clearance.resolve(document)?,
                    },
                    front_band: self.banded.then(|| self.band(document)).transpose()?,
                    group: None,
                };
                Ok(CabinetCommand::Carcass {
                    existing: self.existing,
                    request: Box::new(CarcassRequest {
                        carcass: self.feature,
                        name: self.name.clone(),
                        definition,
                        group: self.group,
                    }),
                })
            }
            Kind::Panel => {
                let band = self.banded.then(|| self.band(document)).transpose()?;
                let pick = |i: usize| band.clone().filter(|_| self.edges[i]);
                let oversize = self.oversize.resolve(document)?;
                let zero = matches!(oversize, Dimension::Literal { value, .. } if value == 0.);
                Ok(CabinetCommand::Panel {
                    editing: self.existing.is_some(),
                    feature: self.feature,
                    name: self.name.clone(),
                    panel: Box::new(PanelDefinition {
                        output: BodyOutput { body: self.body },
                        source: PanelSource::Rectangle {
                            stock,
                            plane: self.plane,
                            origin,
                            length: self.length.resolve(document)?,
                            width: self.panel_width.resolve(document)?,
                        },
                        grain: self.grain,
                        edges: PanelEdges {
                            u_start: pick(0),
                            u_end: pick(1),
                            v_start: pick(2),
                            v_end: pick(3),
                        },
                        oversize: (!zero).then_some(oversize),
                    }),
                })
            }
        }
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
        if self.existing.is_none() {
            ui.horizontal(|ui| {
                changed |= ui
                    .radio_value(
                        &mut self.kind,
                        Kind::Carcass,
                        if self.options.is_some() {
                            "Cabinet"
                        } else {
                            "Carcass (legacy)"
                        },
                    )
                    .changed();
                changed |= ui
                    .radio_value(&mut self.kind, Kind::Panel, "Panel")
                    .changed();
            });
        }
        ui.strong(self.label());
        ui.small(match (self.kind, self.existing.is_some()) {
            (Kind::Carcass, false) => "Sides, top, bottom, back and shelves from an outside envelope. Each panel is a named Part you can select.",
            (Kind::Carcass, true) => "Edit the carcass. Panels whose role remains keep their Parts; shelves and backs are added or removed.",
            (Kind::Panel, _) => "A rectangular panel from panel stock. Thickness comes from the stock; banding and oversize change the core cut size.",
        });
        ui.label("Name");
        changed |= ui
            .add_enabled(
                self.existing.is_none(),
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        let stocks = panel_stocks(document);
        if stocks.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "No panel stock yet. Define panel stock in Model → Materials.",
            );
        }
        ui.label(match self.kind {
            Kind::Carcass if self.options.is_some() => "Main body material / stock",
            Kind::Carcass => "Carcass stock",
            Kind::Panel => "Panel stock",
        });
        changed |= stock_combo(ui, "cabinet-stock", &stocks, &mut self.stock);
        ui.separator();
        match self.kind {
            Kind::Carcass => changed |= self.show_carcass(ui, document, &stocks),
            Kind::Panel => changed |= self.show_panel(ui),
        }
        ui.separator();
        if self.kind == Kind::Carcass {
            self.show_role_changes(ui, document, enabled);
            ui.small("The viewport shows the envelope dimensions and shelf heights of this draft.");
        }
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
                    request: ModelRequest::Cabinet(Box::new(request)),
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
    fn show_role_changes(&self, ui: &mut egui::Ui, document: &Document, enabled: bool) {
        let (Some(carcass), Ok(CabinetCommand::Carcass { request, .. })) =
            (self.existing, self.request(document, enabled))
        else {
            return;
        };
        let changes = cabinet_edit::role_changes(document, carcass, &request.definition);
        ui.small(if self.options.is_some(){format!("Keeps {} panel Parts and their user-authored features. Generated connections follow these settings.",changes.kept)}else{format!("Keeps {} panel(s) with their Parts, drilling and joints.",changes.kept)});
        if !changes.added.is_empty() {
            ui.small(format!("Adds: {}", changes.added.join(", ")));
        }
        for (name, blocker) in &changes.removed {
            match blocker {
                Some(by) => ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!(
                        "Cannot remove {name}: {by} uses it. Delete {by} first or keep the panel."
                    ),
                ),
                None => ui.colored_label(
                    egui::Color32::LIGHT_YELLOW,
                    format!("Removes {name} and its manufacturing record."),
                ),
            };
        }
    }
    fn show_origin(&mut self, ui: &mut egui::Ui, help: &str) -> bool {
        let mut changed = false;
        for (axis, f) in ["X", "Y", "Z"].iter().zip(&mut self.origin) {
            changed |= f.show(ui, &format!("Origin {axis}"), help);
        }
        changed
    }
    fn show_band(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = self.band.show(
            ui,
            "Band thickness",
            "Outside the core; the finished size includes it",
        );
        changed |= self.allowance.show(
            ui,
            "Band allowance",
            "Extra core per banded edge, trimmed flush after banding",
        );
        changed
    }
    fn show_carcass(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        stocks: &[(EntityId, String, f64)],
    ) -> bool {
        let mut changed = false;
        changed |= self.width.show(ui, "Width", "Outside width");
        changed |= self.height.show(
            ui,
            if self.options.is_some() {
                "Floor-to-top height"
            } else {
                "Height"
            },
            "Outside height measured from the local floor datum",
        );
        changed |= self.depth.show(ui, "Depth", "Outside depth, back included");
        if let Some(options) = &mut self.options {
            changed |= options.toe.show(
                ui,
                "Toe-kick clearance",
                "Local floor to the bottom panel underside; no front recess is implied",
            );
            egui::CollapsingHeader::new("Placement").show(ui, |ui| {
                changed |= self.show_origin(ui, "World position of the local floor datum");
                changed |=
                    self.rotation
                        .show(ui, "Rotation", "About +Z through the local floor datum");
            });
        } else {
            changed |= self.show_origin(ui, "World position of the front-left-bottom corner");
            changed |= self.rotation.show(
                ui,
                "Rotation",
                "About +Z through the origin; unrotated, the front faces −Y",
            );
        }
        ui.label("Construction");
        changed |= ui
            .radio_value(
                &mut self.construction,
                Construction::SidesFullHeight,
                "Full-height sides",
            )
            .changed();
        changed |= ui
            .radio_value(
                &mut self.construction,
                Construction::TopBottomFullWidth,
                "Full-width top and bottom",
            )
            .changed();
        ui.separator();
        ui.label("Back");
        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(&mut self.back, BackKind::None, "None")
                .changed();
            changed |= ui
                .radio_value(&mut self.back, BackKind::Inset, "Inset")
                .changed();
            changed |= ui
                .radio_value(&mut self.back, BackKind::Overlay, "Overlay")
                .changed();
        });
        if self.back != BackKind::None {
            ui.label("Back material / stock");
            changed |= stock_combo(ui, "cabinet-back-stock", stocks, &mut self.back_stock);
        }
        if self.back == BackKind::Inset {
            changed |= self
                .recess
                .show(ui, "Back recess", "Distance in from the rear edges");
        }
        ui.separator();
        if self.options.is_none() {
            ui.label("Legacy shelf layout");
            ui.horizontal(|ui| {
                changed |= ui.radio_value(&mut self.offsets, false, "Even").changed();
                changed |= ui.radio_value(&mut self.offsets, true, "Offsets").changed();
            });
            if self.offsets {
                ui.label("Offsets from the interior bottom (mm, comma-separated)");
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.shelf_offsets)
                            .desired_width(ui.available_width()),
                    )
                    .changed();
            } else {
                ui.horizontal(|ui| {
                    ui.label("Count");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.shelf_count)
                                .range(0..=model::MAX_SHELVES as u32),
                        )
                        .changed();
                });
            }
        }
        changed |= self.setback.show(
            ui,
            "Shelf setback",
            "From the cabinet front to the shelf front",
        );
        changed |= self.clearance.show(
            ui,
            "Adjustable shelf clearance",
            "Gap at each adjustable shelf end; fixed shelves meet their receiving sides",
        );
        let parameters = multicad_core::parameters::evaluate(document).ok();
        let numeric = |f: &DimensionField| {
            parameters
                .as_ref()
                .and_then(|p| fields::value(f, document, p).ok())
        };
        let body_t = stocks
            .iter()
            .find(|(id, ..)| Some(*id) == self.stock)
            .map(|(_, _, t)| *t);
        let back_t = stocks
            .iter()
            .find(|(id, ..)| Some(*id) == self.back_stock)
            .map(|(_, _, t)| *t);
        let available_depth = numeric(&self.depth).and_then(|d| {
            Some(
                match self.back {
                    BackKind::None => d,
                    BackKind::Inset => d - back_t? - numeric(&self.recess)?,
                    BackKind::Overlay => d - back_t?,
                } - numeric(&self.setback)?,
            )
        });
        let height = numeric(&self.height);
        if let Some(options) = &mut self.options {
            changed |= options.show(ui, document, stocks, body_t, height, available_depth);
        }
        ui.separator();
        changed |= ui
            .checkbox(&mut self.banded, "Band the front edges")
            .changed();
        if self.banded {
            changed |= self.show_band(ui);
        }
        if self.existing.is_none() {
            changed |= ui.checkbox(&mut self.group, "Group the panels").changed();
        }
        changed
    }
    fn show_panel(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.label("Plane");
        controls::inspector_combo(ui, "panel-plane")
            .selected_text(match self.plane {
                PanelPlane::Xy => "XY · thickness up",
                PanelPlane::Xz => "XZ · thickness toward −Y",
                PanelPlane::Yz => "YZ · thickness toward +X",
            })
            .show_ui(ui, |ui| {
                for (plane, label) in [
                    (PanelPlane::Xy, "XY · thickness up"),
                    (PanelPlane::Xz, "XZ · thickness toward −Y"),
                    (PanelPlane::Yz, "YZ · thickness toward +X"),
                ] {
                    changed |= ui.selectable_value(&mut self.plane, plane, label).changed();
                }
            });
        changed |= self.length.show(
            ui,
            "Length (u)",
            "Finished size along the plane's first axis",
        );
        changed |= self.panel_width.show(
            ui,
            "Width (v)",
            "Finished size along the plane's second axis",
        );
        changed |= self.show_origin(ui, "World position of the panel's minimum corner");
        ui.label("Grain");
        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(&mut self.grain, GrainAxis::U, "Along length")
                .changed();
            changed |= ui
                .radio_value(&mut self.grain, GrainAxis::V, "Along width")
                .changed();
            changed |= ui
                .radio_value(&mut self.grain, GrainAxis::Free, "Any")
                .changed();
        });
        changed |= ui.checkbox(&mut self.banded, "Edge banding").changed();
        if self.banded {
            ui.horizontal_wrapped(|ui| {
                for (i, label) in ["Length start", "Length end", "Width start", "Width end"]
                    .into_iter()
                    .enumerate()
                {
                    changed |= ui.checkbox(&mut self.edges[i], label).changed();
                }
            });
            changed |= self.show_band(ui);
        }
        changed |= self.oversize.show(
            ui,
            "Oversize",
            "Extra core added to both cut dimensions, for trimming",
        );
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel},
        document::{Entity, EntityData},
        materials::{Grain, Stock, prepare_library},
    };
    use multicad_model::Compiler;

    fn session() -> Session {
        let mut s = Session::new(Document::new("Cabinet form").unwrap()).unwrap();
        let birch = Entity::new(
            "Birch",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [210, 180, 140, 255],
                section_hatch: None,
            },
        );
        let stock = |t: f64, name: &str| {
            LibraryDefinition::Stock(Stock {
                material: birch.id,
                specification: StockKind::Panel {
                    thickness_mm: t,
                    length_mm: 2500.,
                    width_mm: 1250.,
                    grain: Grain::AlongLength,
                },
            })
            .entity(EntityId::new(), name)
            .unwrap()
        };
        let entities = [birch.clone(), stock(18., "Ply 18"), stock(6., "Ply 6")];
        s.commit(prepare_library(&s, 0, Actor::local(Channel::Ui), &entities).unwrap())
            .unwrap();
        s
    }

    #[test]
    fn form_builds_a_carcass_and_reopens_it_from_any_panel() {
        let mut s = session();
        let mut form = CabinetPanel::new(s.document(), "Base".into());
        // Thickest stock for the carcass, thinnest for an inset back.
        assert_eq!(form.back, BackKind::Inset);
        form.shelf_count = 2;
        let command = form.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), None);
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(prepared.outputs.len(), 7);
        let model = Compiler::default()
            .compile(prepared.prepared.document(), 1)
            .unwrap();
        let list = model.cut_list(prepared.prepared.document()).unwrap();
        let top = list.iter().find(|r| r.name == "Base top").unwrap();
        assert_eq!(top.cut.finished_mm, [564., 560.]);
        assert_eq!(top.cut.core_mm, [564., 558.]);
        let shelf_feature = list
            .iter()
            .find(|r| r.name == "Base shelf 2")
            .unwrap()
            .feature;
        s.commit(prepared.prepared).unwrap();

        // Opening a shelf opens the carcass; fewer shelves remove one Part.
        let mut edit = CabinetPanel::existing(s.document(), shelf_feature).unwrap();
        assert_eq!(edit.kind, Kind::Carcass);
        assert_eq!(edit.shelf_count, 2);
        edit.shelf_count = 1;
        let prepared = edit
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(prepared.outputs.len(), 6);
        // Malformed offsets are reported, not guessed.
        edit.offsets = true;
        edit.shelf_offsets = "100, four".into();
        assert!(
            edit.request(s.document(), true)
                .unwrap_err()
                .contains("four")
        );
    }

    #[test]
    fn form_places_a_banded_panel_and_edits_it_in_place() {
        let mut s = session();
        let mut form = CabinetPanel::new(s.document(), "Shelf board".into());
        form.kind = Kind::Panel;
        form.edges = [false, false, true, false];
        let command = form.request(s.document(), true).unwrap();
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        let feature = prepared.outputs[0].feature;
        let list = Compiler::default()
            .compile(prepared.prepared.document(), 1)
            .unwrap()
            .cut_list(prepared.prepared.document())
            .unwrap();
        assert_eq!(list[0].cut.core_mm, [800., 398.]);
        s.commit(prepared.prepared).unwrap();
        let mut edit = CabinetPanel::existing(s.document(), feature).unwrap();
        assert_eq!(edit.kind, Kind::Panel);
        assert_eq!(edit.edges, [false, false, true, false]);
        edit.length = field(900., "mm");
        let command = edit.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), Some(feature));
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(prepared.outputs[0].feature, feature);
    }

    fn ui() -> Actor {
        Actor::local(Channel::Ui)
    }
    /// The reported fixture: 600 × 720 × 560 mm, 18 mm stock, inset back and
    /// one shelf, committed as "Cabinet 1".
    fn cabinet_fixture() -> (Session, EntityId, EntityId) {
        let mut s = session();
        let form = CabinetPanel::new(s.document(), "Cabinet 1".into());
        assert_eq!(form.back, BackKind::Inset);
        let CabinetCommand::Carcass { request, .. } = form.request(s.document(), true).unwrap()
        else {
            panic!("carcass")
        };
        let carcass = request.carcass;
        let prepared = model::prepare_carcass(&s, s.revision(), ui(), &request).unwrap();
        Compiler::default()
            .compile(prepared.model.prepared.document(), 1)
            .unwrap();
        s.commit(prepared.model.prepared).unwrap();
        let group = model::read_carcass(s.document(), carcass)
            .unwrap()
            .group
            .unwrap();
        (s, carcass, group)
    }
    fn drill(s: &mut Session, name: &str, target: EntityId) -> EntityId {
        let p = model::prepare_machining(
            s,
            s.revision(),
            ui(),
            name,
            &model::MachiningDefinition {
                generated_by: None,
                kind: model::MachiningKind::Drill {
                    holes: vec![[length(100.), length(100.)]],
                    diameter: length(5.),
                    depth: Some(length(10.)),
                },
                face: model::MachiningFace::Top,
                targets: vec![target],
                suppressed: false,
            },
        )
        .unwrap();
        Compiler::default()
            .compile(p.model.prepared.document(), 1)
            .unwrap();
        s.commit(p.model.prepared).unwrap();
        p.operation
    }

    #[test]
    fn selecting_the_cabinet_group_reopens_its_saved_definition() {
        let (s, carcass, group) = cabinet_fixture();
        let d = s.document();
        let panels = model::carcass_panels(d, carcass).unwrap();
        // Group, carcass, any panel Part and any panel feature resolve.
        let target = cabinet_edit::carcass_target(d, group).expect("group opens the cabinet");
        assert_eq!(target.carcass, carcass);
        assert_eq!(target.name, "Cabinet 1");
        for id in [carcass, panels[2].1, panels[3].2] {
            assert_eq!(
                cabinet_edit::carcass_target(d, id).unwrap().carcass,
                carcass
            );
        }
        let form = CabinetPanel::existing(d, target.feature).unwrap();
        assert_eq!(form.kind, Kind::Carcass);
        assert_eq!(form.existing, Some(carcass));
        assert_eq!(form.name, "Cabinet 1");
        let value = |f: &DimensionField| match f.resolve(d).unwrap() {
            Dimension::Literal { value, .. } => value,
            _ => panic!("literal"),
        };
        assert_eq!(
            [value(&form.width), value(&form.height), value(&form.depth)],
            [600., 720., 560.]
        );
        assert_eq!(form.back, BackKind::Inset);
        assert_eq!(value(&form.recess), 10.);
        assert!(!form.offsets);
        assert_eq!(form.shelf_count, 1);
        // A rectangle panel or unrelated entity is not a cabinet.
        let mut rect = CabinetPanel::new(d, "Loose".into());
        rect.kind = Kind::Panel;
        let p = rect.request(d, true).unwrap().prepare(&s, ui()).unwrap();
        let loose = p.outputs[0].body;
        assert!(cabinet_edit::carcass_target(p.prepared.document(), loose).is_none());
        assert!(cabinet_edit::carcass_target(d, d.id()).is_none());
    }

    #[test]
    fn width_and_shelf_edit_applies_once_keeps_identities_and_drilling() {
        let (mut s, carcass, group) = cabinet_fixture();
        let before_panels = model::carcass_panels(s.document(), carcass).unwrap();
        let left = before_panels
            .iter()
            .find(|(r, ..)| *r == model::PanelRole::LeftSide)
            .unwrap()
            .2;
        let holes = drill(&mut s, "Hinge holes", left);
        let before = s.document().clone();
        let revision = s.revision();
        let target = cabinet_edit::carcass_target(s.document(), group).unwrap();
        let mut form = CabinetPanel::existing(s.document(), target.feature).unwrap();
        form.width = field(800., "mm");
        form.shelf_count = 2;
        form.stamp.changed();
        // Visual dimensions follow the draft before Preview.
        let guide = form.guide(s.document(), revision).unwrap().clone();
        let labels: Vec<_> = guide.dimensions.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, ["Width 800 mm", "Height 720 mm", "Depth 560 mm"]);
        assert_eq!(guide.shelves.len(), 2);
        assert_eq!(guide.edges.len(), 12);
        let changes = cabinet_edit::role_changes(
            s.document(),
            carcass,
            &match form.request(s.document(), true).unwrap() {
                CabinetCommand::Carcass { request, .. } => request.definition,
                _ => unreachable!(),
            },
        );
        assert_eq!(changes.kept, 6, "sides, top, bottom, back, shelf 1");
        assert_eq!(changes.added, ["shelf 2"]);
        assert!(changes.removed.is_empty());
        let command = form.request(s.document(), true).unwrap();
        assert!(
            matches!(command, CabinetCommand::Carcass { existing: Some(id), .. } if id == carcass)
        );
        let prepared = command.prepare(&s, ui()).unwrap();
        let model = Compiler::default()
            .compile(prepared.prepared.document(), 1)
            .unwrap();
        let list = model.cut_list(prepared.prepared.document()).unwrap();
        let top = list.iter().find(|r| r.name == "Cabinet 1 top").unwrap();
        assert_eq!(top.cut.finished_mm[0], 764.);
        s.commit(prepared.prepared).unwrap();
        assert_eq!(s.revision(), revision + 1, "one undoable edit");
        let d = s.document();
        assert_eq!(model::carcasses(d).count(), 1, "no Cabinet 2");
        let after = model::carcass_panels(d, carcass).unwrap();
        for panel in &before_panels {
            assert!(after.contains(panel), "retained identity {panel:?}");
        }
        assert_eq!(after.len(), before_panels.len() + 1);
        let added = after.iter().find(|p| !before_panels.contains(p)).unwrap();
        assert!(d.hierarchy().members[&group].contains(&added.2));
        assert_eq!(model::machinings_of(d, left), vec![holes]);
        s.undo(s.revision(), ui()).unwrap();
        assert_eq!(s.document(), &before);
    }

    #[test]
    fn removing_a_drilled_shelf_is_named_and_refused_without_source_change() {
        let (mut s, carcass, group) = cabinet_fixture();
        let shelf = model::carcass_panels(s.document(), carcass)
            .unwrap()
            .into_iter()
            .find(|(r, ..)| matches!(r, model::PanelRole::Shelf { .. }))
            .unwrap()
            .2;
        drill(&mut s, "Shelf pins", shelf);
        let before = s.document().clone();
        let target = cabinet_edit::carcass_target(s.document(), group).unwrap();
        let mut form = CabinetPanel::existing(s.document(), target.feature).unwrap();
        form.shelf_count = 0;
        let CabinetCommand::Carcass { request, .. } = form.request(s.document(), true).unwrap()
        else {
            panic!("carcass")
        };
        let changes = cabinet_edit::role_changes(s.document(), carcass, &request.definition);
        assert!(changes.blocked());
        assert_eq!(
            changes.removed,
            [(
                "Cabinet 1 shelf 1".to_string(),
                Some("Shelf pins".to_string())
            )]
        );
        let error = form
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, ui())
            .unwrap_err();
        assert!(error.contains("Shelf pins"), "{error}");
        assert_eq!(s.document(), &before);
    }

    #[test]
    fn envelope_guide_paints_dimensions_and_rejects_invalid_drafts() {
        let (s, _, group) = cabinet_fixture();
        let target = cabinet_edit::carcass_target(s.document(), group).unwrap();
        let mut form = CabinetPanel::existing(s.document(), target.feature).unwrap();
        let guide = form.guide(s.document(), s.revision()).unwrap().clone();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        let mut camera = crate::camera::Camera::default();
        let points: Vec<[f32; 3]> = guide
            .edges
            .iter()
            .flatten()
            .map(|p| p.map(|v| v as f32))
            .collect();
        camera.fit(&points, rect);
        let ctx = egui::Context::default();
        let mut labels = 0;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            labels = guide.paint(ui.painter(), camera, rect);
        });
        output.textures_delta.clear();
        assert_eq!(labels, 4, "three envelope dimensions and one shelf height");
        // A draft the planner refuses has no guide, like Preview.
        form.width = field(20., "mm");
        form.stamp.changed();
        assert!(form.guide(s.document(), s.revision()).is_none());
    }

    fn inspector_frame(
        ctx: &egui::Context,
        form: &mut CabinetPanel,
        s: &Session,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320., 2600.),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                let _ = form.show(ui, s.document(), true, PreviewState::Idle);
            },
        )
    }
    fn inspector_click(ctx: &egui::Context, form: &mut CabinetPanel, s: &Session, label: &str) {
        let output = inspector_frame(ctx, form, s, vec![]);
        let at = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing inspector control {label}"));
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            inspector_frame(
                ctx,
                form,
                s,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        for _ in 0..3 {
            inspector_frame(ctx, form, s, vec![]).drop_without_applying_deltas();
        }
    }
    #[test]
    fn explicit_cabinet_inspector_add_remove_and_support_controls_keep_the_retained_row() {
        let s = session();
        let original = s.document().clone();
        let mut form = CabinetPanel::cabinet(s.document(), "Interactive controls".into());
        let ctx = egui::Context::default();
        inspector_click(&ctx, &mut form, &s, "Add shelf");
        let cfg = form.options.as_ref().unwrap().spec(s.document()).unwrap();
        assert_eq!(
            cfg.shelves.iter().map(|s| s.key).collect::<Vec<_>>(),
            [0, 1]
        );
        let retained = cfg.shelves[1].clone();
        inspector_click(&ctx, &mut form, &s, "Remove shelf");
        let cfg = form.options.as_ref().unwrap().spec(s.document()).unwrap();
        assert_eq!(cfg.shelves, [retained]);
        inspector_click(&ctx, &mut form, &s, "System 32 adjustable");
        let cfg = form.options.as_ref().unwrap().spec(s.document()).unwrap();
        assert_eq!(cfg.shelves[0].key, 1);
        assert!(matches!(
            cfg.shelves[0].support,
            model::ShelfSupport::System32 { .. }
        ));
        assert!(cfg.shelves[0].stock.is_none());
        let prepared = form
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, ui())
            .unwrap();
        Compiler::default()
            .compile(prepared.prepared.document(), s.revision() + 1)
            .unwrap();
        assert_eq!(
            s.document(),
            &original,
            "inspector changes and preview are source neutral"
        );
    }
    #[test]
    fn explicit_cabinet_inspector_reopens_owned_connections_and_guides_actual_datums() {
        let mut s = session();
        let mut form = CabinetPanel::cabinet(s.document(), "Cabinet controls".into());
        let mut cfg = form.options.as_ref().unwrap().spec(s.document()).unwrap();
        let thin = panel_stocks(s.document())
            .into_iter()
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap()
            .0;
        cfg.shelves.push(model::CabinetShelf {
            key: 3,
            underside: length(0.),
            depth: length(350.),
            stock: Some(thin),
            support: model::ShelfSupport::System32 { level: 3 },
        });
        form.options = Some(CabinetFields::of(&cfg).unwrap());
        let guide = form.guide(s.document(), s.revision()).unwrap();
        assert!(
            guide
                .dimensions
                .iter()
                .any(|d| d.label.contains("Toe clearance 100 mm"))
        );
        assert!(
            guide
                .dimensions
                .iter()
                .any(|d| d.label == "Floor-to-top 720 mm")
        );
        assert!(
            guide
                .notes
                .iter()
                .any(|(_, label)| label.contains("Local floor datum"))
        );
        assert!(
            guide
                .shelves
                .iter()
                .any(|s| s.adjustable == Some(false) && s.label.contains("underside 360 mm"))
        );
        assert!(
            guide
                .shelves
                .iter()
                .any(|s| s.adjustable == Some(true) && s.label.contains("System 32 L3"))
        );
        let CabinetCommand::Carcass { request, .. } = form.request(s.document(), true).unwrap()
        else {
            unreachable!()
        };
        let created = model::prepare_carcass(&s, s.revision(), ui(), &request).unwrap();
        let id = created.carcass;
        Compiler::default()
            .compile(created.model.prepared.document(), s.revision() + 1)
            .unwrap();
        s.commit(created.model.prepared).unwrap();
        let mut reopened =
            CabinetPanel::existing(s.document(), created.panels[0].1.feature).unwrap();
        assert_eq!(
            reopened
                .options
                .as_ref()
                .unwrap()
                .spec(s.document())
                .unwrap(),
            cfg
        );
        let mut remove = cfg.clone();
        remove.shelves.remove(0);
        reopened.options = Some(CabinetFields::of(&remove).unwrap());
        let CabinetCommand::Carcass { request, .. } = reopened.request(s.document(), true).unwrap()
        else {
            unreachable!()
        };
        let changes = cabinet_edit::role_changes(s.document(), id, &request.definition);
        assert!(!changes.blocked());
        assert_eq!(changes.removed.len(), 1);
        assert!(changes.removed[0].1.is_none());
        let original = s.document().clone();
        let owned_cut = model::panel_joint_cuts(s.document())
            .find(|e| {
                model::read_panel_joint(
                    s.document(),
                    model::read_panel_joint_cut(s.document(), e.id)
                        .unwrap()
                        .joint,
                )
                .unwrap()
                .generated_by
                    == Some(id)
            })
            .unwrap()
            .id;
        let mut controller = super::super::ModelToolController::default();
        controller.set_backend(Box::new(super::super::native_fixture::NativeBackend::new()));
        controller
            .begin_existing(&s, &Default::default(), owned_cut, Channel::Ui)
            .unwrap();
        assert_eq!(controller.label(), "Cabinet");
        assert_eq!(s.document(), &original);
    }
}
