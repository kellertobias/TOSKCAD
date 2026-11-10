//! Certify a full circle chosen as the master pcurve of a planar rim.

use super::*;
use std::f64::consts::PI;

#[cfg(test)]
mod tests;

fn lift(plane: &Plane, [u, v, _]: Point) -> Point {
    std::array::from_fn(|i| plane.origin[i] + u * plane.u[i] + v * plane.v[i])
}

impl Shell<'_, '_> {
    pub(super) fn verify_planar_circle_master(
        &self,
        curve: u32,
        surface: u32,
        pcurve: u32,
    ) -> Result<()> {
        let [Value::String(_), axis] = self.args(surface, "PLANE")? else {
            return Err(ReadError::Invalid);
        };
        let plane = self.placement(reference(axis)?)?;
        let [Value::String(_), axis, Value::Number(radius)] = self.args(curve, "CIRCLE")? else {
            return Err(ReadError::Invalid);
        };
        let actual = self.placement(reference(axis)?)?;
        if *radius <= self.tolerance {
            return Err(ReadError::Invalid);
        }
        match self.components(pcurve)? {
            [component] if component.name == "CIRCLE" => {
                let [Value::String(_), axis, Value::Number(rad)] = component.args.as_slice() else {
                    return Err(ReadError::Invalid);
                };
                let [Value::String(_), center, reference_axis] =
                    self.args(reference(axis)?, "AXIS2_PLACEMENT_2D")?
                else {
                    return Err(ReadError::Invalid);
                };
                let center = lift(
                    &plane,
                    self.parameter_pair(reference(center)?, "CARTESIAN_POINT")?,
                );
                let [u, v, _] = match reference_axis {
                    Value::Omitted => [1., 0., 0.],
                    value => unit(self.parameter_pair(reference(value)?, "DIRECTION")?)?,
                };
                let radial = std::array::from_fn(|i| u * plane.u[i] + v * plane.v[i]);
                let tangent = std::array::from_fn(|i| -v * plane.u[i] + u * plane.v[i]);
                let error = length(sub(center, actual.origin))
                    + (rad - radius).abs()
                    + rad * (length(sub(radial, actual.u)) + length(sub(tangent, actual.v)));
                if *rad <= 0. || error > self.tolerance {
                    return Err(ReadError::Invalid);
                }
                Ok(())
            }
            components => self.verify_rational_circle(components, &plane, &actual, *radius),
        }
    }

    fn verify_rational_circle(
        &self,
        components: &[Entity<'_>],
        plane: &Plane,
        actual: &Plane,
        radius: f64,
    ) -> Result<()> {
        let args = |name| {
            components
                .iter()
                .find(|component| component.name == name)
                .map(|component| component.args.as_slice())
                .ok_or(ReadError::Unsupported)
        };
        let [
            Value::Number(2.),
            controls,
            Value::Enum("UNSPECIFIED"),
            Value::Enum("T"),
            Value::Enum("F"),
        ] = args("B_SPLINE_CURVE")?
        else {
            return Err(ReadError::Unsupported);
        };
        let [
            Value::List(multiplicities),
            Value::List(knots),
            Value::Enum("UNSPECIFIED" | "PIECEWISE_BEZIER_KNOTS"),
        ] = args("B_SPLINE_CURVE_WITH_KNOTS")?
        else {
            return Err(ReadError::Unsupported);
        };
        let [Value::List(weights)] = args("RATIONAL_B_SPLINE_CURVE")? else {
            return Err(ReadError::Unsupported);
        };
        let controls = refs(controls)?;
        let (
            [
                Value::Number(1.),
                active_multiplicities @ ..,
                Value::Number(1.),
            ],
            [
                Value::Number(before),
                active_knots @ ..,
                Value::Number(after),
            ],
        ) = (multiplicities.as_slice(), knots.as_slice())
        else {
            return Err(ReadError::Unsupported);
        };
        if !(4..=17).contains(&active_knots.len())
            || active_knots.len() != active_multiplicities.len()
            || controls.len() != 2 * active_knots.len() - 1
            || weights.len() != controls.len()
            || !active_multiplicities
                .iter()
                .all(|v| matches!(v, Value::Number(2.)))
        {
            return Err(ReadError::Unsupported);
        }
        let [Value::Number(first), .., Value::Number(last)] = active_knots else {
            return Err(ReadError::Invalid);
        };
        if before >= first
            || after <= last
            || radius * first.abs() > self.tolerance
            || radius * (last - TAU).abs() > self.tolerance
        {
            return Err(ReadError::Invalid);
        }
        // Double active knots on a quadratic curve make each span a
        // rational Bézier. Prove the exact circle construction on every
        // span, including the weights and the full positive parameter run.
        for (i, pair) in active_knots.windows(2).enumerate() {
            let [Value::Number(a), Value::Number(b)] = pair else {
                return Err(ReadError::Invalid);
            };
            let angle = b - a;
            if !(0. < angle && angle < PI) {
                return Err(ReadError::Invalid);
            }
            let start = 2 * i;
            let [Value::Number(1.), Value::Number(weight), Value::Number(1.)] =
                &weights[start..start + 3]
            else {
                return Err(ReadError::Unsupported);
            };
            if *weight <= 0. {
                return Err(ReadError::Invalid);
            }
            let ideal_weight = (angle / 2.).cos();
            let circle_point = |angle: f64, r| {
                let (sin, cos) = angle.sin_cos();
                lift(actual, [r * cos, r * sin, 0.])
            };
            let ideal = [
                circle_point(*a, radius),
                circle_point(a.midpoint(*b), radius / ideal_weight),
                circle_point(*b, radius),
            ];
            let mut error = 0_f64;
            for (&control, expected) in controls[start..start + 3].iter().zip(ideal) {
                let actual = lift(plane, self.parameter_pair(control, "CARTESIAN_POINT")?);
                error = error.max(length(sub(actual, expected)));
            }
            let diameter = length(sub(ideal[0], ideal[1])).max(length(sub(ideal[2], ideal[1])));
            // Positive weights keep the position perturbation within the
            // largest control residual. The remaining weight perturbation
            // is bounded by the control diameter over the minimum weight.
            error += diameter * (weight - ideal_weight).abs() / weight.min(1.);
            if error > self.tolerance {
                return Err(ReadError::Invalid);
            }
        }
        Ok(())
    }
}
