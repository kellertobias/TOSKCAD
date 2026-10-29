//! Weldment connection inspector: connect two members with a paired miter,
//! a square butt joint or a cope, or edit an existing connection. The cuts
//! are derived natively from both members.
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
    model::{self, ConnectionDefinition, ConnectionKind, length},
};

fn member_parts(document: &Document) -> Vec<(EntityId, String)> {
    model::members(document)
        .filter_map(|e| model::read_member(document, e.id).ok())
        .map(|m| {
            let name = document
                .get(m.output.body)
                .map_or("Member".into(), |e| e.name.clone());
            (m.output.body, name)
        })
        .collect()
}
fn connection_list(document: &Document) -> Vec<(EntityId, String)> {
    model::connections(document)
        .map(|e| (e.id, e.name.clone()))
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

pub(super) struct ConnectionPanel {
    stamp: FormStamp,
    existing: Option<EntityId>,
    name: String,
    kind: ConnectionKind,
    members: [Option<EntityId>; 2],
    gap: DimensionField,
    error: Option<String>,
}
impl ConnectionPanel {
    pub fn new(document: &Document, name: String) -> Self {
        let parts = member_parts(document);
        Self {
            stamp: FormStamp::new(),
            existing: None,
            name,
            kind: ConnectionKind::Miter,
            members: [parts.first().map(|p| p.0), parts.get(1).map(|p| p.0)],
            gap: DimensionField::from_dimension(&length(0.), "mm").expect("literal field"),
            error: None,
        }
    }
    /// Load an existing connection into the form.
    fn open(&mut self, document: &Document, id: EntityId) -> Result<(), String> {
        let c = model::read_connection(document, id).map_err(|e| e.to_string())?;
        self.existing = Some(id);
        self.name = document.get(id).map(|e| e.name.clone()).unwrap_or_default();
        self.kind = c.kind;
        self.members = c.members.map(Some);
        self.gap = DimensionField::from_dimension(c.gap.as_ref().unwrap_or(&length(0.)), "mm")?;
        Ok(())
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Miter / butt / cope"
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<WeldmentCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let [Some(a), Some(b)] = self.members else {
            return Err("Choose both members".into());
        };
        if a == b {
            return Err("Choose two different members".into());
        }
        let gap = self.gap.resolve(document)?;
        let zero = matches!(gap, Dimension::Literal { value, .. } if value == 0.);
        Ok(WeldmentCommand::Connection {
            existing: self.existing,
            name: self.name.clone(),
            connection: ConnectionDefinition {
                kind: self.kind,
                members: [a, b],
                gap: (!zero).then_some(gap),
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
        ui.small("Cuts both member ends from their actual geometry. The members stay separate Parts, and the cuts follow later path edits.");
        let existing = connection_list(document);
        if !existing.is_empty() {
            let mut pick = self.existing;
            if combo(
                ui,
                "weld-connection-existing",
                "New connection",
                &existing,
                &mut pick,
            ) && let Some(id) = pick
            {
                if let Err(e) = self.open(document, id) {
                    self.error = Some(e);
                }
                changed = true;
            }
        }
        ui.label("Name");
        changed |= ui
            .add_enabled(
                self.existing.is_none(),
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        ui.horizontal(|ui| {
            for (k, text) in [
                (ConnectionKind::Miter, "Miter"),
                (ConnectionKind::Butt, "Butt"),
                (ConnectionKind::Cope, "Cope"),
            ] {
                changed |= ui.radio_value(&mut self.kind, k, text).changed();
            }
        });
        let (first, second) = match self.kind {
            ConnectionKind::Miter => ("First member", "Second member"),
            ConnectionKind::Butt => ("Terminating member", "Continuing member"),
            ConnectionKind::Cope => ("Branch", "Receiving member"),
        };
        ui.small(match self.kind {
            ConnectionKind::Miter => "Both members end at the joint and are cut on the bisecting plane. Their sections must mirror each other.",
            ConnectionKind::Butt => "The terminating member ends square against a flat face of the continuing member, which is cut square to cover a corner.",
            ConnectionKind::Cope => "The branch is fitted to the actual outside of the receiving member, plus the gap.",
        });
        let parts = member_parts(document);
        if parts.len() < 2 {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Create at least two members with Tube / profile first.",
            );
        }
        ui.label(first);
        changed |= combo(
            ui,
            "weld-connection-a",
            "Choose a member",
            &parts,
            &mut self.members[0],
        );
        ui.label(second);
        changed |= combo(
            ui,
            "weld-connection-b",
            "Choose a member",
            &parts,
            &mut self.members[1],
        );
        changed |= self.gap.show(
            ui,
            "Gap",
            "Clearance between the cut end and the face it meets; a miter gap is shared",
        );
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
        model::{MemberDefinition, MemberPath, MemberRequest},
    };
    use multicad_model::Compiler;

    #[test]
    fn form_miters_two_members_and_reopens_the_connection() {
        let mut s = Session::new(Document::new("Connection form").unwrap()).unwrap();
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
        let actor = || Actor::local(Channel::Ui);
        s.commit(prepare_library(&s, 0, actor(), &[steel, shs]).unwrap())
            .unwrap();
        let member = |name: &str, a: [f64; 3], b: [f64; 3]| {
            MemberRequest::new(
                name,
                MemberDefinition {
                    output: model::BodyOutput {
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
        let requests = [
            member("A", [0., 0., 0.], [500., 0., 0.]),
            member("B", [500., 0., 0.], [500., 400., 0.]),
        ];
        let p = model::prepare_members(&s, s.revision(), actor(), &requests, None).unwrap();
        s.commit(p.prepared).unwrap();

        let form = ConnectionPanel::new(s.document(), "Corner".into());
        assert_eq!(form.members.map(|m| m.is_some()), [true, true]);
        let command = form.request(s.document(), true).unwrap();
        let prepared = command.prepare(&s, actor()).unwrap();
        assert_eq!(prepared.outputs.len(), 2);
        let list = Compiler::default()
            .compile(prepared.prepared.document(), s.revision() + 1)
            .unwrap()
            .member_cut_list(prepared.prepared.document())
            .unwrap();
        let a = list.iter().find(|r| r.name == "A").unwrap();
        assert_eq!(a.ends[1].kind, Some(ConnectionKind::Miter));
        assert!((a.length_mm - 515.).abs() < 1e-6);
        s.commit(prepared.prepared).unwrap();

        // Reopening the connection edits it in place.
        let id = model::connections(s.document()).next().unwrap().id;
        let mut edit = ConnectionPanel::new(s.document(), String::new());
        edit.open(s.document(), id).unwrap();
        assert_eq!(edit.name, "Corner");
        edit.kind = ConnectionKind::Butt;
        let WeldmentCommand::Connection { existing, .. } =
            edit.request(s.document(), true).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(existing, Some(id));
        // The same member twice is refused before preview.
        edit.members[1] = edit.members[0];
        assert!(edit.request(s.document(), true).is_err());
    }
}
