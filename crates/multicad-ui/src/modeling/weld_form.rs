//! Weld inspector: attach a weld to a member connection (or edit one), and
//! author the weld sequence by moving welds up and down. Welds and the
//! sequence are documentation, not welding procedures or calculations.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
    weldment::WeldmentCommand,
};
use crate::{controls, egui};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    drawings::{WeldSide, WeldSymbol},
    model::{self, SequenceDefinition, SequenceStep, WeldDefinition, length},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Weld,
    Sequence,
}
fn named(document: &Document, ids: impl Iterator<Item = EntityId>) -> Vec<(EntityId, String)> {
    ids.map(|id| (id, document.get(id).map_or("".into(), |e| e.name.clone())))
        .collect()
}
fn combo(
    ui: &mut egui::Ui,
    id: &str,
    empty: &str,
    items: &[(EntityId, String)],
    value: &mut Option<EntityId>,
) -> bool {
    let selected = value
        .and_then(|v| items.iter().find(|(s, _)| *s == v))
        .map_or(empty.into(), |(_, n)| n.clone());
    let mut changed = false;
    controls::inspector_combo(ui, id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (s, n) in items {
                changed |= ui.selectable_value(value, Some(*s), n).changed();
            }
        });
    changed
}
const SYMBOLS: [(WeldSymbol, &str); 7] = [
    (WeldSymbol::Fillet, "Fillet"),
    (WeldSymbol::SquareGroove, "Square groove"),
    (WeldSymbol::VGroove, "V groove"),
    (WeldSymbol::BevelGroove, "Bevel groove"),
    (WeldSymbol::Plug, "Plug"),
    (WeldSymbol::Spot, "Spot"),
    (WeldSymbol::Seam, "Seam"),
];

