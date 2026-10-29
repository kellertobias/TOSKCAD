//! FDM aid inspector: add or edit a breakaway tab, thin bridge or removable
//! support as its own Part serving a finished Part. The finished Part is
//! never changed; a support's height is derived from it natively.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId},
    model::{
        self, AidDefinition, AidKind, AidRequest, AidShape, BodyOutput, ModelPrepared, angle,
        length,
    },
};

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub struct FdmAidCommand {
    /// The aid feature being edited, if any.
    pub existing: Option<EntityId>,
    pub request: AidRequest,
}
impl FdmAidCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        match self.existing {
            Some(feature) => {
                model::prepare_aid_edit(session, revision, actor, feature, &self.request.aid)
            }
            None => model::prepare_aid(session, revision, actor, &self.request),
        }
        .map_err(|e| e.to_string())
    }
    pub fn edit_id(&self) -> Option<EntityId> {
        self.existing
    }
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
fn xyz(d: &[Dimension; 3]) -> Result<[DimensionField; 3], String> {
    Ok([of(&d[0], "mm")?, of(&d[1], "mm")?, of(&d[2], "mm")?])
}
/// Finished Parts an aid can serve: every Part not produced by an aid.
fn finished_parts(document: &Document) -> Vec<(EntityId, String)> {
    let aids = model::aid_parts(document);
    document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Part { .. }) && !aids.contains(&e.id))
        .map(|e| (e.id, e.name.clone()))
        .collect()
}
fn processes(document: &Document) -> Vec<(EntityId, String)> {
    model::processes(document)
        .map(|e| (e.id, e.name.clone()))
        .collect()
}
fn combo(
    ui: &mut egui::Ui,
    id: &str,
    choices: &[(EntityId, String)],
    value: &mut Option<EntityId>,
    none: &str,
) -> bool {
    let selected = value
        .and_then(|v| choices.iter().find(|(c, _)| *c == v))
        .map_or(none.to_owned(), |(_, n)| n.clone());
    let mut changed = false;
    controls::inspector_combo(ui, id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            changed |= ui.selectable_value(value, None, none).changed();
            for (c, n) in choices {
                changed |= ui.selectable_value(value, Some(*c), n).changed();
            }
        });
    changed
}

