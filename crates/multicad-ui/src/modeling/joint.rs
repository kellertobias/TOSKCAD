//! Panel joint inspector: join panel pairs that overlap at a corner with a
//! butt (either panel through), miter or finger joint. Every pair's cuts
//! preview and apply together; an applied joint reopens to change its kind.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{self, ModelPrepared, PanelJointDefinition, PanelJointKind, PanelJointPair, length},
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
}

pub(super) struct JointPanel {
    stamp: FormStamp,
    existing: Option<EntityId>,
    name: String,
    kind: Kind,
    count: u32,
    fit: DimensionField,
    pairs: Vec<[Option<EntityId>; 2]>,
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
            pairs: vec![[None, None]],
            error: None,
        }
    }
    /// Open the joint that owns an applied joint cut.
    pub fn existing(document: &Document, cut: EntityId) -> Result<Self, String> {
        let cut = model::read_panel_joint_cut(document, cut).map_err(|e| e.to_string())?;
        let joint = model::read_panel_joint(document, cut.joint).map_err(|e| e.to_string())?;
        let mut form = Self::new(
            document
                .get(cut.joint)
                .map(|e| e.name.clone())
                .unwrap_or_default(),
        );
        form.existing = Some(cut.joint);
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
        };
        Ok(form)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Panel joint"
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
        ui.strong("Panel joint");
        ui.small(
            "Panels must overlap at a right-angle corner: extend both to the outside corner. Each panel keeps its share of the corner block.",
        );
        ui.label("Joint name");
        let mut changed = ui
            .add_enabled(
                self.existing.is_none(),
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        ui.label("Kind");
        for (kind, label) in [
            (Kind::ButtA, "Butt · A runs through"),
            (Kind::ButtB, "Butt · B runs through"),
            (Kind::Miter, "Miter"),
            (Kind::Finger, "Fingers"),
        ] {
            changed |= ui.radio_value(&mut self.kind, kind, label).changed();
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