pub(super) struct WeldPanel {
    stamp: FormStamp,
    kind: Kind,
    existing: Option<EntityId>,
    connection: Option<EntityId>,
    mark: String,
    symbol: WeldSymbol,
    side: WeldSide,
    size: DimensionField,
    all_around: bool,
    field: bool,
    process: String,
    notes: String,
    /// The edited sequence (the first one in the document), and its steps.
    sequence: Option<EntityId>,
    steps: Vec<SequenceStep>,
    error: Option<String>,
}
impl WeldPanel {
    pub fn new(document: &Document) -> Self {
        let used: Vec<String> = model::welds(document)
            .filter_map(|e| model::read_weld(document, e.id).ok())
            .map(|w| w.mark)
            .collect();
        let mark = (1..)
            .map(|i| format!("W{i}"))
            .find(|m| !used.contains(m))
            .expect("unbounded marks");
        let sequence = model::sequences(document).next().map(|e| e.id);
        let steps = sequence
            .and_then(|id| model::read_sequence(document, id).ok())
            .map_or(vec![], |s| s.steps);
        Self {
            stamp: FormStamp::new(),
            kind: Kind::Weld,
            existing: None,
            connection: model::connections(document).next().map(|e| e.id),
            mark,
            symbol: WeldSymbol::Fillet,
            side: WeldSide::Arrow,
            size: DimensionField::from_dimension(&length(4.), "mm").expect("literal field"),
            all_around: false,
            field: false,
            process: String::new(),
            notes: String::new(),
            sequence,
            steps,
            error: None,
        }
    }
    fn open(&mut self, document: &Document, id: EntityId) -> Result<(), String> {
        let w = model::read_weld(document, id).map_err(|e| e.to_string())?;
        self.existing = Some(id);
        self.connection = Some(w.connection);
        self.mark = w.mark;
        self.symbol = w.symbol;
        self.side = w.side;
        self.size = DimensionField::from_dimension(w.size.as_ref().unwrap_or(&length(0.)), "mm")?;
        self.all_around = w.all_around;
        self.field = w.field;
        self.process = w.process;
        self.notes = w.notes;
        Ok(())
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        match self.kind {
            Kind::Weld => "Weld joint",
            Kind::Sequence => "Weld sequence",
        }
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<WeldmentCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        match self.kind {
            Kind::Weld => {
                let connection = self
                    .connection
                    .ok_or("Choose the member connection; create one with Miter / cope")?;
                let size = self.size.resolve(document)?;
                let zero = matches!(size, Dimension::Literal { value, .. } if value == 0.);
                let weld = WeldDefinition {
                    connection,
                    mark: self.mark.trim().to_owned(),
                    symbol: self.symbol,
                    side: self.side,
                    size: (!zero).then_some(size),
                    length: None,
                    all_around: self.all_around,
                    field: self.field,
                    process: self.process.trim().to_owned(),
                    notes: self.notes.trim().to_owned(),
                };
                weld.validate().map_err(|e| e.to_string())?;
                Ok(WeldmentCommand::Weld {
                    existing: self.existing,
                    weld: Box::new(weld),
                })
            }
            Kind::Sequence => {
                let sequence = SequenceDefinition {
                    steps: self.steps.clone(),
                };
                sequence.validate().map_err(|e| e.to_string())?;
                Ok(WeldmentCommand::Sequence {
                    existing: self.sequence,
                    sequence,
                })
            }
        }
    }
    fn show_weld(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        let welds = named(document, model::welds(document).map(|e| e.id));
        if !welds.is_empty() {
            let mut pick = self.existing;
            if combo(ui, "weld-existing", "New weld", &welds, &mut pick)
                && let Some(id) = pick
            {
                if let Err(e) = self.open(document, id) {
                    self.error = Some(e);
                }
                changed = true;
            }
        }
        let connections = named(document, model::connections(document).map(|e| e.id));
        if connections.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Connect two members with Miter / cope first.",
            );
        }
        ui.label("Connection");
        changed |= combo(
            ui,
            "weld-connection",
            "Choose a connection",
            &connections,
            &mut self.connection,
        );
        ui.label("Mark");
        changed |= ui.text_edit_singleline(&mut self.mark).changed();
        controls::inspector_combo(ui, "weld-symbol")
            .selected_text(
                SYMBOLS
                    .iter()
                    .find(|(s, _)| *s == self.symbol)
                    .map_or("", |s| s.1),
            )
            .show_ui(ui, |ui| {
                for (s, label) in SYMBOLS {
                    changed |= ui.selectable_value(&mut self.symbol, s, label).changed();
                }
            });
        ui.horizontal(|ui| {
            for (s, label) in [
                (WeldSide::Arrow, "Arrow side"),
                (WeldSide::Other, "Other side"),
                (WeldSide::Both, "Both"),
            ] {
                changed |= ui.radio_value(&mut self.side, s, label).changed();
            }
        });
        changed |= self.size.show(
            ui,
            "Size",
            "Authored weld size, e.g. the fillet leg; 0 for none",
        );
        changed |= ui.checkbox(&mut self.all_around, "All around").changed();
        changed |= ui.checkbox(&mut self.field, "Field weld").changed();
        ui.label("Process");
        changed |= ui.text_edit_singleline(&mut self.process).changed();
        ui.label("Access and orientation notes");
        changed |= ui.text_edit_multiline(&mut self.notes).changed();
        changed
    }
    fn show_sequence(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        let mark = |id: EntityId| model::read_weld(document, id).map_or("?".into(), |w| w.mark);
        let mut swap = None;
        let mut remove = None;
        for (i, step) in self.steps.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}. {}", i + 1, mark(step.weld)));
                if ui.small_button("Up").clicked() && i > 0 {
                    swap = Some(i - 1);
                }
                if ui.small_button("Down").clicked() && i + 1 < self.steps.len() {
                    swap = Some(i);
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = swap {
            self.steps.swap(i, i + 1);
            changed = true;
        }
        if let Some(i) = remove {
            self.steps.remove(i);
            changed = true;
        }
        let unsequenced: Vec<_> = model::welds(document)
            .map(|e| e.id)
            .filter(|w| !self.steps.iter().any(|s| s.weld == *w))
            .collect();
        for w in unsequenced {
            if ui.button(format!("Add {}", mark(w))).clicked() {
                self.steps.push(SequenceStep {
                    weld: w,
                    note: String::new(),
                });
                changed = true;
            }
        }
        changed
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
        ui.horizontal(|ui| {
            changed |= ui.radio_value(&mut self.kind, Kind::Weld, "Weld").changed();
            changed |= ui
                .radio_value(&mut self.kind, Kind::Sequence, "Sequence")
                .changed();
        });
        ui.strong(self.label());
        ui.small("Documentation for drawings and schedules: not a welding procedure, distortion, strength or heat-input calculation.");
        changed |= match self.kind {
            Kind::Weld => self.show_weld(ui, document),
            Kind::Sequence => self.show_sequence(ui, document),
        };
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
                    request: ModelRequest::Weldment(Box::new(request)),
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
        commands::{Actor, Channel, Session},
        document::{Entity, EntityData},
        materials::{LibraryDefinition, ProfileShape, Stock, StockKind, prepare_library},
        model::{
            BodyOutput, ConnectionDefinition, ConnectionKind, MemberDefinition, MemberPath,
            MemberRequest,
        },
    };
    use multicad_model::Compiler;

    #[test]
    fn form_adds_a_weld_and_orders_the_sequence() {
        let actor = || Actor::local(Channel::Ui);
        let mut s = Session::new(Document::new("Weld form").unwrap()).unwrap();
        let steel = Entity::new(
            "Steel",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [120, 124, 130, 255],
                section_hatch: None,
            },
        );
        let shs = LibraryDefinition::Stock(Stock {
            material: steel.id,
            specification: StockKind::Profile {
                designation: "SHS 30".into(),
                length_mm: 6000.,
                width_mm: 30.,
                height_mm: 30.,
                wall_mm: Some(2.),
                shape: ProfileShape::Rectangular,
            },
        })
        .entity(EntityId::new(), "SHS 30")
        .unwrap();
        let stock = shs.id;
        s.commit(prepare_library(&s, 0, actor(), &[steel, shs]).unwrap())
            .unwrap();
        let member = |name: &str, a: [f64; 3], b: [f64; 3]| {
            MemberRequest::new(
                name,
                MemberDefinition {
                    output: BodyOutput {
                        body: EntityId::new(),
                    },
                    stock,
                    path: MemberPath::Points {
                        start: a.map(length),
                        end: b.map(length),
                    },
                    anchor: Default::default(),
                    rotation: None,
                    extend_start: None,
                    extend_end: None,
                },
            )
        };
        let all = [
            member("A", [0., 0., 0.], [500., 0., 0.]),
            member("B", [500., 0., 0.], [500., 400., 0.]),
        ];
        let p = model::prepare_members(&s, s.revision(), actor(), &all, None).unwrap();
        s.commit(p.prepared).unwrap();
        let (corner, p) = model::prepare_connection(
            &s,
            s.revision(),
            actor(),
            "Corner",
            &ConnectionDefinition {
                kind: ConnectionKind::Miter,
                members: [all[0].body, all[1].body],
                gap: None,
            },
        )
        .unwrap();
        s.commit(p).unwrap();

        // Two welds on the same connection through the form.
        for process in ["135", "141"] {
            let mut form = WeldPanel::new(s.document());
            assert_eq!(form.connection, Some(corner));
            form.process = process.into();
            let prepared = form
                .request(s.document(), true)
                .unwrap()
                .prepare(&s, actor())
                .unwrap();
            s.commit(prepared.prepared).unwrap();
        }
        let marks: Vec<_> = model::welds(s.document())
            .map(|e| model::read_weld(s.document(), e.id).unwrap().mark)
            .collect();
        assert_eq!(marks.len(), 2);
        assert!(marks.contains(&"W1".to_owned()) && marks.contains(&"W2".to_owned()));

        // A sequence of both, then moved: the order changes, the welds do not.
        let mut form = WeldPanel::new(s.document());
        form.kind = Kind::Sequence;
        for w in model::welds(s.document()).map(|e| e.id).collect::<Vec<_>>() {
            form.steps.push(SequenceStep {
                weld: w,
                note: String::new(),
            });
        }
        let first = form.steps[0].weld;
        form.steps.swap(0, 1);
        let prepared = form
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, actor())
            .unwrap();
        s.commit(prepared.prepared).unwrap();
        let schedule = Compiler::default()
            .compile(s.document(), s.revision())
            .unwrap()
            .weld_schedule(s.document())
            .unwrap();
        assert_eq!(schedule[1].weld, first);
        assert_eq!(schedule[1].step, Some(2));
        assert!(schedule.iter().all(|r| r.connection == corner));
        // An empty mark is refused before preview.
        let mut bad = WeldPanel::new(s.document());
        bad.mark = " ".into();
        assert!(bad.request(s.document(), true).is_err());
    }
}
