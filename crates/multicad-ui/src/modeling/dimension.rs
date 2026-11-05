use crate::egui;
use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    parameters::{BoundExpression, LengthUnit, Unit, parse_length},
};
/// Up to six decimals without trailing zeros.
fn trimmed(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

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
    /// `unit` is the field's canonical unit. Millimetre fields show and read
    /// unqualified input in the Settings length unit (CAD-187); typed values
    /// are stored with that explicit unit, so the preference never changes a
    /// stored dimension.
    pub fn from_dimension(value: &Dimension, unit: &'static str) -> Result<Self, String> {
        let unit = if unit == "mm" {
            crate::preferences::length_unit().symbol()
        } else {
            unit
        };
        let source = Unit::parse(value.unit()).ok_or("Unknown dimension unit")?;
        let target = Unit::parse(unit).ok_or("Unknown field unit")?;
        if source.dimension() != target.dimension() {
            return Err("Incompatible dimension unit".into());
        }
        let text = match value {
            Dimension::Literal { value, .. } if source == target => value.to_string(),
            // A converted literal shows a rounded value; left unchanged it
            // still resolves to the exact authored dimension.
            Dimension::Literal { value, .. } => trimmed(value * source.scale() / target.scale()),
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
    /// Keep an exact authored pick while displaying it in this field's unit.
    /// Unedited rounded display text resolves to the original canonical value,
    /// just as it does when reopening a stored literal.
    pub fn set_exact_literal(&mut self, value: f64, unit: &str) {
        let (Some(from), Some(to)) = (Unit::parse(unit), Unit::parse(&self.unit)) else {
            return;
        };
        if from.dimension() != to.dimension() {
            return;
        }
        self.text = trimmed(value * from.scale() / to.scale());
        self.original_text = self.text.clone();
        self.original = Dimension::Literal {
            value,
            unit: unit.into(),
        };
        self.previous = None;
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
    /// Stable identity for forms whose validation or preview messages can insert
    /// widgets before an active edit. Other tools keep the existing auto-ID path.
    pub fn show_named(
        &mut self,
        ui: &mut egui::Ui,
        id_salt: impl egui::AsIdSalt,
        label: &str,
        help: &str,
    ) -> bool {
        let id = ui.make_persistent_id(id_salt);
        ui.label(format!("{label} ({})", self.unit));
        ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .id(id)
                .desired_width(crate::controls::inspector_width(ui))
                .hint_text(format!("Value or =$parameter · {}", self.unit)),
        )
        .on_hover_text(help)
        .changed()
    }
    /// Two-column Inspector grids reserve room for their label instead of
    /// requesting the entire row for the value. The caller supplies an
    /// owner/field identity, independent of validation text or preceding widgets.
    pub fn show_compact(
        &mut self,
        ui: &mut egui::Ui,
        id_salt: impl egui::AsIdSalt,
        label: &str,
        help: &str,
    ) -> bool {
        let row_width = ui.max_rect().width().min(ui.clip_rect().width()).max(0.);
        let width = (row_width * 0.4).min(120.);
        let label_width = (row_width - width - ui.spacing().item_spacing.x - 4.).clamp(0., 160.);
        ui.add_sized(
            [label_width, 0.],
            egui::Label::new(format!("{label} ({})", self.unit)).wrap(),
        );
        let id = ui.make_persistent_id(id_salt);
        ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .id(id)
                .desired_width(width)
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
        let (value, unit) = match self.text.trim().parse::<f64>() {
            Ok(value) => (value, self.unit.clone()),
            // Explicitly qualified lengths such as `1 in` keep their own unit.
            Err(_) => Unit::parse(&self.unit)
                .filter(|unit| unit.dimension() == multicad_core::parameters::Dimension::LENGTH)
                .and_then(|_| parse_length(&self.text, LengthUnit::Millimeter).ok())
                .map(|input| (input.value, input.unit.symbol().to_string()))
                .ok_or_else(|| {
                    "Enter a number, a length such as 1 in, or an expression beginning with '='"
                        .to_string()
                })?,
        };
        if !value.is_finite() || value.abs() > 1e9 {
            return Err("Value must be finite and no greater than 1,000,000,000".into());
        }
        if self.text.trim() == self.original_text.trim()
            && matches!(self.original, Dimension::Literal { .. })
        {
            return Ok(self.original.clone());
        }
        Ok(Dimension::Literal { value, unit })
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
    #[test]
    fn length_fields_follow_the_settings_unit_without_changing_stored_dimensions() {
        use crate::preferences::{LengthUnit, set_length_unit};
        let doc = Document::new("Inch fields").unwrap();
        let authored = Dimension::Literal {
            value: 25.4,
            unit: "mm".into(),
        };
        set_length_unit(LengthUnit::Inch);
        let mut field = DimensionField::from_dimension(&authored, "mm").unwrap();
        assert_eq!(field.text, "1");
        assert_eq!(field.unit, "in");
        // Untouched, the authored millimetre literal is kept exactly.
        assert_eq!(field.resolve(&doc).unwrap(), authored);
        // Unqualified input is inches; qualified input keeps its unit.
        field.text = "2".into();
        assert_eq!(
            field.resolve(&doc).unwrap(),
            Dimension::Literal {
                value: 2.,
                unit: "in".into()
            }
        );
        field.text = "30 mm".into();
        assert_eq!(
            field.resolve(&doc).unwrap(),
            Dimension::Literal {
                value: 30.,
                unit: "mm".into()
            }
        );
        field.text = "5 deg".into();
        assert!(field.resolve(&doc).is_err());
        // Angles and counts are unaffected by the length preference.
        let angle = DimensionField::from_dimension(
            &Dimension::Literal {
                value: 90.,
                unit: "deg".into(),
            },
            "deg",
        )
        .unwrap();
        assert_eq!(angle.unit, "deg");
        // An inch literal shown in millimetres stays inches until edited.
        set_length_unit(LengthUnit::Millimeter);
        let inch = Dimension::Literal {
            value: 1.,
            unit: "in".into(),
        };
        let field = DimensionField::from_dimension(&inch, "mm").unwrap();
        assert_eq!((field.text.as_str(), field.unit.as_str()), ("25.4", "mm"));
        assert_eq!(field.resolve(&doc).unwrap(), inch);
        assert_eq!(
            inch.value_in(Unit::Millimeter, None).unwrap(),
            authored.value_in(Unit::Millimeter, None).unwrap()
        );
    }
}
