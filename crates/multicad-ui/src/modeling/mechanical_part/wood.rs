//! Wood intent is independent of the machine-thread recipe and its controls.
use super::super::dimension::DimensionField;
use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityId},
    model::*,
};

pub(super) struct WoodForm {
    convention: ScrewLengthConvention,
    fields: [DimensionField; 8],
    source: WoodScrewParameters,
}
impl WoodForm {
    pub fn new(source: &WoodScrewParameters) -> Result<Self, String> {
        let ScrewHead::ExternalHex {
            across_flats,
            height,
        } = &source.head
        else {
            return Err("Wood screw requires an external-hex head".into());
        };
        let dimensions = [
            source.length_input.value(),
            &source.nominal_diameter,
            across_flats,
            height,
            &source.reference_profile.pitch,
            &source.reference_profile.core_diameter,
            &source.reference_profile.included_angle,
            &source.minimum_tip_inclusive_thread_extent,
        ];
        let fields = dimensions
            .into_iter()
            .enumerate()
            .map(|(i, d)| DimensionField::from_dimension(d, if i == 6 { "deg" } else { "mm" }))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "Invalid wood fields")?;
        Ok(Self {
            convention: source.length_input.convention(),
            fields,
            source: source.clone(),
        })
    }
    pub fn resolve(
        &self,
        document: &Document,
        frame: PrimitiveFrame,
    ) -> Result<MechanicalPartParameters, String> {
        let mut source = self.source.clone();
        let d = |i: usize| self.fields[i].resolve(document);
        source.length_input = match self.convention {
            ScrewLengthConvention::Underhead => ScrewLengthInput::Underhead { value: d(0)? },
            ScrewLengthConvention::Overall => ScrewLengthInput::Overall { value: d(0)? },
        };
        source.nominal_diameter = d(1)?;
        source.head = ScrewHead::ExternalHex {
            across_flats: d(2)?,
            height: d(3)?,
        };
        source.reference_profile = WoodVReference {
            pitch: d(4)?,
            core_diameter: d(5)?,
            included_angle: d(6)?,
        };
        source.minimum_tip_inclusive_thread_extent = d(7)?;
        source.frame = frame;
        Ok(MechanicalPartParameters::WoodScrew {
            parameters: Box::new(source),
        })
    }
    pub fn show(&mut self, ui: &mut egui::Ui, feature: EntityId) -> bool {
        let mut changed = false;
        ui.strong("Simplified wood-screw envelope");
        ui.small("External hex head and drive. Smooth stock envelope; thread and manufacturer point are not modeled. Manufacturing export is unavailable.");
        controls::inspector_combo(ui, (feature, "wood_length_convention"))
            .selected_text(match self.convention {
                ScrewLengthConvention::Underhead => "Underhead length",
                ScrewLengthConvention::Overall => "Overall length",
            })
            .show_ui(ui, |ui| {
                for (value, label) in [
                    (ScrewLengthConvention::Underhead, "Underhead length"),
                    (ScrewLengthConvention::Overall, "Overall length"),
                ] {
                    changed |= ui
                        .selectable_value(&mut self.convention, value, label)
                        .changed();
                }
            });
        for (i, label) in [
            "Length",
            "Nominal stock diameter",
            "Head across flats",
            "Head height",
            "Reference pitch",
            "Reference core diameter",
            "Reference V angle",
            "Minimum thread extent including point",
        ]
        .into_iter()
        .enumerate()
        {
            changed |= self.fields[i].show_named(ui,(feature,"wood",i),label,"Retained value or bound expression; reference intent does not establish modeled manufacturing geometry.");
        }
        ui.small("The minimum extent includes the unspecified point and thread-start transition. It is a lower bound, not the actual thread length or start. The flat distal plane encloses the unknown point. Changing length convention retains the authored value.");
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::Session, constraint_sketch::Dimension, parameters::BoundExpression,
    };
    #[test]
    fn retained_wood_expression_and_reference_intent_survive_reopening() {
        let session = Session::new(Document::new("Wood form").unwrap()).unwrap();
        let MechanicalPartParameters::WoodScrew { mut parameters } = default_wood_screw() else {
            unreachable!()
        };
        let expression = Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=44", "mm", None).unwrap(),
        };
        parameters.length_input = ScrewLengthInput::Overall {
            value: expression.clone(),
        };
        let mut form = WoodForm::new(&parameters).unwrap();
        assert_eq!(
            form.resolve(session.document(), parameters.frame.clone())
                .unwrap(),
            MechanicalPartParameters::WoodScrew {
                parameters: parameters.clone()
            }
        );
        form.fields[0].text = "=".into();
        assert!(
            form.resolve(session.document(), parameters.frame.clone())
                .is_err()
        );
        form.fields[0].text = "=44".into();
        form.fields[2].text = "11".into();
        let MechanicalPartParameters::WoodScrew { parameters: edited } = form
            .resolve(session.document(), parameters.frame.clone())
            .unwrap()
        else {
            unreachable!()
        };
        assert_eq!(
            edited.length_input,
            ScrewLengthInput::Overall { value: expression }
        );
        assert_eq!(edited.reference_profile, parameters.reference_profile);
        assert_eq!(edited.point, parameters.point);
        assert_eq!(
            edited.representation,
            WoodScrewRepresentation::SimplifiedEnvelope
        );
        assert_ne!(edited.head, parameters.head);
    }
    #[test]
    fn wood_controls_never_offer_machine_fit_or_modeled_thread() {
        let MechanicalPartParameters::WoodScrew { parameters } = default_wood_screw() else {
            unreachable!()
        };
        let mut form = WoodForm::new(&parameters).unwrap();
        let context = egui::Context::default();
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(440., 1100.),
                )),
                ..Default::default()
            },
            |ui| {
                form.show(ui, EntityId::new());
            },
        );
        fn collect(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(t) => {
                    text.push_str(t.galley.text());
                    text.push('\n');
                }
                egui::Shape::Vec(shapes) => {
                    for s in shapes {
                        collect(s, text)
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for shape in &output.shapes {
            collect(&shape.shape, &mut text);
        }
        output.drop_without_applying_deltas();
        assert!(text.contains("Simplified wood-screw envelope"));
        assert!(text.contains("Manufacturing export is unavailable"));
        for absent in ["Metric fit", "Modeled helix", "Thread hand", "Starts"] {
            assert!(!text.contains(absent), "{absent}");
        }
    }
}
