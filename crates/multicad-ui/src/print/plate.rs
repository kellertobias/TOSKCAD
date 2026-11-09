//! Editor for saved output-only plate arrangements.
use crate::{controls, egui, viewport::Scene};
use multicad_core::{
    commands::{Actor, Channel, Session, SessionStamp},
    document::{Document, EntityId},
    exchange::{
        MAX_PRINT_PARTS, PartRef, PrintPlateDefinition, PrintPlatePlacement, PrintPreview,
        prepare_print_plate, print_candidates, print_plates, read_print_plate, resolve_occurrences,
    },
};
use std::collections::BTreeSet;

mod canvas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateEditorAction {
    Save,
    Discard,
}

#[derive(Clone, PartialEq)]
struct Draft {
    id: EntityId,
    name: String,
    definition: PrintPlateDefinition,
}
struct Choice {
    reference: PartRef,
    label: String,
}

#[derive(Default)]
pub struct PlateEditor {
    draft: Option<Draft>,
    original: Option<Draft>,
    stamp: Option<SessionStamp>,
    selected_placement: Option<usize>,
    add_source: Option<PartRef>,
    choices: Vec<Choice>,
    choices_stamp: Option<(EntityId, u64)>,
    view: canvas::View,
}

impl PlateEditor {
    pub fn selected(&self) -> Option<EntityId> {
        Some(self.draft.as_ref()?.id)
    }

    pub fn name(&self) -> Option<&str> {
        Some(&self.draft.as_ref()?.name)
    }

    pub fn definition(&self) -> Option<&PrintPlateDefinition> {
        Some(&self.draft.as_ref()?.definition)
    }

    pub fn dirty(&self) -> bool {
        self.draft != self.original
    }

    pub fn new_plate(&mut self, session: &Session) -> Result<(), String> {
        self.require_clean()?;
        let document = session.document();
        let names: BTreeSet<_> = print_plates(document)
            .filter_map(|id| document.get(id).map(|entity| entity.name.as_str()))
            .collect();
        let name = (1..)
            .map(|number| format!("Print plate {number}"))
            .find(|name| !names.contains(name.as_str()))
            .expect("finite existing names");
        self.draft = Some(Draft {
            id: EntityId::new(),
            name,
            definition: PrintPlateDefinition::default(),
        });
        self.original = None;
        self.stamp = Some(session.stamp());
        self.reset_selection();
        Ok(())
    }

    pub fn open(&mut self, session: &Session, id: EntityId) -> Result<(), String> {
        self.require_clean()?;
        self.load(session, id)?;
        self.reset_selection();
        Ok(())
    }

    pub fn sync(&mut self, session: &Session) {
        if !self.dirty()
            && self
                .stamp
                .as_ref()
                .is_some_and(|stamp| session.validate_stamp(stamp).is_err())
            && let Some(id) = self.selected()
            && self.load(session, id).is_err()
        {
            self.clear();
        }
    }

    pub fn save(&mut self, session: &mut Session, channel: Channel) -> Result<String, String> {
        let draft = self
            .draft
            .as_ref()
            .ok_or("Create or select a print plate")?;
        session
            .validate_stamp(self.stamp.as_ref().ok_or("Open the print plate first")?)
            .map_err(|_| "The project changed. Discard or reopen the plate before saving.")?;
        let prepared = prepare_print_plate(
            session,
            session.revision(),
            Actor::local(channel),
            draft.id,
            draft.name.trim(),
            &draft.definition,
        )
        .map_err(|error| error.to_string())?;
        let id = draft.id;
        let revision = session.revision();
        session
            .commit(prepared)
            .map_err(|error| error.to_string())?;
        self.load(session, id)?;
        Ok(if revision == session.revision() {
            "No print plate changes"
        } else {
            "Print plate saved · one Undo step"
        }
        .into())
    }

    pub fn discard(&mut self, session: &Session) {
        if let Some(id) = self.original.as_ref().map(|original| original.id) {
            if self.load(session, id).is_err() {
                self.clear();
            }
        } else {
            self.clear();
        }
        self.reset_selection();
    }

