use crate::{egui, modeling::dimension::DimensionField};
use multicad_core::{constraint_sketch::Dimension, document::Document};
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct Fields {
    values: BTreeMap<String, DimensionField>,
    pub error: Option<String>,
    pub changed: bool,
}
impl Fields {
    pub fn start(&mut self) {
        self.error = None;
        self.changed = false;
    }
    pub fn clear(&mut self) {
        self.values.clear();
        self.changed = true;
    }
    pub fn length(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        key: String,
        label: &str,
        value: &mut Dimension,
    ) {
        let field = self
            .values
            .entry(key)
            .or_insert_with(|| DimensionField::from_dimension(value, "mm").expect("length field"));
        self.changed |= field.show(
            ui,
            label,
            "Enter a length or an expression beginning with =; use $name to reference a parameter",
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
