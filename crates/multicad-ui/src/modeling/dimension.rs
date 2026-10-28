use crate::egui;
use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    parameters::{BoundExpression, Unit},
};

/// Text stays local, including temporarily incomplete formulas. Stable bindings
/// are retained when editing a previously bound dimension.
#[derive(Clone, Debug)]
pub(super) struct DimensionField {
    pub text: String,
    unit: String,
    previous: Option<BoundExpression>,
}
impl DimensionField {
    pub fn from_dimension(value: &Dimension, unit: &'static str) -> Result<Self, String> {
        let source = Unit::parse(value.unit()).ok_or("Unknown dimension unit")?;
        let target = Unit::parse(unit).ok_or("Unknown field unit")?;
        if source.dimension() != target.dimension() {
            return Err("Incompatible dimension unit".into());
        }
        let text = match value {
            Dimension::Literal { value, .. } => {
                (value * source.scale() / target.scale()).to_string()
            }
            Dimension::Expression { expression } => expression.text.clone(),
        };
        Ok(Self {
            text,
            unit: if value.expression().is_some() {
                value.unit()
            } else {
                unit
            }
            .into(),
            previous: value.expression().cloned(),
        })
    }
    pub fn show(&mut self, ui: &mut egui::Ui, label: &str, help: &str) -> bool {
        ui.label(format!("{label} ({})", self.unit));
        ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .desired_width(ui.available_width())
                .hint_text(format!("Value or =$parameter · {}", self.unit)),
        )
        .on_hover_text(help)
        .changed()
    }
    pub fn resolve(&self, document: &Document) -> Result<Dimension, String> {
        if self.text.trim_start().starts_with('=') {
            return BoundExpression::bind(
                document,
                self.text.trim(),
                &self.unit,
                self.previous.as_ref(),
            )
            .map(|expression| Dimension::Expression { expression })
            .map_err(|error| error.to_string());
        }
        let value = self
            .text
            .trim()
            .parse::<f64>()
            .map_err(|_| "Enter a number or an expression beginning with '='".to_string())?;
        if !value.is_finite() || value.abs() > 1e9 {
            return Err("Value must be finite and no greater than 1,000,000,000".into());
        }
        Ok(Dimension::Literal {
            value,
            unit: self.unit.clone(),
        })
    }
    pub fn remember(&mut self, dimension: &Dimension) {
        self.previous = dimension.expression().cloned();
    }
}
