//! Sheet-metal inspector: place a rectangular base sheet from sheet stock,
//! or edit an applied sheet part. Thickness comes from the stock; the inside
//! radius and K-factor come from the stock unless overridden here. Sheet
//! stock is defined in Project → Materials.
mod edges;

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
        self, BendDirection, BodyOutput, Cutout, Flange, FlangeKind, FlangeReview, ModelPrepared,
        MountingPattern, PanelPlane, SheetBase, SheetDefinition, SheetRequest,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Bend,
    Hem,
    Jog,
}
/// One rectangle edge's flange in the form: off, or a bend, hem or jog with
/// its values and an equal corner inset at both ends.
struct FlangeRow {
    on: bool,
    kind: RowKind,
    angle: DimensionField,
    length: DimensionField,
    gap: DimensionField,
    offset: DimensionField,
    down: bool,
    inset: DimensionField,
}
impl FlangeRow {
    fn new() -> Self {
        Self {
            on: false,
            kind: RowKind::Bend,
            angle: field(90., "deg"),
            length: field(20., "mm"),
            gap: field(1., "mm"),
            offset: field(10., "mm"),
            down: false,
            inset: field(0., "mm"),
        }
    }
    /// The row for an applied flange, when the form can show it.
    fn from_flange(f: &Flange) -> Result<Option<Self>, String> {
        if f.edge > 3
            || f.relief.is_some()
            || f.inside_radius.is_some()
            || f.start_inset != f.end_inset
        {
            return Ok(None);
        }
        let mut row = Self::new();
        row.on = true;
        row.length = of(&f.length, "mm")?;
        row.down = f.direction == BendDirection::Down;
        if let Some(inset) = &f.start_inset {
            row.inset = of(inset, "mm")?;
        }
        match &f.kind {
            FlangeKind::Bend { angle } => row.angle = of(angle, "deg")?,
            FlangeKind::Hem { gap } => {
                row.kind = RowKind::Hem;
                row.gap = of(gap, "mm")?;
            }
            FlangeKind::Jog { angle, offset } => {
                row.kind = RowKind::Jog;
                row.angle = of(angle, "deg")?;
                row.offset = of(offset, "mm")?;
            }
        }
        Ok(Some(row))
    }
    fn flange(&self, edge: u32, document: &Document) -> Result<Flange, String> {
        let inset = self.inset.resolve(document)?;
        let zero = matches!(inset, Dimension::Literal { value, .. } if value == 0.);
        Ok(Flange {
            edge,
            kind: match self.kind {
                RowKind::Bend => FlangeKind::Bend {
                    angle: self.angle.resolve(document)?,
                },
                RowKind::Hem => FlangeKind::Hem {
                    gap: self.gap.resolve(document)?,
                },
                RowKind::Jog => FlangeKind::Jog {
                    angle: self.angle.resolve(document)?,
                    offset: self.offset.resolve(document)?,
                },
            },
            direction: if self.down {
                BendDirection::Down
            } else {
                BendDirection::Up
            },
            length: self.length.resolve(document)?,
            inside_radius: None,
            start_inset: (!zero).then(|| inset.clone()),
            end_inset: (!zero).then_some(inset),
            relief: None,
        })
    }
    /// One line describing the flange, for rows that are not expanded.
    fn summary(&self) -> String {
        let text = |f: &DimensionField| f.text.trim().to_owned();
        let side = if self.down { "down" } else { "up" };
        let shape = match self.kind {
            RowKind::Bend => format!("Bend {}° {side}", text(&self.angle)),
            RowKind::Hem => format!("Hem {side} · gap {} mm", text(&self.gap)),
            RowKind::Jog => format!(
                "Jog {}° {side} · offset {} mm",
                text(&self.angle),
                text(&self.offset)
            ),
        };
        format!("{shape} · leg {} mm", text(&self.length))
    }
    fn show_fields(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            for (kind, text) in [
                (RowKind::Bend, "Bend"),
                (RowKind::Hem, "Hem"),
                (RowKind::Jog, "Jog"),
            ] {
                changed |= ui.radio_value(&mut self.kind, kind, text).changed();
            }
            changed |= ui.checkbox(&mut self.down, "Down").changed();
        });
        match self.kind {
            RowKind::Bend => {
                changed |= self
                    .angle
                    .show(ui, "Angle", "Bend angle, above 0° and at most 180°");
            }
            RowKind::Hem => {
                changed |= self.gap.show(
                    ui,
                    "Hem gap",
                    "Gap between the folded leg and the base face",
                );
            }
            RowKind::Jog => {
                changed |= self
                    .angle
                    .show(ui, "Jog angle", "Angle of both jog bends, at most 90°");
                changed |= self.offset.show(
                    ui,
                    "Jog offset",
                    "Distance between the base face and the leg's matching face",
                );
            }
        }
        changed |= self
            .length
            .show(ui, "Leg length", "Straight leg beyond the last bend");
        changed |= self.inset.show(
            ui,
            "Corner inset",
            "Gap at both ends of the edge, so flanges on neighboring edges do not touch",
        );
        changed
    }
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
    /// Flanges and cutouts kept unchanged by this form.
    kept_forming: (Vec<Flange>, Vec<Cutout>),
    /// Mounting patterns kept unchanged by this form.
    kept_fasteners: Vec<MountingPattern>,
    /// Flanges on the four rectangle edges, when the form can edit them.
    rows: Option<[FlangeRow; 4]>,
    /// The edge whose flange is expanded in the Inspector and dimensioned
    /// in the viewer.
    selected: Option<usize>,
    /// The viewer target under the pointer.
    hover: Option<edges::Target>,
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
            kept_forming: (vec![], vec![]),
            kept_fasteners: vec![],
            rows: Some(std::array::from_fn(|_| FlangeRow::new())),
            selected: None,
            hover: None,
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
        form.kept_forming = (sheet.flanges.clone(), sheet.cutouts.clone());
        form.kept_fasteners = sheet.fasteners.clone();
        form.rows = None;
        if form.kept_base.is_none() {
            let mut rows: [FlangeRow; 4] = std::array::from_fn(|_| FlangeRow::new());
            let mut shown = true;
            for f in &sheet.flanges {
                match FlangeRow::from_flange(f)? {
                    Some(row) => rows[f.edge as usize] = row,
                    None => shown = false,
                }
            }
            if shown {
                form.selected = rows.iter().position(|r| r.on);
                form.rows = Some(rows);
                form.kept_forming.0.clear();
            }
        }
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
        "Sheet part"
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<SheetCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        Ok(SheetCommand {
            editing: self.existing.is_some(),
            feature: self.feature,
            name: self.name.clone(),
            sheet: self.definition(document)?,
        })
    }
    /// The sheet definition the form describes, shared by the Inspector
    /// and the viewer.
    fn definition(&self, document: &Document) -> Result<SheetDefinition, String> {
        let stock = self
            .stock
            .ok_or("Choose sheet stock; define it in Project → Materials")?;
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
        Ok(SheetDefinition {
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
            flanges: match &self.rows {
                Some(rows) => rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.on)
                    .map(|(edge, r)| r.flange(edge as u32, document))
                    .collect::<Result<_, _>>()?,
                None => self.kept_forming.0.clone(),
            },
            cutouts: self.kept_forming.1.clone(),
            fasteners: self.kept_fasteners.clone(),
        })
    }
    /// Outline, flange guides and pre-check findings of the current inputs;
    /// `None` while the stock or base does not resolve.
    pub fn review(&self, document: &Document) -> Option<FlangeReview> {
        let sheet = self.definition(document).ok()?;
        let parameters = multicad_core::parameters::evaluate(document).ok()?;
        model::review_flanges(document, &parameters, &sheet).ok()
    }
    /// Whether the viewer offers edge picking: the form edits the flanges
    /// of a rectangle base.
    pub fn edge_picking(&self) -> bool {
        self.rows.is_some()
    }
    fn edited(&mut self) {
        self.stamp.changed();
        self.error = None;
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
                "No sheet stock yet. Define sheet stock in Project → Materials.",
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
        ui.strong("Flanges");
        if self.rows.is_some() {
            ui.small(
                "Click a base edge here or in the viewport to add or select its flange; the \
                 viewport shows its bend, leg and direction.",
            );
            let review = self.review(document);
            if let Some(edge) = self.diagram(ui, review.as_ref()) {
                changed |= self.pick_edge(edge);
            }
            let selected = self.selected;
            let mut select = None;
            if let Some(rows) = &mut self.rows {
                for (edge, (row, label)) in rows.iter_mut().zip(edges::EDGE_TEXT).enumerate() {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut row.on, "").changed() {
                            changed = true;
                            select = Some(edge);
                        }
                        if ui
                            .selectable_label(selected == Some(edge), label)
                            .on_hover_text("Show this flange's values and dimensions")
                            .clicked()
                        {
                            select = Some(edge);
                        }
                    });
                    if !row.on {
                        continue;
                    }
                    if selected == Some(edge) {
                        ui.indent(("sheet-flange", edge), |ui| {
                            changed |= row.show_fields(ui);
                        });
                    } else {
                        ui.small(format!("    {}", row.summary()));
                    }
                }
            }
            if select.is_some() {
                self.selected = select;
            }
            if let Some(review) = &review {
                if let Some(g) = self
                    .selected
                    .and_then(|e| review.guides.iter().find(|g| g.edge == e))
                {
                    ui.small(format!(
                        "Edge {} as built: {} · {} · t {} mm",
                        g.edge + 1,
                        edges::bend_text(g),
                        edges::leg_text(g),
                        review.thickness_mm
                    ));
                }
                for issue in &review.issues {
                    ui.colored_label(egui::Color32::LIGHT_RED, &issue.message);
                }
            }
        } else {
            ui.small(format!(
                "{} flange(s) are kept as they are; edit them through MCP set_sheet_forming.",
                self.kept_forming.0.len()
            ));
        }
        if !self.kept_forming.1.is_empty() {
            ui.small(format!(
                "{} cutout(s) are kept as they are.",
                self.kept_forming.1.len()
            ));
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
        // Flanges on the long edges; reopening shows them in their rows.
        edit.rows.as_mut().unwrap()[0].on = true;
        let hem = &mut edit.rows.as_mut().unwrap()[2];
        hem.on = true;
        hem.kind = RowKind::Hem;
        let command = edit.request(s.document(), true).unwrap();
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        let records = Compiler::default()
            .compile(prepared.prepared.document(), 2)
            .unwrap()
            .sheet_metal(prepared.prepared.document())
            .unwrap();
        assert_eq!(records[0].bends.len(), 2);
        assert_eq!(records[0].bends[1].angle_deg, 180.);
        s.commit(prepared.prepared).unwrap();
        let mut edit = SheetPanel::existing(s.document(), feature).unwrap();
        let rows = edit.rows.as_ref().unwrap();
        assert!(rows[0].on && rows[2].on && !rows[1].on);
        assert_eq!(rows[2].kind, RowKind::Hem);
        // Reopening selects the first applied flange and guides both edges.
        assert_eq!(edit.selected, Some(0));
        let review = edit.review(s.document()).unwrap();
        let edges: Vec<_> = review.guides.iter().map(|g| g.edge).collect();
        assert_eq!(edges, vec![0, 2]);
        assert_eq!(review.guides[1].kind, "hem");
        // A K-factor outside 0–1 is refused before any geometry exists.
        edit.k_override = true;
        edit.k_factor = field(2., "ratio");
        let command = edit.request(s.document(), true).unwrap();
        assert!(command.prepare(&s, Actor::local(Channel::Ui)).is_err());
    }

    const RECT: egui::Rect = egui::Rect {
        min: egui::Pos2::ZERO,
        max: egui::Pos2::new(800., 600.),
    };
    fn camera() -> crate::camera::Camera {
        crate::camera::Camera {
            yaw: 0.5,
            pitch: 0.6,
            scale: 2.5,
            target: [100., 50., 0.],
            perspective: false,
        }
    }
    fn screen(p: [f64; 3]) -> egui::Pos2 {
        let s = camera().project(p.map(|v| v as f32), RECT);
        egui::pos2(s[0], s[1])
    }
    /// One viewer frame with the pointer at `pointer`; returns whether the
    /// definition changed and every text the overlay painted.
    fn frame(
        form: &mut SheetPanel,
        s: &Session,
        pointer: egui::Pos2,
        clicked: bool,
    ) -> (bool, Vec<String>) {
        let ctx = egui::Context::default();
        let mut changed = false;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(RECT),
                ..Default::default()
            },
            |ui| {
                changed = form.viewport(
                    s.document(),
                    crate::modeling::TerminationViewport {
                        scene: None,
                        report: None,
                        camera: camera(),
                        rect: RECT,
                        pointer: Some(pointer),
                        clicked,
                        painter: ui.painter(),
                        section: None,
                    },
                );
            },
        );
        output.textures_delta.clear();
        let texts = output
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        (changed, texts)
    }
    fn compile_error(s: &Session, command: &SheetCommand) -> Option<String> {
        let prepared = command.prepare(s, Actor::local(Channel::Ui)).unwrap();
        Compiler::default()
            .compile(prepared.prepared.document(), s.revision() + 1)
            .err()
            .map(|e| e.message)
    }

    #[test]
    fn viewer_edge_clicks_and_inspector_rows_build_the_same_checked_sheet() {
        let s = session();
        let mut form = SheetPanel::new(s.document(), "Front panel".into());
        assert!(form.edge_picking());
        // Hovering identifies edge 1 (v = 0) without changing anything.
        let edge1 = screen([100., 0., 0.]);
        let (changed, texts) = frame(&mut form, &s, edge1, false);
        assert!(!changed);
        assert_eq!(form.hover, Some(edges::Target::Edge(0)));
        assert!(!form.rows.as_ref().unwrap()[0].on);
        for label in ["1", "2", "3", "4"] {
            assert!(texts.iter().any(|t| t == label), "{texts:?}");
        }
        let caption = texts.iter().find(|t| t.starts_with("Sheet edges")).unwrap();
        assert!(
            caption.contains("t 2 mm (stock) · R 2 mm (stock) · K 0.4 (stock)"),
            "{caption}"
        );
        // Clicking it adds a 90° flange and shows its bend and leg.
        let (changed, texts) = frame(&mut form, &s, edge1, true);
        assert!(changed);
        assert_eq!(form.selected, Some(0));
        assert!(form.rows.as_ref().unwrap()[0].on);
        assert!(texts.iter().any(|t| t == "Leg 20 mm"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "90° up · R 2 mm"), "{texts:?}");
        // Clicking the selected edge again keeps it.
        assert!(!frame(&mut form, &s, edge1, true).0);
        // The handle at the leg end flips the flange down.
        let review = form.review(s.document()).unwrap();
        let outline = review.outline.as_ref().unwrap();
        let g = &review.guides[0];
        let handle = screen(g.world(outline, g.leg[1], g.width_mm() / 2.));
        let (changed, texts) = frame(&mut form, &s, handle, true);
        assert!(changed);
        assert!(form.rows.as_ref().unwrap()[0].down);
        assert!(texts.iter().any(|t| t == "90° down · R 2 mm"), "{texts:?}");

        // The same intent typed into the Inspector rows.
        let mut typed = SheetPanel::new(s.document(), "Front panel".into());
        let row = &mut typed.rows.as_mut().unwrap()[0];
        row.on = true;
        row.down = true;
        let visual = form.request(s.document(), true).unwrap().sheet;
        let inspector = typed.request(s.document(), true).unwrap().sheet;
        assert_eq!(visual.flanges, inspector.flanges);
        assert_eq!(visual.base, inspector.base);
        let command = form.request(s.document(), true).unwrap();
        assert_eq!(compile_error(&s, &command), None);
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        let records = Compiler::default()
            .compile(prepared.prepared.document(), 1)
            .unwrap()
            .sheet_metal(prepared.prepared.document())
            .unwrap();
        assert_eq!(records[0].bends.len(), 1);
        assert_eq!(records[0].bends[0].edge, 0);
        assert_eq!(records[0].bends[0].angle_deg, 90.);
    }

    #[test]
    fn neighboring_flanges_and_invalid_bends_are_diagnosed_on_their_edges() {
        let s = session();
        let mut form = SheetPanel::new(s.document(), "Tray".into());
        assert!(frame(&mut form, &s, screen([100., 0., 0.]), true).0);
        assert!(frame(&mut form, &s, screen([200., 50., 0.]), true).0);
        assert_eq!(form.selected, Some(1));
        let review = form.review(s.document()).unwrap();
        assert_eq!(review.issues.len(), 1);
        assert_eq!(review.issues[0].edges, vec![0, 1]);
        let (_, texts) = frame(&mut form, &s, egui::pos2(1., 1.), false);
        assert!(
            texts.iter().any(|t| t.ends_with("1 flange problem(s)")),
            "{texts:?}"
        );
        // Native evaluation refuses the same corner.
        let command = form.request(s.document(), true).unwrap();
        let native = compile_error(&s, &command).unwrap();
        assert!(native.contains("Flanges 1 and 2"), "{native}");
        // A corner inset clears both the pre-check and native evaluation.
        form.rows.as_mut().unwrap()[1].inset = field(5., "mm");
        assert_eq!(form.review(s.document()).unwrap().issues, vec![]);
        let command = form.request(s.document(), true).unwrap();
        assert_eq!(compile_error(&s, &command), None);
        // An invalid bend is named with its edge, and the edge has no guide.
        form.rows.as_mut().unwrap()[1].angle = field(190., "deg");
        let review = form.review(s.document()).unwrap();
        assert_eq!(review.issues.len(), 1);
        assert!(
            review.issues[0]
                .message
                .starts_with("Flange on edge 2: the bend angle"),
            "{}",
            review.issues[0].message
        );
        assert!(review.guides.iter().all(|g| g.edge != 1));
        // The Inspector shows the finding and the numbered edge list.
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360., 1600.),
                )),
                ..Default::default()
            },
            |ui| {
                form.show(ui, s.document(), true, PreviewState::default());
            },
        );
        output.textures_delta.clear();
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().replace('\n', " ")),
                _ => None,
            })
            .collect();
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with("Flange on edge 2: the bend angle")),
            "{texts:?}"
        );
        assert!(texts.iter().any(|t| t == "Edge 1 · along length at v = 0"));
        assert!(texts.iter().any(|t| t == "base face · u →"));
    }
}
