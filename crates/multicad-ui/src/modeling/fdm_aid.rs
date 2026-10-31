//! FDM aid inspector: add or edit a breakaway tab, thin bridge or removable
//! support as its own Part serving a finished Part. The finished Part is
//! never changed; a support's height is derived from it natively. Contacts
//! and directions can be picked on the part in the viewer (`placement`).
mod placement;
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
pub(crate) use placement::{PickRole, Review, placement_block};

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
/// The finished Part a new aid starts with, from the selection: the one
/// selected finished Part (a selected feature counts for its Part). Without
/// a selection, the document's only finished Part. Anything else leaves the
/// choice open with the reason, never an unrelated first Part.
pub(super) fn seed_target(
    document: &Document,
    selection: &[EntityId],
) -> (Option<EntityId>, Option<String>) {
    let parts = finished_parts(document);
    let finished = |id: EntityId| parts.iter().any(|(p, _)| *p == id);
    let name = |id: EntityId| document.get(id).map_or(String::new(), |e| e.name.clone());
    if selection.is_empty() {
        return match parts.as_slice() {
            [(only, _)] => (Some(*only), None),
            [] => (None, None),
            _ => (
                None,
                Some("Select the finished Part first, or choose it below".into()),
            ),
        };
    }
    let mut chosen: Vec<EntityId> = vec![];
    let mut refused = vec![];
    for &id in selection {
        match super::panel_face::selected_part(document, id) {
            Some(part) if finished(part) => {
                if !chosen.contains(&part) {
                    chosen.push(part);
                }
            }
            Some(part) => refused.push(format!(
                "{} is a print aid, not a finished Part",
                name(part)
            )),
            None => refused.push(format!("{} is not a Part", name(id))),
        }
    }
    match chosen.as_slice() {
        [one] => (Some(*one), None),
        [] => (
            None,
            Some(format!(
                "{}; choose the finished Part below",
                refused.join("; ")
            )),
        ),
        many => (
            None,
            Some(format!(
                "{} Parts are selected; choose the one the aid serves",
                many.len()
            )),
        ),
    }
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
    /// Why the finished Part was not taken from the selection.
    seed_note: Option<String>,
    error: Option<String>,
    picking: Option<PickRole>,
    hover: Option<placement::Hover>,
    review: Option<Review>,
}
impl FdmAidPanel {
    pub fn new(document: &Document, name: String) -> Self {
        Self::with_selection(document, name, &[])
    }
    /// The finished Part the aid serves, once chosen.
    pub fn target(&self) -> Option<EntityId> {
        self.target
    }
    /// A new aid whose finished Part comes from the selection.
    pub fn with_selection(document: &Document, name: String, selection: &[EntityId]) -> Self {
        let (target, seed_note) = seed_target(document, selection);
        Self {
            seed_note,
            stamp: FormStamp::new(),
            kind: AidKind::Tab,
            existing: None,
            feature: EntityId::new(),
            body: EntityId::new(),
            name,
            target,
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
            // A new aid starts by picking its contact on the part.
            picking: Some(PickRole::Contact),
            hover: None,
            review: None,
        }
    }
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let aid = model::read_aid(document, feature).map_err(|e| e.to_string())?;
        let mut form = Self::new(document, String::new());
        form.existing = Some(feature);
        form.feature = feature;
        form.picking = None;
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
    /// The preview state Apply sees: a ready preview whose native contact
    /// check refuses the placement is not ready to Apply.
    fn apply_gate<'a>(&self, preview: PreviewState<'a>) -> PreviewState<'a> {
        if self
            .current_review()
            .is_some_and(|review| review.blocked.is_some())
        {
            PreviewState::Idle
        } else {
            preview
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
                for (kind, label) in [
                    (AidKind::Tab, "Tab"),
                    (AidKind::Bridge, "Bridge"),
                    (AidKind::Support, "Support"),
                ] {
                    if ui.radio_value(&mut self.kind, kind, label).changed() {
                        changed = true;
                        self.picking = Some(PickRole::Contact);
                        self.hover = None;
                    }
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
        let picked = combo(
            ui,
            "fdm-aid-target",
            &parts,
            &mut self.target,
            "Choose a Part",
        );
        if picked {
            self.seed_note = None;
        }
        changed |= picked;
        if let Some(note) = &self.seed_note {
            ui.colored_label(egui::Color32::LIGHT_RED, note);
        }
        ui.label("Process allowances");
        changed |= combo(
            ui,
            "fdm-aid-process",
            &processes(document),
            &mut self.process,
            "None · contact checks only",
        );
        ui.separator();
        changed |= self.show_placement(ui, document);
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
        // A ready preview whose native contact check refuses the placement
        // offers Preview again but never Apply.
        let gate = self.apply_gate(preview);
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, gate);
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
                if gate.ready(self.stamp) && self.request(document, enabled).is_ok() =>
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

    fn surface(
        point: [f64; 3],
        normal: [f64; 3],
        exact_plane: bool,
    ) -> crate::viewport::SurfacePoint {
        crate::viewport::SurfacePoint {
            point_mm: point,
            normal,
            exact_plane,
        }
    }
    fn second_part(s: &mut Session) -> EntityId {
        let post = PrimitiveRequest::new(
            "Post",
            PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(30.)],
                frame: PrimitiveFrame {
                    origin: [length(60.), length(0.), length(0.)],
                    ..Default::default()
                },
            },
        );
        let p =
            model::prepare_primitive(s, s.revision(), Actor::local(Channel::Ui), &post).unwrap();
        s.commit(p.prepared).unwrap();
        post.body
    }

    #[test]
    fn picks_fill_the_inspector_coordinates_for_each_aid_kind() {
        let (mut s, block) = session();
        let post = second_part(&mut s);
        let doc = s.document();
        // Tab: an exact -Y face pick sits at the given base, pointing -Y.
        let mut tab = FdmAidPanel::new(doc, "Tab".into());
        assert_eq!(tab.picking, Some(PickRole::Contact));
        let stamp = tab.stamp();
        let message = tab
            .pick_surface(
                doc,
                block,
                surface([12.5, 0., 20.], [0., -1., 0.], true),
                Some(15.),
            )
            .unwrap();
        assert!(message.contains("exact plane"), "{message}");
        assert_ne!(tab.stamp(), stamp);
        assert_eq!(tab.picking, None);
        assert_eq!(tab.target, Some(block));
        assert_eq!(tab.origin_mm(doc).unwrap(), [12.5, 0., 15.]);
        assert_eq!(tab.angle_text(), "-90");
        // Aim the direction in the build plane, in whole degrees.
        tab.picking = Some(PickRole::Direction);
        tab.aim(doc, [22.4, 9.8, 15.]).unwrap();
        assert_eq!(tab.angle_text(), "45");
        assert!(tab.aim(doc, [12.5, 0., 15.]).is_err());
        // A top-face pick and an aid Part are refused without a change.
        tab.picking = Some(PickRole::Contact);
        let stamp = tab.stamp();
        let error = tab
            .pick_surface(
                doc,
                block,
                surface([10., 10., 25.], [0., 0., 1.], true),
                Some(15.),
            )
            .unwrap_err();
        assert!(error.contains("side face"), "{error}");
        assert!(
            tab.pick_surface(
                doc,
                EntityId::new(),
                surface([0.; 3], [1., 0., 0.], true),
                None
            )
            .is_err()
        );
        // The contact goes on the chosen finished Part, not a neighbour.
        let error = tab
            .pick_surface(
                doc,
                post,
                surface([60., 5., 20.], [-1., 0., 0.], true),
                Some(0.),
            )
            .unwrap_err();
        assert!(error.contains("chosen finished Part"), "{error}");
        assert_eq!(tab.stamp(), stamp);

        // Bridge: start on the block, end on the post makes it the second Part.
        let mut bridge = FdmAidPanel::new(doc, "Bridge".into());
        bridge.kind = AidKind::Bridge;
        bridge
            .pick_surface(
                doc,
                block,
                surface([40., 10., 20.], [1., 0., 0.], true),
                Some(15.),
            )
            .unwrap();
        assert_eq!(bridge.picking, Some(PickRole::End));
        bridge
            .pick_surface(
                doc,
                post,
                surface([60., 5., 20.], [-1., 0., 0.], true),
                Some(0.),
            )
            .unwrap();
        assert_eq!(bridge.picking, None);
        assert_eq!((bridge.target, bridge.other), (Some(block), Some(post)));
        assert_eq!(bridge.origin_mm(doc).unwrap(), [40., 10., 20.]);
        let command = bridge.request(doc, true).unwrap();
        assert!(matches!(
            command.request.aid.shape,
            AidShape::Bridge { other: Some(o), .. } if o == post
        ));

        // Support: the footprint centres under the pick.
        let mut support = FdmAidPanel::new(doc, "Support".into());
        support.kind = AidKind::Support;
        support.length = field(10., "mm");
        support.width = field(4., "mm");
        support
            .pick_surface(
                doc,
                block,
                surface([20., 10., 15.], [0., 0., -1.], true),
                Some(15.),
            )
            .unwrap();
        assert_eq!(support.origin_mm(doc).unwrap(), [15., 8., 0.]);
        // Aimed at +Y, the footprint is re-centred on the same point.
        support.picking = Some(PickRole::Direction);
        support.aim(doc, [20., 30., 0.]).unwrap();
        assert_eq!(support.angle_text(), "90");
        support.picking = Some(PickRole::Contact);
        support
            .pick_surface(
                doc,
                block,
                surface([20., 10., 15.], [0., 0., -1.], true),
                Some(15.),
            )
            .unwrap();
        assert_eq!(support.origin_mm(doc).unwrap(), [22., 5., 0.]);
        let prepared = support
            .request(doc, true)
            .unwrap()
            .prepare(&s, Actor::local(Channel::Ui))
            .unwrap();
        Compiler::default()
            .compile(prepared.prepared.document(), s.revision() + 1)
            .unwrap();
    }

    #[test]
    fn typed_inspector_values_drive_the_marker_and_direction_handle() {
        let (s, block) = session();
        let doc = s.document();
        let mut tab = FdmAidPanel::new(doc, "Tab".into());
        tab.pick_surface(
            doc,
            block,
            surface([12.5, 0., 20.], [0., -1., 0.], true),
            Some(15.),
        )
        .unwrap();
        // Typed coordinates replace the picked ones as the marker position
        // the viewer draws from, with no pick involved.
        tab.origin[0] = field(30., "mm");
        tab.origin[1] = field(20., "mm");
        assert_eq!(tab.origin_mm(doc).unwrap(), [30., 20., 15.]);
        // The direction handle starts from the typed contact: aiming at a
        // point straight +X of it gives 0 degrees, not the picked -90.
        tab.picking = Some(PickRole::Direction);
        tab.aim(doc, [40., 20., 15.]).unwrap();
        assert_eq!(tab.angle_text(), "0");
        // An incomplete typed value hides the marker instead of drawing a
        // stale position, and the handle cannot be aimed from it.
        tab.origin[0].text = "not a number".into();
        assert_eq!(tab.origin_mm(doc), None);
        tab.picking = Some(PickRole::Direction);
        assert!(tab.aim(doc, [40., 20., 15.]).is_err());
    }

    #[test]
    fn a_blocked_review_keeps_preview_but_never_lets_apply_through() {
        let (s, block) = session();
        let mut form = FdmAidPanel::new(s.document(), "Tab".into());
        let ready = PreviewState::Ready { stamp: form.stamp };
        let review = |blocked: Option<&str>, stamp| Review {
            stamp,
            result: Err("not measured".into()),
            blocked: blocked.map(Into::into),
        };
        assert!(form.apply_gate(ready).ready(form.stamp));
        form.set_review(Some(review(
            Some("The tab does not touch Block"),
            form.stamp,
        )));
        assert!(!form.apply_gate(ready).ready(form.stamp));
        // A review of older inputs does not apply to the current ones.
        let old = form.stamp;
        form.pick_surface(
            s.document(),
            block,
            surface([12.5, 0., 20.], [0., -1., 0.], true),
            Some(15.),
        )
        .unwrap();
        form.set_review(Some(review(Some("stale"), old)));
        let ready = PreviewState::Ready { stamp: form.stamp };
        assert!(form.apply_gate(ready).ready(form.stamp));
        form.set_review(Some(review(None, form.stamp)));
        assert!(form.apply_gate(ready).ready(form.stamp));
    }
}
