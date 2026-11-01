//! Shared editable dowel settings for standalone joints and the Cabinet tool.
use super::dimension::DimensionField;
use crate::egui;
use multicad_core::{
    document::Document,
    model::{self, DowelSpec, length},
};

#[derive(Clone, Debug)]
pub(super) struct DowelFields {
    diameter: DimensionField,
    count: u32,
    inset: DimensionField,
    offset: DimensionField,
    face_depth: DimensionField,
    edge_depth: DimensionField,
    fit: DimensionField,
}
impl Default for DowelFields {
    fn default() -> Self {
        Self::of(&DowelSpec::default()).expect("literal dowel settings")
    }
}
impl DowelFields {
    pub(super) fn of(spec: &DowelSpec) -> Result<Self, String> {
        let field = |d| DimensionField::from_dimension(d, "mm");
        Ok(Self {
            diameter: field(&spec.diameter)?,
            count: spec.count,
            inset: field(&spec.inset)?,
            offset: field(&spec.offset)?,
            face_depth: field(&spec.face_depth)?,
            edge_depth: field(&spec.edge_depth)?,
            fit: field(&spec.fit)?,
        })
    }
    pub(super) fn fitted(thickness: f64) -> Self {
        let spec = DowelSpec {
            diameter: length(8_f64.min((thickness - 4.2).max(0.5))),
            face_depth: length(12_f64.min((thickness - 2.).max(0.5))),
            ..Default::default()
        };
        Self::of(&spec).expect("literal fitted dowel settings")
    }
    pub(super) fn spec(&self, document: &Document) -> Result<DowelSpec, String> {
        Ok(DowelSpec {
            diameter: self.diameter.resolve(document)?,
            count: self.count,
            inset: self.inset.resolve(document)?,
            offset: self.offset.resolve(document)?,
            face_depth: self.face_depth.resolve(document)?,
            edge_depth: self.edge_depth.resolve(document)?,
            fit: self.fit.resolve(document)?,
        })
    }
    pub(super) fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = self.diameter.show(
            ui,
            "Dowel diameter",
            "Nominal diameter; receiving holes include the fit allowance",
        );
        ui.horizontal(|ui| {
            ui.label("Dowels per pair");
            changed |= ui
                .add(egui::DragValue::new(&mut self.count).range(1..=model::MAX_TENONS))
                .changed();
        });
        changed |= self.inset.show(
            ui,
            "End inset",
            "First and last centers measured from each end of the mating contact",
        );
        changed |= self.offset.show(
            ui,
            "Thickness offset",
            "Hole center offset from the middle of the edge panel thickness",
        );
        changed |=
            self.face_depth
                .show(ui, "Face receiving depth", "Blind hole into the panel face");
        changed |= self.edge_depth.show(
            ui,
            "End receiving depth",
            "Blind hole into the mating panel end",
        );
        changed |= self.fit.show(
            ui,
            "Diametral fit",
            "Total allowance added to the hole diameter",
        );
        changed
    }
}
