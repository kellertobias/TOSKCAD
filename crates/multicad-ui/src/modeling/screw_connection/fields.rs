use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub(super) struct Fields {
    values: BTreeMap<(EntityId, &'static str), DimensionField>,
    pub error: Option<String>,
    pub changed: bool,
}

impl Fields {
    pub fn remove_owner(&mut self, owner: EntityId) {
        self.values.retain(|(id, _), _| *id != owner);
    }
    pub fn remove_field(&mut self, owner: EntityId, key: &'static str) {
        self.values.remove(&(owner, key));
    }
    pub fn start(&mut self) {
        self.error = None;
        self.changed = false;
    }

    pub fn dimension(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        owner: EntityId,
        field: (&'static str, &str, &'static str),
        value: &mut Dimension,
    ) {
        let (key, label, unit) = field;
        let field = match self.values.entry((owner, key)) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                match DimensionField::from_dimension(value, unit) {
                    Ok(field) => entry.insert(field),
                    Err(error) => {
                        self.error.get_or_insert(error);
                        return;
                    }
                }
            }
        };
        self.changed |= field.show_named(
            ui,
            ("screw-connection-field", owner, key),
            label,
            "Retain a typed value or use =$parameter; Preview checks the complete connection",
        );
        match field.resolve(document) {
            Ok(next) => {
                *value = next;
                field.remember(value);
            }
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, &error);
                self.error.get_or_insert(error);
            }
        }
    }
}
