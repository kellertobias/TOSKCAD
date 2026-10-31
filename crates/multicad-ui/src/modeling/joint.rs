//! Panel joint inspector: join panel pairs that overlap at a corner with a
//! butt (either panel through), miter or finger joint, or panels that meet
//! face to edge with loose tenons (Domino style). Every pair's cuts preview
//! and apply together; an applied joint reopens to change its settings.
#[cfg(test)]
mod placement_tests;
mod viewport;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{
        self, LooseTenonSpec, ModelPrepared, PanelJointDefinition, PanelJointKind, PanelJointPair,
        length,
    },
};

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub struct JointCommand {
    pub existing: Option<EntityId>,
    pub name: String,
    pub joint: PanelJointDefinition,
}
impl JointCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        match self.existing {
            Some(id) => {
                model::prepare_panel_joint_edit(session, revision, actor, id, &self.joint.kind)
            }
            None => model::prepare_panel_joint(session, revision, actor, &self.name, &self.joint),
        }
        .map(|p| p.model)
        .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ButtA,
    ButtB,
    Miter,
    Finger,
    LooseTenon,
}
/// Loose-tenon settings in the Inspector.
#[derive(Clone, Debug)]
struct TenonFields {
    thickness: DimensionField,
    width: DimensionField,
    length: DimensionField,
    count: u32,
    inset: DimensionField,
    offset: DimensionField,
    face_depth: DimensionField,
    edge_depth: DimensionField,
    face_play: DimensionField,
    edge_play: DimensionField,
    fit: DimensionField,
}
fn mm(v: f64) -> DimensionField {
    DimensionField::from_dimension(&length(v), "mm").expect("literal")
}
/// The example loose tenon: a Festool DOMINO 8 × 22 × 40 mm tenon cut with
/// the 8 mm DF 500 cutter, 15 mm into an 18 mm face (3 mm left behind) and
/// 28 mm into the mating end, tight in the end mortise and with 6 mm play
/// along the face mortise, two per joint 60 mm in from each end. These are
/// editable example values, not a tested machine compatibility claim.
impl Default for TenonFields {
    fn default() -> Self {
        Self {
            thickness: mm(8.),
            width: mm(22.),
            length: mm(40.),
            count: 2,
            inset: mm(60.),
            offset: mm(0.),
            face_depth: mm(15.),
            edge_depth: mm(28.),
            face_play: mm(6.),
            edge_play: mm(0.),
            fit: mm(0.),
        }
    }
}
impl TenonFields {
    fn of(spec: &LooseTenonSpec) -> Result<Self, String> {
        let f = |d| DimensionField::from_dimension(d, "mm");
        Ok(Self {
            thickness: f(&spec.thickness)?,
            width: f(&spec.width)?,
            length: f(&spec.length)?,
            count: spec.count,
            inset: f(&spec.inset)?,
            offset: match &spec.offset {
                Some(o) => f(o)?,
                None => mm(0.),
            },
            face_depth: f(&spec.face_depth)?,
            edge_depth: f(&spec.edge_depth)?,
            face_play: f(&spec.face_play)?,
            edge_play: f(&spec.edge_play)?,
            fit: f(&spec.fit)?,
        })
    }
    fn spec(&self, document: &Document) -> Result<LooseTenonSpec, String> {
        let r = |f: &DimensionField| f.resolve(document);
        let offset = r(&self.offset)?;
        Ok(LooseTenonSpec {
            thickness: r(&self.thickness)?,
            width: r(&self.width)?,
            length: r(&self.length)?,
            count: self.count,
            inset: r(&self.inset)?,
            offset: (self.offset.text.trim() != "0").then_some(offset),
            face_depth: r(&self.face_depth)?,
            edge_depth: r(&self.edge_depth)?,
            face_play: r(&self.face_play)?,
            edge_play: r(&self.edge_play)?,
            fit: r(&self.fit)?,
        })
    }
    fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.small("Tenon: a rounded slot shape, thickness = cutter diameter, width including the rounded ends.");
        changed |= self
            .thickness
            .show(ui, "Tenon thickness", "Cutter diameter");
        changed |= self.width.show(
            ui,
            "Tenon width",
            "Along the joint, including the rounded ends",
        );
        changed |= self.length.show(ui, "Tenon length", "Mortise to mortise");
        ui.horizontal(|ui| {
            ui.label("Tenons per pair");
            changed |= ui
                .add(egui::DragValue::new(&mut self.count).range(1..=model::MAX_TENONS))
                .changed();
        });
        changed |= self.inset.show(
            ui,
            "End inset",
            "From each end of the contact to the first and last tenon center",
        );
        changed |= self.offset.show(
            ui,
            "Offset",
            "Tenon center across the edge panel's thickness, from its middle",
        );
        changed |= self.face_depth.show(
            ui,
            "Face mortise depth",
            "Into the panel whose face receives the joint",
        );
        changed |= self.edge_depth.show(
            ui,
            "End mortise depth",
            "Into the end of the panel whose edge meets the face",
        );
        changed |= self.face_play.show(
            ui,
            "Face mortise play",
            "Extra length beyond the tenon width (0 = tight)",
        );
        changed |= self.edge_play.show(
            ui,
            "End mortise play",
            "Extra length beyond the tenon width (0 = tight)",
        );
        changed |= self.fit.show(
            ui,
            "Thickness fit",
            "Extra mortise thickness beyond the tenon thickness",
        );
        changed
    }
}

