use multicad_core::{
    constraint_sketch::Dimension,
    model::{self, NumericTransform},
    parameters::{EvaluatedParameters, Unit},
};
pub(crate) type Point = [f64; 3];
pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
pub(crate) fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(crate) fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn mul(a: Point, b: f64) -> Point {
    a.map(|a| a * b)
}
pub(crate) fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn number(d: &Dimension, unit: Unit, parameters: &EvaluatedParameters) -> Result<f64, String> {
    let value = match d {
        Dimension::Literal {
            value,
            unit: source,
        } => {
            let source = Unit::parse(source).ok_or("Unknown transform unit")?;
            if source.dimension() != unit.dimension() {
                return Err("Incompatible transform unit".into());
            }
            value * source.scale() / unit.scale()
        }
        Dimension::Expression { expression } => expression
            .evaluate(parameters)
            .map_err(|e| e.to_string())?
            .in_unit(unit)
            .ok_or("Incompatible transform formula")?,
    };
    if !value.is_finite() {
        return Err("Nonfinite transform control".into());
    }
    Ok(value)
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Values {
    pub translation: Point,
    pub pivot: Point,
    pub axis: Point,
    pub angle_deg: f64,
}
impl Values {
    pub fn read(
        source: &NumericTransform,
        parameters: &EvaluatedParameters,
    ) -> Result<Self, String> {
        source.validate().map_err(|e| e.to_string())?;
        let three = |values: &[Dimension; 3]| -> Result<Point, String> {
            Ok([
                number(&values[0], Unit::Millimeter, parameters)?,
                number(&values[1], Unit::Millimeter, parameters)?,
                number(&values[2], Unit::Millimeter, parameters)?,
            ])
        };
        Ok(Self {
            translation: three(&source.translation)?,
            pivot: three(&source.pivot)?,
            axis: source.axis,
            angle_deg: number(&source.angle, Unit::Degree, parameters)?,
        })
    }
    pub fn origin(self) -> Point {
        add(self.pivot, self.translation)
    }
}
pub(crate) fn translated(
    source: &NumericTransform,
    values: Values,
    delta: Point,
) -> Result<NumericTransform, String> {
    let mut result = source.clone();
    for (i, delta) in delta.into_iter().enumerate() {
        if delta.abs() > 1e-10 {
            result.translation[i] = model::length(values.translation[i] + delta);
        }
    }
    result.validate().map_err(|e| e.to_string())?;
    Ok(result)
}
fn axis_angle(rotation: [[f64; 3]; 3]) -> Result<(Point, f64), String> {
    let trace = rotation[0][0] + rotation[1][1] + rotation[2][2];
    let choices = [
        1. + 2. * rotation[0][0] - trace,
        1. + 2. * rotation[1][1] - trace,
        1. + 2. * rotation[2][2] - trace,
        1. + trace,
    ];
    let k = (0..4)
        .max_by(|a, b| choices[*a].total_cmp(&choices[*b]))
        .unwrap();
    let mut q = [0.; 4];
    q[k] = choices[k].max(0.).sqrt() * 0.5;
    let denominator = 4. * q[k];
    if denominator < 1e-12 {
        return Err("Rotation has no stable axis representation".into());
    }
    match k {
        3 => {
            q[0] = (rotation[2][1] - rotation[1][2]) / denominator;
            q[1] = (rotation[0][2] - rotation[2][0]) / denominator;
            q[2] = (rotation[1][0] - rotation[0][1]) / denominator;
        }
        0 => {
            q[1] = (rotation[1][0] + rotation[0][1]) / denominator;
            q[2] = (rotation[2][0] + rotation[0][2]) / denominator;
            q[3] = (rotation[2][1] - rotation[1][2]) / denominator;
        }
        1 => {
            q[0] = (rotation[1][0] + rotation[0][1]) / denominator;
            q[2] = (rotation[2][1] + rotation[1][2]) / denominator;
            q[3] = (rotation[0][2] - rotation[2][0]) / denominator;
        }
        _ => {
            q[0] = (rotation[2][0] + rotation[0][2]) / denominator;
            q[1] = (rotation[2][1] + rotation[1][2]) / denominator;
            q[3] = (rotation[1][0] - rotation[0][1]) / denominator;
        }
    }
    let norm = dot([q[0], q[1], q[2]], [q[0], q[1], q[2]]) + q[3] * q[3];
    if !norm.is_finite() || norm < 1e-12 {
        return Err("Invalid rotation representation".into());
    }
    let sign = if q[3] < 0. { -1. } else { 1. };
    q = q.map(|x| x / norm.sqrt() * sign);
    let sine = dot([q[0], q[1], q[2]], [q[0], q[1], q[2]]).sqrt();
    if sine < 1e-12 {
        return Ok(([0., 0., 1.], 0.));
    }
    Ok((
        [q[0] / sine, q[1] / sine, q[2] / sine],
        (2. * sine.atan2(q[3])).to_degrees(),
    ))
}
pub(crate) fn rotated(
    source: &NumericTransform,
    values: Values,
    axis: Point,
    angle_deg: f64,
    parameters: &EvaluatedParameters,
) -> Result<NumericTransform, String> {
    if angle_deg.abs() < 1e-10 {
        return Ok(source.clone());
    }
    let axis = mul(axis, 1. / dot(axis, axis).sqrt());
    let source_axis = mul(values.axis, 1. / dot(values.axis, values.axis).sqrt());
    let mut result = source.clone();
    if dot(axis, source_axis).abs() > 1. - 1e-12 {
        result.angle = model::angle(values.angle_deg + angle_deg * dot(axis, source_axis).signum());
    } else {
        let delta = NumericTransform {
            pivot: values.origin().map(model::length),
            axis,
            angle: model::angle(angle_deg),
            ..Default::default()
        }
        .resolve(parameters)
        .map_err(|e| e.to_string())?;
        let desired = source
            .resolve(parameters)
            .and_then(|p| p.then(delta))
            .map_err(|e| e.to_string())?;
        let (axis, angle) = axis_angle(desired.rotation)?;
        result.axis = axis;
        result.angle = model::angle(angle);
        let rotated = desired.vector(values.pivot);
        for (i, rotated) in rotated.into_iter().enumerate() {
            let translation = desired.translation_mm[i] - values.pivot[i] + rotated;
            if (translation - values.translation[i]).abs() > 1e-10 {
                result.translation[i] = model::length(translation);
            }
        }
    }
    result.validate().map_err(|e| e.to_string())?;
    Ok(result)
}
pub(crate) fn axis_parameter(ray: crate::viewport::Ray, origin: Point, axis: Point) -> Option<f64> {
    let direction = ray.direction();
    let w = sub(ray.origin(), origin);
    let parallel = dot(axis, direction);
    let denominator = 1. - parallel * parallel;
    if denominator.abs() < 1e-7 {
        return None;
    }
    let parameter = (dot(axis, w) - parallel * dot(direction, w)) / denominator;
    parameter.is_finite().then_some(parameter)
}
pub(crate) fn plane_point(
    ray: crate::viewport::Ray,
    origin: Point,
    normal: Point,
) -> Option<Point> {
    let denominator = dot(ray.direction(), normal);
    if denominator.abs() < 1e-8 {
        return None;
    }
    let t = dot(sub(origin, ray.origin()), normal) / denominator;
    (t.is_finite() && t >= 0.).then(|| add(ray.origin(), mul(ray.direction(), t)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::model::RigidPose;
    fn parameters() -> EvaluatedParameters {
        multicad_core::parameters::evaluate(
            &multicad_core::document::Document::new("Gizmo math").unwrap(),
        )
        .unwrap()
    }
    fn close(a: RigidPose, b: RigidPose) {
        for (a, b) in a
            .rotation
            .into_iter()
            .flatten()
            .chain(a.translation_mm)
            .zip(b.rotation.into_iter().flatten().chain(b.translation_mm))
        {
            assert!((a - b).abs() < 1e-9, "{a} != {b}");
        }
    }
    #[test]
    fn mixed_axis_rotation_composes_about_the_displayed_translated_pivot_without_source_changes() {
        let p = parameters();
        let source = NumericTransform {
            translation: [model::length(8.), model::length(-4.), model::length(2.)],
            pivot: [model::length(3.), model::length(5.), model::length(-7.)],
            axis: [0., 0., 1.],
            angle: model::angle(37.),
        };
        for axis in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [-1., 0., 0.]] {
            for angle in [-180., -90., 17., 90., 180., 270.] {
                let values = Values::read(&source, &p).unwrap();
                let output = rotated(&source, values, axis, angle, &p).unwrap();
                let delta = NumericTransform {
                    pivot: values.origin().map(model::length),
                    axis,
                    angle: model::angle(angle),
                    ..Default::default()
                }
                .resolve(&p)
                .unwrap();
                close(
                    output.resolve(&p).unwrap(),
                    source.resolve(&p).unwrap().then(delta).unwrap(),
                );
            }
        }
    }
    #[test]
    fn quaternion_representation_retains_exact_cardinal_and_half_turn_poses() {
        let p = parameters();
        for axis in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 2., 3.]] {
            for angle in [0., 90., 180., 270., 359.999] {
                let pose = NumericTransform {
                    axis,
                    angle: model::angle(angle),
                    ..Default::default()
                }
                .resolve(&p)
                .unwrap();
                let (a, d) = axis_angle(pose.rotation).unwrap();
                close(
                    pose,
                    NumericTransform {
                        axis: a,
                        angle: model::angle(d),
                        ..Default::default()
                    }
                    .resolve(&p)
                    .unwrap(),
                );
            }
        }
    }
    #[test]
    fn pointer_axis_and_plane_solutions_are_finite_and_reject_edge_on_degeneracy() {
        let ray = crate::viewport::Ray::new([4., 5., 20.], [0., 0., -1.]).unwrap();
        assert_eq!(axis_parameter(ray, [0.; 3], [1., 0., 0.]), Some(4.));
        assert_eq!(plane_point(ray, [0.; 3], [0., 0., 1.]), Some([4., 5., 0.]));
        assert!(axis_parameter(ray, [0.; 3], [0., 0., 1.]).is_none());
        assert!(plane_point(ray, [0.; 3], [1., 0., 0.]).is_none());
    }
}
