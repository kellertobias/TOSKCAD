use crate::egui;
use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    parameters::{BoundExpression, Unit},
};

/// Text stays local, including temporarily incomplete formulas. Stable bindings
/// are retained when editing a previously bound dimension.
#[derive(Clone, Debug)]
pub(crate) struct DimensionField {
    pub text: String,
    unit: String,
    previous: Option<BoundExpression>,
    original: Dimension,
    original_text: String,
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
            original_text: text.clone(),
            original: value.clone(),
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
    /// Replace the text with a literal given in `unit`, converted to this
    /// field's own unit (a formula field may keep an authored unit).
    pub fn set_literal(&mut self, value: f64, unit: &str) {
        let scale = match (Unit::parse(unit), Unit::parse(&self.unit)) {
            (Some(from), Some(to)) if from.dimension() == to.dimension() => {
                from.scale() / to.scale()
            }
            _ => 1.,
        };
        self.text = super::value_entry::format_number(value * scale);
    }
    pub fn show(&mut self, ui: &mut egui::Ui, label: &str, help: &str) -> bool {
        ui.label(format!("{label} ({})", self.unit));
        ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .desired_width(crate::controls::inspector_width(ui))
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
        if self.text.trim() == self.original_text.trim()
            && matches!(self.original, Dimension::Literal { .. })
        {
            return Ok(self.original.clone());
        }
        Ok(Dimension::Literal {
            value,
            unit: self.unit.clone(),
        })
    }
    pub fn remember(&mut self, dimension: &Dimension) {
        self.previous = dimension.expression().cloned();
        self.original = dimension.clone();
        self.original_text = self.text.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::document::{Entity, EntityData};
    #[test]
    fn unchanged_literal_cm_and_rad_retain_authored_units_until_edited() {
        let doc = Document::new("Unit baseline").unwrap();
        for (original, target) in [
            (
                Dimension::Literal {
                    value: 2.,
                    unit: "cm".into(),
                },
                "mm",
            ),
            (
                Dimension::Literal {
                    value: std::f64::consts::FRAC_PI_2,
                    unit: "rad".into(),
                },
                "deg",
            ),
        ] {
            let mut field = DimensionField::from_dimension(&original, target).unwrap();
            assert_eq!(field.resolve(&doc).unwrap(), original);
            field.remember(&original);
            assert_eq!(field.resolve(&doc).unwrap(), original);
            field.text = "25".into();
            let edited = field.resolve(&doc).unwrap();
            assert_eq!(
                edited,
                Dimension::Literal {
                    value: 25.,
                    unit: target.into()
                }
            );
            field.remember(&edited);
            assert_eq!(field.resolve(&doc).unwrap(), edited);
        }
    }
    #[test]
    fn formula_replacement_remembers_stable_binding_and_still_rejects_deleted_parameters() {
        let mut doc = Document::new("Formula baseline").unwrap();
        let mut parameter = Entity::new(
            "width",
            EntityData::Parameter {
                expression: "2".into(),
                unit: "cm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        doc.insert(parameter.clone()).unwrap();
        let mut field = DimensionField::from_dimension(
            &Dimension::Literal {
                value: 1.,
                unit: "cm".into(),
            },
            "mm",
        )
        .unwrap();
        field.text = "=$width + 5".into();
        let bound = field.resolve(&doc).unwrap();
        field.remember(&bound);
        parameter.name = "renamed_width".into();
        doc.replace(parameter.clone()).unwrap();
        assert_eq!(field.resolve(&doc).unwrap(), bound);
        doc.remove(parameter.id).unwrap();
        assert!(field.resolve(&doc).is_err());
    }
}
