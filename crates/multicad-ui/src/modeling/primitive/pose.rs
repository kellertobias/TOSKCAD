use super::super::gizmo::{Interaction, PlacementViewport};
use crate::egui;
use multicad_core::{
    model::{NumericTransform, PrimitiveFrame},
    parameters::EvaluatedParameters,
};

#[derive(Default)]
pub(super) struct PoseGizmo {
    interaction: Interaction,
    start: Option<PrimitiveFrame>,
}

pub(super) struct PoseGesture {
    pub input_owned: bool,
    pub released: bool,
    pub changed: Option<PrimitiveFrame>,
}

impl PoseGizmo {
    pub(super) fn active(&self) -> bool {
        self.interaction.active()
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        view: &PlacementViewport<'_>,
        frame: &PrimitiveFrame,
        parameters: &EvaluatedParameters,
    ) -> Result<PoseGesture, String> {
        let source = NumericTransform {
            translation: frame.origin.clone(),
            ..Default::default()
        };
        let gesture = self.interaction.show_with_help(
            ui,
            view,
            &source,
            parameters,
            "drag to adjust the primitive; choose Apply or press Enter to confirm",
        )?;
        if gesture.started {
            self.start = Some(frame.clone());
        }
        let changed = gesture
            .changed
            .map(|transform| {
                // The shared gizmo reports a cumulative transform from mouse-down.
                let start = self
                    .start
                    .as_ref()
                    .ok_or("Primitive pose was not captured")?;
                changed_frame(start, transform, parameters)
            })
            .transpose();
        if gesture.released {
            self.start = None;
        }
        Ok(PoseGesture {
            input_owned: gesture.input_owned,
            released: gesture.released,
            changed: changed?,
        })
    }
}

fn changed_frame(
    start: &PrimitiveFrame,
    transform: NumericTransform,
    parameters: &EvaluatedParameters,
) -> Result<PrimitiveFrame, String> {
    let pose = transform.resolve(parameters).map_err(|e| e.to_string())?;
    let rotation_only = pose.rotation != [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let frame = PrimitiveFrame {
        // For rotation the origin is the pivot. Keep its authored dimensions
        // even if the shared transform's pivot compensation had roundoff.
        origin: if rotation_only {
            start.origin.clone()
        } else {
            transform.translation
        },
        u: pose.vector(start.u),
        v: pose.vector(start.v),
    };
    frame.validate().map_err(|e| e.to_string())?;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::gizmo::math::{Values, rotated, translated};
    use multicad_core::{
        constraint_sketch::Dimension,
        document::{Document, Entity, EntityData},
        model,
        parameters::{BoundExpression, evaluate},
    };

    fn placed_frame() -> (PrimitiveFrame, EvaluatedParameters) {
        let mut document = Document::new("Primitive pose").unwrap();
        document
            .insert(Entity::new(
                "offset",
                EntityData::Parameter {
                    expression: "75".into(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            ))
            .unwrap();
        let origin = Dimension::Expression {
            expression: BoundExpression::bind(&document, "=$offset + 5", "mm", None).unwrap(),
        };
        (
            PrimitiveFrame {
                origin: [origin.clone(), origin, model::length(20.)],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            evaluate(&document).unwrap(),
        )
    }

    #[test]
    fn moving_one_axis_preserves_unmoved_formula_and_basis() {
        let (start, parameters) = placed_frame();
        let source = NumericTransform {
            translation: start.origin.clone(),
            ..Default::default()
        };
        let values = Values::read(&source, &parameters).unwrap();
        let moved = translated(&source, values, [12., 0., 0.]).unwrap();
        let frame = changed_frame(&start, moved, &parameters).unwrap();
        assert_eq!(frame.origin[0], model::length(92.));
        assert_eq!(frame.origin[1..], start.origin[1..]);
        assert_eq!([frame.u, frame.v], [start.u, start.v]);
    }

    #[test]
    fn rotating_an_oriented_frame_preserves_origin_and_uses_each_cumulative_delta_once() {
        let (start, parameters) = placed_frame();
        let source = NumericTransform {
            translation: start.origin.clone(),
            ..Default::default()
        };
        let values = Values::read(&source, &parameters).unwrap();
        let around_x = |degrees| {
            let delta = rotated(&source, values, [1., 0., 0.], degrees, &parameters).unwrap();
            changed_frame(&start, delta, &parameters).unwrap()
        };
        let halfway = around_x(45.);
        assert_eq!(halfway.origin, start.origin);
        assert!((halfway.u[1] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        let right_angle = around_x(90.);
        assert_eq!(right_angle.origin, start.origin);
        assert!(right_angle.u[1].abs() < 1e-12 && (right_angle.u[2] - 1.).abs() < 1e-12);
        assert!((right_angle.v[1] + 1.).abs() < 1e-12 && right_angle.v[2].abs() < 1e-12);
        assert_eq!(around_x(0.), start);
    }
}