pub(super) struct FdmAidPanel {
    stamp: FormStamp,
    kind: AidKind,
    existing: Option<EntityId>,
    feature: EntityId,
    body: EntityId,
    name: String,
    target: Option<EntityId>,
    process: Option<EntityId>,
    other: Option<EntityId>,
    origin: [DimensionField; 3],
    end: [DimensionField; 3],
    angle: DimensionField,
    length: DimensionField,
    width: DimensionField,
    thickness: DimensionField,
    overlap: DimensionField,
    base: DimensionField,
    gap: DimensionField,
    error: Option<String>,
}
impl FdmAidPanel {
    pub fn new(document: &Document, name: String) -> Self {
        Self {
            stamp: FormStamp::new(),
            kind: AidKind::Tab,
            existing: None,
            feature: EntityId::new(),
            body: EntityId::new(),
            name,
            target: finished_parts(document).first().map(|p| p.0),
            process: processes(document).first().map(|p| p.0),
            other: None,
            origin: [field(0., "mm"), field(0., "mm"), field(0., "mm")],
            end: [field(20., "mm"), field(0., "mm"), field(0., "mm")],
            angle: field(0., "deg"),
            length: field(8., "mm"),
            width: field(6., "mm"),
            thickness: field(0.4, "mm"),
            overlap: field(2., "mm"),
            base: field(0., "mm"),
            gap: field(0.2, "mm"),
            error: None,
        }
    }
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let aid = model::read_aid(document, feature).map_err(|e| e.to_string())?;
        let mut form = Self::new(document, String::new());
        form.existing = Some(feature);
        form.feature = feature;
        form.body = aid.output.body;
        form.name = document
            .get(aid.output.body)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        form.kind = aid.shape.kind();
        form.target = Some(aid.target);
        form.process = aid.process;
        match &aid.shape {
            AidShape::Tab {
                origin,
                angle,
                length,
                width,
                thickness,
                overlap,
            } => {
                form.origin = xyz(origin)?;
                form.angle = of(angle, "deg")?;
                form.length = of(length, "mm")?;
                form.width = of(width, "mm")?;
                form.thickness = of(thickness, "mm")?;
                form.overlap = of(overlap, "mm")?;
            }
            AidShape::Bridge {
                start,
                end,
                width,
                thickness,
                other,
            } => {
                form.origin = xyz(start)?;
                form.end = xyz(end)?;
                form.width = of(width, "mm")?;
                form.thickness = of(thickness, "mm")?;
                form.other = *other;
            }
            AidShape::Support {
                origin,
                angle,
                length,
                width,
                base,
                gap,
            } => {
                form.origin[0] = of(&origin[0], "mm")?;
                form.origin[1] = of(&origin[1], "mm")?;
                form.angle = of(angle, "deg")?;
                form.length = of(length, "mm")?;
                form.width = of(width, "mm")?;
                form.base = of(base, "mm")?;
                form.gap = of(gap, "mm")?;
            }
        }
        Ok(form)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        match self.kind {
            AidKind::Tab => "Breakaway tab",
            AidKind::Bridge => "Thin bridge",
            AidKind::Support => "Removable support",
        }
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<FdmAidCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let target = self
            .target
            .ok_or("Choose the finished Part the aid serves")?;
        let r = |f: &DimensionField| f.resolve(document);
        let point = |p: &[DimensionField; 3]| -> Result<[Dimension; 3], String> {
            Ok([r(&p[0])?, r(&p[1])?, r(&p[2])?])
        };
        let shape = match self.kind {
            AidKind::Tab => AidShape::Tab {
                origin: point(&self.origin)?,
                angle: r(&self.angle)?,
                length: r(&self.length)?,
                width: r(&self.width)?,
                thickness: r(&self.thickness)?,
                overlap: r(&self.overlap)?,
            },
            AidKind::Bridge => AidShape::Bridge {
                start: point(&self.origin)?,
                end: point(&self.end)?,
                width: r(&self.width)?,
                thickness: r(&self.thickness)?,
                other: self.other,
            },
            AidKind::Support => AidShape::Support {
                origin: [r(&self.origin[0])?, r(&self.origin[1])?],
                angle: r(&self.angle)?,
                length: r(&self.length)?,
                width: r(&self.width)?,
                base: r(&self.base)?,
                gap: r(&self.gap)?,
            },
        };
        let aid = AidDefinition {
            output: BodyOutput { body: self.body },
            target,
            shape,
            process: self.process,
        };
        aid.validate().map_err(|e| e.to_string())?;
        Ok(FdmAidCommand {
            existing: self.existing,
            request: AidRequest {
                feature: self.feature,
                body: self.body,
                name: self.name.clone(),
                aid,
                material: None,
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
        if self.existing.is_none() {
            ui.horizontal(|ui| {
                for (kind, label) in [
                    (AidKind::Tab, "Tab"),
                    (AidKind::Bridge, "Bridge"),
                    (AidKind::Support, "Support"),
                ] {
                    changed |= ui.radio_value(&mut self.kind, kind, label).changed();
                }
            });
        }
        ui.strong(self.label());
        ui.small(match self.kind {
            AidKind::Tab => "A flat pad from the contact point outward, overlapping under the part. Its own Part; export recipes can exclude it.",
            AidKind::Bridge => "A thin bar between two points, tying the part to itself or to a second Part. Its own Part; export recipes can exclude it.",
            AidKind::Support => "A flat-topped block from the base up to the gap below the lowest part surface over its footprint. The height follows the part.",
        });
        ui.small("+Z is the build direction; keep this orientation in the print recipe.");
        ui.label("Name");
        changed |= ui
            .add(egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()))
            .changed();
        let parts = finished_parts(document);
        ui.label("Finished Part");
        changed |= combo(
            ui,
            "fdm-aid-target",
            &parts,
            &mut self.target,
            "Choose a Part",
        );
        ui.label("Process allowances");
        changed |= combo(
            ui,
            "fdm-aid-process",
            &processes(document),
            &mut self.process,
            "None · contact checks only",
        );
        ui.separator();
        match self.kind {
            AidKind::Tab => {
                for (axis, f) in ["X", "Y", "Z"].iter().zip(&mut self.origin) {
                    changed |= f.show(
                        ui,
                        &format!("Contact {axis}"),
                        "World point on the part outline",
                    );
                }
                changed |= self
                    .angle
                    .show(ui, "Direction", "Outward direction about +Z");
                changed |= self
                    .length
                    .show(ui, "Length", "Outward from the contact point");
                changed |= self.overlap.show(ui, "Overlap", "Back under the part");
                changed |= self.width.show(ui, "Width", "Across the tab");
                changed |= self
                    .thickness
                    .show(ui, "Thickness", "Up from the contact point");
            }
            AidKind::Bridge => {
                for (axis, f) in ["X", "Y", "Z"].iter().zip(&mut self.origin) {
                    changed |= f.show(ui, &format!("Start {axis}"), "Centreline start");
                }
                for (axis, f) in ["X", "Y", "Z"].iter().zip(&mut self.end) {
                    changed |= f.show(ui, &format!("End {axis}"), "Centreline end");
                }
                changed |= self.width.show(ui, "Width", "Horizontal, across the bar");
                changed |= self
                    .thickness
                    .show(ui, "Thickness", "Across the bar and width");
                ui.label("Second Part");
                let others: Vec<_> = parts
                    .iter()
                    .filter(|(p, _)| Some(*p) != self.target)
                    .cloned()
                    .collect();
                changed |= combo(
                    ui,
                    "fdm-aid-other",
                    &others,
                    &mut self.other,
                    "None · same Part",
                );
            }
            AidKind::Support => {
                changed |= self.origin[0].show(ui, "Footprint X", "Footprint corner");
                changed |= self.origin[1].show(ui, "Footprint Y", "Footprint corner");
                changed |= self
                    .angle
                    .show(ui, "Direction", "Footprint length direction");
                changed |= self.length.show(ui, "Length", "Along the direction");
                changed |= self.width.show(ui, "Width", "Across the direction");
                changed |= self
                    .base
                    .show(ui, "Base", "Bottom height, usually the plate");
                changed |= self.gap.show(ui, "Gap", "Below the lowest part surface");
            }
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
                    request: ModelRequest::FdmAid(Box::new(request)),
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
        commands::Channel,
        model::{PrimitiveFrame, PrimitiveParameters, PrimitiveRequest},
    };
    use multicad_model::Compiler;

    fn session() -> (Session, EntityId) {
        let mut s = Session::new(Document::new("FDM form").unwrap()).unwrap();
        let block = PrimitiveRequest::new(
            "Block",
            PrimitiveParameters::Box {
                size: [length(40.), length(20.), length(10.)],
                frame: PrimitiveFrame {
                    origin: [length(0.), length(0.), length(15.)],
                    ..Default::default()
                },
            },
        );
        let p = model::prepare_primitive(&s, 0, Actor::local(Channel::Ui), &block).unwrap();
        s.commit(p.prepared).unwrap();
        (s, block.body)
    }

    #[test]
    fn form_adds_a_support_and_reopens_it_for_editing() {
        let (mut s, block) = session();
        let mut form = FdmAidPanel::new(s.document(), "Support 1".into());
        assert_eq!(form.target, Some(block));
        form.kind = AidKind::Support;
        form.origin[0] = field(5., "mm");
        form.origin[1] = field(5., "mm");
        form.length = field(30., "mm");
        form.width = field(10., "mm");
        let command = form.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), None);
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        let body = prepared.outputs[0].body;
        let feature = prepared.outputs[0].feature;
        let model = Compiler::default()
            .compile(prepared.prepared.document(), 1)
            .unwrap();
        let top = model
            .solid(model.snapshot().bodies[&body])
            .unwrap()
            .bounds()
            .unwrap()
            .max[2];
        assert!((top - 14.8).abs() < 1e-6, "{top}");
        s.commit(prepared.prepared).unwrap();

        // The aid is not offered as a finished Part for another aid.
        assert_eq!(finished_parts(s.document()).len(), 1);
        let mut edit = FdmAidPanel::existing(s.document(), feature).unwrap();
        assert_eq!(edit.kind, AidKind::Support);
        assert_eq!(edit.name, "Support 1");
        edit.gap = field(1., "mm");
        let command = edit.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), Some(feature));
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(prepared.outputs[0].body, body);
        // A tab without a thickness is refused before Preview.
        let mut bad = FdmAidPanel::new(s.document(), "Tab".into());
        bad.thickness = field(0., "mm");
        let error = bad
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, Actor::local(Channel::Ui))
            .unwrap_err();
        assert!(error.contains("thickness"), "{error}");
    }
}