pub(super) struct JointPanel {
    stamp: FormStamp,
    existing: Option<EntityId>,
    name: String,
    kind: Kind,
    count: u32,
    fit: DimensionField,
    tenon: TenonFields,
    pairs: Vec<[Option<EntityId>; 2]>,
    /// Pick pair panels by clicking their faces in the viewport.
    place: bool,
    pick_note: Option<String>,
    error: Option<String>,
}
/// Rectangular panel Parts: (Part, name).
fn panel_parts(document: &Document) -> Vec<(EntityId, String)> {
    model::panels(document)
        .filter_map(|e| {
            let panel = model::read_panel(document, e.id).ok()?;
            if matches!(panel.source, model::PanelSource::Profile { .. }) {
                return None;
            }
            let part = panel.output.body;
            Some((part, document.get(part)?.name.clone()))
        })
        .collect()
}
impl JointPanel {
    pub fn new(name: String) -> Self {
        Self {
            stamp: FormStamp::new(),
            existing: None,
            name,
            kind: Kind::ButtA,
            count: 5,
            fit: DimensionField::from_dimension(&length(0.2), "mm").expect("literal"),
            tenon: TenonFields::default(),
            pairs: vec![[None, None]],
            place: true,
            pick_note: None,
            error: None,
        }
    }
    /// A new loose-tenon joint with the example Domino settings.
    pub fn loose_tenon(name: String) -> Self {
        let mut form = Self::new(name);
        form.kind = Kind::LooseTenon;
        form
    }
    /// Every chosen panel of every pair, in order.
    pub fn pair_parts(&self) -> Vec<EntityId> {
        self.pairs.iter().flatten().flatten().copied().collect()
    }
    /// Seed the first pair, for example from two selected panels.
    pub fn with_pair(mut self, a: Option<EntityId>, b: Option<EntityId>) -> Self {
        if self.existing.is_none() {
            self.pairs = vec![[a, b]];
        }
        self
    }
    /// Open the joint that owns an applied joint cut or loose tenon.
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let joint_id = match model::read_panel_joint_cut(document, feature) {
            Ok(cut) => cut.joint,
            Err(error) => model::read_loose_tenon(document, feature)
                .map(|t| t.joint)
                .map_err(|_| error.to_string())?,
        };
        let joint = model::read_panel_joint(document, joint_id).map_err(|e| e.to_string())?;
        let mut form = Self::new(
            document
                .get(joint_id)
                .map(|e| e.name.clone())
                .unwrap_or_default(),
        );
        form.existing = Some(joint_id);
        form.pairs = joint.pairs.iter().map(|p| [Some(p.a), Some(p.b)]).collect();
        form.kind = match &joint.kind {
            PanelJointKind::ButtAThrough => Kind::ButtA,
            PanelJointKind::ButtBThrough => Kind::ButtB,
            PanelJointKind::Miter => Kind::Miter,
            PanelJointKind::Finger { count, fit } => {
                form.count = *count;
                form.fit = DimensionField::from_dimension(fit, "mm")?;
                Kind::Finger
            }
            PanelJointKind::LooseTenon(spec) => {
                form.tenon = TenonFields::of(spec)?;
                Kind::LooseTenon
            }
            PanelJointKind::Dowels(_) => {
                return Err("Edit dowel connections through the Cabinet inspector".into());
            }
        };
        Ok(form)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        if self.kind == Kind::LooseTenon {
            "Loose tenon (Domino)"
        } else {
            "Panel joint"
        }
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<JointCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a joint name with 1–1000 bytes".into());
        }
        let pairs = self
            .pairs
            .iter()
            .enumerate()
            .map(|(i, [a, b])| match (a, b) {
                (Some(a), Some(b)) => Ok(PanelJointPair { a: *a, b: *b }),
                _ => Err(format!("Choose both panels of pair {}", i + 1)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let kind = match self.kind {
            Kind::ButtA => PanelJointKind::ButtAThrough,
            Kind::ButtB => PanelJointKind::ButtBThrough,
            Kind::Miter => PanelJointKind::Miter,
            Kind::Finger => PanelJointKind::Finger {
                count: self.count,
                fit: self.fit.resolve(document)?,
            },
            Kind::LooseTenon => PanelJointKind::LooseTenon(Box::new(self.tenon.spec(document)?)),
        };
        Ok(JointCommand {
            existing: self.existing,
            name: self.name.clone(),
            joint: PanelJointDefinition { kind, pairs },
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
        ui.strong(self.label());
        ui.small(if self.kind == Kind::LooseTenon {
            "One panel's edge must lie flat on the other panel's face. Each tenon gets a rounded mortise in both panels and the tenons become their own Part."
        } else {
            "Panels must overlap at a right-angle corner: extend both to the outside corner. Each panel keeps its share of the corner block."
        });
        ui.checkbox(&mut self.place, "Pick pairs in the viewport")
            .on_hover_text("Click panel A, then panel B, on their visible faces; the next click starts the next pair");
        if let Some(note) = &self.pick_note {
            ui.colored_label(egui::Color32::LIGHT_RED, note);
        }
        ui.label("Joint name");
        let mut changed = ui
            .add_enabled(
                self.existing.is_none(),
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        ui.label("Kind");
        let loose = self.existing.is_some() && self.kind == Kind::LooseTenon;
        let corner = self.existing.is_some() && self.kind != Kind::LooseTenon;
        for (kind, label) in [
            (Kind::ButtA, "Butt · A runs through"),
            (Kind::ButtB, "Butt · B runs through"),
            (Kind::Miter, "Miter"),
            (Kind::Finger, "Fingers"),
            (Kind::LooseTenon, "Loose tenon (Domino)"),
        ] {
            let allowed = if kind == Kind::LooseTenon {
                !corner
            } else {
                !loose
            };
            if ui
                .add_enabled(allowed, egui::RadioButton::new(self.kind == kind, label))
                .clicked()
                && self.kind != kind
            {
                self.kind = kind;
                changed = true;
            }
        }
        if self.kind == Kind::LooseTenon {
            changed |= self.tenon.show(ui);
        }
        if self.kind == Kind::Finger {
            ui.horizontal(|ui| {
                ui.label("Fingers");
                changed |= ui
                    .add(egui::DragValue::new(&mut self.count).range(2..=model::MAX_FINGERS))
                    .changed();
            });
            changed |= self.fit.show(ui, "Fit", "Total gap at each finger face");
        }
        ui.separator();
        ui.strong("Panel pairs");
        let parts = panel_parts(document);
        let name_of = |id: Option<EntityId>| {
            id.and_then(|id| parts.iter().find(|(p, _)| *p == id).map(|(_, n)| n.clone()))
                .unwrap_or_else(|| "Choose a panel".into())
        };
        let mut remove = None;
        let editable = self.existing.is_none();
        for (i, pair) in self.pairs.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                for (k, label) in ["A", "B"].into_iter().enumerate() {
                    ui.add_enabled_ui(editable, |ui| {
                        controls::inspector_combo(ui, label)
                            .selected_text(format!("{} {label}: {}", i + 1, name_of(pair[k])))
                            .show_ui(ui, |ui| {
                                for (id, name) in &parts {
                                    changed |= ui
                                        .selectable_value(&mut pair[k], Some(*id), name)
                                        .changed();
                                }
                            });
                    });
                }
                if editable && ui.small_button("Remove pair").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.pairs.remove(i);
            changed = true;
        }
        if editable && ui.button("Add pair").clicked() {
            self.pairs.push([None, None]);
            changed = true;
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
                    request: ModelRequest::Joint(Box::new(request)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel},
        document::{Entity, EntityData},
        materials::{Grain, LibraryDefinition, Stock, StockKind, prepare_library},
        model::{
            BodyOutput, GrainAxis, PanelDefinition, PanelEdges, PanelPlane, PanelRequest,
            PanelSource,
        },
    };
    use multicad_model::Compiler;

    #[test]
    fn form_joins_a_corner_and_reopens_to_change_the_kind() {
        let actor = || Actor::local(Channel::Ui);
        let mut s = Session::new(Document::new("Joint form").unwrap()).unwrap();
        let birch = Entity::new(
            "Birch",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [210, 180, 140, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: birch.id,
            specification: StockKind::Panel {
                thickness_mm: 18.,
                length_mm: 2500.,
                width_mm: 1250.,
                grain: Grain::None,
            },
        })
        .entity(EntityId::new(), "Ply 18")
        .unwrap();
        let stock_id = stock.id;
        s.commit(prepare_library(&s, 0, actor(), &[birch, stock]).unwrap())
            .unwrap();
        let mut parts = vec![];
        for (name, plane, l, w) in [
            ("Side", PanelPlane::Yz, 400., 300.),
            ("Bottom", PanelPlane::Xy, 600., 400.),
        ] {
            let request = PanelRequest::new(
                name,
                PanelDefinition {
                    output: BodyOutput {
                        body: EntityId::new(),
                    },
                    source: PanelSource::Rectangle {
                        stock: stock_id,
                        plane,
                        origin: [length(0.), length(0.), length(0.)],
                        length: length(l),
                        width: length(w),
                    },
                    grain: GrainAxis::U,
                    edges: PanelEdges::default(),
                    oversize: None,
                },
            );
            parts.push(request.body);
            let p = model::prepare_panel(&s, s.revision(), actor(), &request).unwrap();
            s.commit(p.prepared).unwrap();
        }
        let mut form = JointPanel::new("Corner".into());
        assert!(
            form.request(s.document(), true)
                .unwrap_err()
                .contains("pair 1")
        );
        assert_eq!(panel_parts(s.document()).len(), 2);
        form.pairs = vec![[Some(parts[0]), Some(parts[1])]];
        form.kind = Kind::Miter;
        let prepared = form
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, actor())
            .unwrap();
        assert_eq!(prepared.outputs.len(), 2);
        let cut = prepared.outputs[0].feature;
        let model = Compiler::default()
            .compile(prepared.prepared.document(), 3)
            .unwrap();
        let head = model.snapshot().bodies[&parts[1]];
        let volume = model.solid(head).unwrap().volume_mm3().unwrap();
        assert!((volume - (600. * 400. * 18. - 18. * 18. * 400. / 2.)).abs() < 1e-3);
        s.commit(prepared.prepared).unwrap();
        let mut edit = JointPanel::existing(s.document(), cut).unwrap();
        assert_eq!(edit.kind, Kind::Miter);
        edit.kind = Kind::Finger;
        edit.count = 3;
        let prepared = edit
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, actor())
            .unwrap();
        assert_eq!(prepared.outputs.len(), 2);
    }
}