    pub fn show_settings(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        scene: Option<&Scene>,
    ) -> Option<PlateEditorAction> {
        self.sync(session);
        self.refresh_choices(session);
        controls::inspector_layout(ui);
        ui.heading("Print plate settings");
        let dirty = self.dirty();
        let Some(draft) = &mut self.draft else {
            ui.label("Choose or create a print plate.");
            return None;
        };
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(dirty, egui::Button::new("Save plate"))
                .clicked()
            {
                action = Some(PlateEditorAction::Save);
            }
            if ui
                .add_enabled(dirty, egui::Button::new("Discard edits"))
                .clicked()
            {
                action = Some(PlateEditorAction::Discard);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("print-plate-settings")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let name = ui.label("Plate name");
                ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(f32::INFINITY))
                    .labelled_by(name.id);
                ui.strong("Bed size");
                number(ui, "Bed width (X)", &mut draft.definition.bed_mm[0], 1. ..=100_000., " mm", 1.);
                number(ui, "Bed depth (Y)", &mut draft.definition.bed_mm[1], 1. ..=100_000., " mm", 1.);
                ui.separator();
                ui.strong("Add a Part or placed occurrence");
                let placed: BTreeSet<_> = draft.definition.placements.iter().map(|p| &p.source).collect();
                let choice = self.add_source.as_ref()
                    .and_then(|source| self.choices.iter().find(|choice| &choice.reference == source))
                    .filter(|choice| !placed.contains(&choice.reference));
                controls::inspector_combo(ui, "plate-pick-source")
                    .selected_text(choice.map_or("Choose a Part", |choice| &choice.label))
                    .show_ui(ui, |ui| {
                        for choice in &self.choices {
                            if !placed.contains(&choice.reference) && ui
                                .selectable_label(self.add_source.as_ref() == Some(&choice.reference), &choice.label)
                                .clicked()
                            {
                                self.add_source = Some(choice.reference.clone());
                            }
                        }
                    });
                let can_add = draft.definition.placements.len() < MAX_PRINT_PARTS
                    && self.add_source.as_ref().is_some_and(|source| !placed.contains(source));
                if ui.add_enabled(can_add, egui::Button::new("Add to plate")).clicked()
                    && let Some(source) = self.add_source.take()
                {
                    let mut placement = PrintPlatePlacement {
                        source,
                        position_mm: [5., 5.],
                        rotation_deg: [0.; 3],
                    };
                    placement.position_mm = self.view.initial_position(
                        scene, session, &draft.definition, &placement,
                    );
                    draft.definition.placements.push(placement);
                    self.selected_placement = Some(draft.definition.placements.len() - 1);
                }
                if self.choices.is_empty() {
                    ui.small("Create a finished Part or placed occurrence in Model first.");
                }
                ui.separator();
                ui.strong(format!("Placed objects ({})", draft.definition.placements.len()));
                for (index, placement) in draft.definition.placements.iter().enumerate() {
                    let label = source_label(session.document(), &placement.source);
                    if ui.selectable_label(
                        self.selected_placement == Some(index),
                        format!("{}. {label}", index + 1),
                    ).clicked() {
                        self.selected_placement = Some(index);
                    }
                }
                if let Some(index) = self.selected_placement
                    && let Some(placement) = draft.definition.placements.get_mut(index)
                {
                    ui.separator();
                    ui.strong("Selected placement");
                    for (axis, position) in ["X position", "Y position"].into_iter().zip(&mut placement.position_mm) {
                        number(ui, axis, position, -1_000_000. ..=1_000_000., " mm", 0.5);
                    }
                    for (axis, angle) in ["X rotation", "Y rotation", "Z rotation"].into_iter().zip(&mut placement.rotation_deg) {
                        number(ui, axis, angle, -360_000. ..=360_000., "°", 1.);
                    }
                    let remove = ui.horizontal_wrapped(|ui| {
                        if ui.button("Rotate Z +90°").clicked() {
                            placement.rotation_deg[2] = (placement.rotation_deg[2] + 90.).rem_euclid(360.);
                        }
                        ui.button("Remove from plate").clicked()
                    }).inner;
                    if remove {
                        draft.definition.placements.remove(index);
                        self.selected_placement = None;
                    }
                }
                ui.separator();
                ui.collapsing("3MF mesh settings", |ui| {
                    ui.small("3MF stores the arranged positions in millimeters.");
                    number(ui, "Chord deviation", &mut draft.definition.quality.chord_deviation_mm, 0.001 ..=10., " mm", 0.01);
                    number(ui, "Angular deviation", &mut draft.definition.quality.angular_deviation_deg, 0.6 ..=57., "°", 0.1);
                    ui.checkbox(&mut draft.definition.stl_per_part, "Also write one STL per object");
                    if draft.definition.stl_per_part {
                        ui.small("Each STL contains the object in its arranged bed coordinates. STL stores no unit; coordinates are written in millimeters.");
                    }
                });
            });
        action
    }

    pub fn show_canvas(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        scene: Option<&Scene>,
        preview: Option<&PrintPreview>,
    ) {
        if let Some(draft) = &mut self.draft {
            self.view.show(
                ui,
                session,
                scene,
                &mut draft.definition,
                &mut self.selected_placement,
                preview,
            );
        } else {
            ui.label("Choose or create a print plate to arrange its output copies.");
        }
    }

    fn load(&mut self, session: &Session, id: EntityId) -> Result<(), String> {
        let definition =
            read_print_plate(session.document(), id).map_err(|error| error.to_string())?;
        let name = session
            .document()
            .get(id)
            .ok_or("Print plate is missing")?
            .name
            .clone();
        let draft = Draft {
            id,
            name,
            definition,
        };
        self.original = Some(draft.clone());
        self.draft = Some(draft);
        self.stamp = Some(session.stamp());
        if self.selected_placement.is_some_and(|index| {
            self.definition()
                .is_none_or(|def| index >= def.placements.len())
        }) {
            self.selected_placement = None;
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.original = None;
        self.draft = None;
        self.stamp = None;
    }

    fn reset_selection(&mut self) {
        self.selected_placement = None;
        self.add_source = None;
        self.view = canvas::View::default();
    }

    fn require_clean(&self) -> Result<(), String> {
        if self.dirty() {
            return Err("Save or discard the current print plate edits first".into());
        }
        Ok(())
    }

    fn refresh_choices(&mut self, session: &Session) {
        let document = session.document();
        let stamp = (document.id(), session.revision());
        if self.choices_stamp == Some(stamp) {
            return;
        }
        self.choices_stamp = Some(stamp);
        let aids = multicad_core::model::aid_parts(document);
        let mut choices = BTreeSet::new();
        for candidate in print_candidates(document).unwrap_or_default() {
            for part in resolve_occurrences(document, candidate.entity).unwrap_or_default() {
                if !aids.contains(&part.reference.part) {
                    choices.insert((source_label(document, &part.reference), part.reference));
                }
            }
        }
        self.choices = choices
            .into_iter()
            .map(|(label, reference)| Choice { label, reference })
            .collect();
    }
}

fn source_label(document: &Document, source: &PartRef) -> String {
    let name = |id| {
        document
            .get(id)
            .map_or("Missing Part", |entity| entity.name.as_str())
    };
    let part = name(source.part);
    if source.occurrence_path.is_empty() {
        part.into()
    } else {
        let path = source
            .occurrence_path
            .iter()
            .copied()
            .map(name)
            .collect::<Vec<_>>()
            .join(" / ");
        format!("{part} · {path}")
    }
}

fn number(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    unit: &str,
    speed: f64,
) {
    ui.horizontal_wrapped(|ui| {
        let label = ui.label(name);
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .suffix(unit)
                .speed(speed)
                .max_decimals(3),
        )
        .labelled_by(label.id);
    });
}
