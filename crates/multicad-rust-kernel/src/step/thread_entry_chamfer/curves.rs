//! One finite polynomial curve shared by its two surface parameter charts.
use super::*;

pub(super) struct UvContext(Entity);
impl UvContext {
    pub fn new(file: &mut StepFile) -> Self {
        Self(file.entity(format_args!("(GEOMETRIC_REPRESENTATION_CONTEXT(2)PARAMETRIC_REPRESENTATION_CONTEXT()REPRESENTATION_CONTEXT('2D SPACE',''))")))
    }
    pub fn spline(
        &self,
        file: &mut StepFile,
        surface: Entity,
        degree: usize,
        knots: &[f64],
        poles: &[[f64; 2]],
    ) -> Result<Entity, Error> {
        if poles.iter().flatten().any(|value| !value.is_finite()) {
            return Err(Error::InvalidGeometry);
        }
        let points = poles
            .iter()
            .map(|[x, y]| file.entity(format_args!("CARTESIAN_POINT('',({x:.16E},{y:.16E}))")))
            .collect::<Vec<_>>();
        let curve = record(file, degree, knots, &points)?;
        let context = self.0;
        let representation = file.entity(format_args!(
            "DEFINITIONAL_REPRESENTATION('',({curve}),{context})"
        ));
        Ok(file.entity(format_args!("PCURVE('',{surface},{representation})")))
    }
    pub fn line(
        &self,
        file: &mut StepFile,
        surface: Entity,
        domain: [f64; 2],
        ends: [[f64; 2]; 2],
    ) -> Result<Entity, Error> {
        self.spline(file, surface, 1, &domain, &ends)
    }
}

pub(super) fn record(
    file: &mut StepFile,
    degree: usize,
    knots: &[f64],
    points: &[Entity],
) -> Result<Entity, Error> {
    if knots.len() > crate::step_read::MAX_STEP_ENTITIES / (helical::DEGREE + 1)
        || points.len() > crate::step_read::MAX_STEP_ENTITIES
    {
        return Err(Error::Limit);
    }
    if !(1..=helical::DEGREE).contains(&degree)
        || knots.len() < 2
        || points.len() != degree * (knots.len() - 1) + 1
        || knots.iter().any(|value| !value.is_finite())
        || knots.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(Error::InvalidGeometry);
    }
    let multiplicities = (0..knots.len())
        .map(|i| {
            if i == 0 || i + 1 == knots.len() {
                degree + 1
            } else {
                degree
            }
            .to_string()
        })
        .collect::<Vec<_>>()
        .join(",");
    let values = knots
        .iter()
        .map(|value| format!("{value:.16E}"))
        .collect::<Vec<_>>()
        .join(",");
    Ok(file.entity(format_args!("B_SPLINE_CURVE_WITH_KNOTS('',{degree},{},.UNSPECIFIED.,.F.,.F.,({multiplicities}),({values}),.UNSPECIFIED.)",Entities(points))))
}

pub(super) fn edge(
    file: &mut StepFile,
    vertices: [Vertex; 2],
    curve: Entity,
    pcurves: &[Entity],
    seam: bool,
    same_sense: bool,
) -> Entity {
    let geometry = if pcurves.is_empty() {
        curve
    } else {
        let kind = if seam { "SEAM_CURVE" } else { "SURFACE_CURVE" };
        let master = if seam { "PCURVE_S1" } else { "CURVE_3D" };
        file.entity(format_args!(
            "{kind}('',{curve},{},.{master}.)",
            Entities(pcurves)
        ))
    };
    let [from, to] = vertices.map(|vertex| vertex.vertex);
    file.entity(format_args!(
        "EDGE_CURVE('',{from},{to},{geometry},{})",
        if same_sense { ".T." } else { ".F." }
    ))
}

pub(super) fn spline(
    file: &mut StepFile,
    spline: &helical::Spline,
    vertices: [Vertex; 2],
    points: impl Iterator<Item = Point>,
) -> Result<Entity, Error> {
    let mut controls = Vec::with_capacity(spline.poles.len());
    for (i, point) in points.enumerate() {
        controls.push(if i == 0 {
            vertices[0].cartesian
        } else if i + 1 == spline.poles.len() {
            vertices[1].cartesian
        } else {
            file.point(point)
        });
    }
    record(file, helical::DEGREE, &spline.knots, &controls)
}

pub(super) fn cubic(
    file: &mut StepFile,
    placement: &Placement,
    trims: &[Trim],
) -> Result<Entity, Error> {
    let mut points = Vec::with_capacity(3 * trims.len() + 1);
    let mut knots = Vec::with_capacity(trims.len() + 1);
    for (i, trim) in trims.iter().enumerate() {
        if i == 0 {
            knots.push(trim.phase[0]);
        }
        knots.push(trim.phase[1]);
        for point in trim.poles.iter().skip(usize::from(i != 0)) {
            points.push(file.point(placement.point_f64([point[0], point[1]], point[2])));
        }
    }
    record(file, 3, &knots, &points)
}

pub(super) fn cubic_uv(
    trims: &[Trim],
    component: impl Fn(&Trim, usize) -> [f64; 2],
) -> (Vec<f64>, Vec<[f64; 2]>) {
    let mut points = Vec::with_capacity(3 * trims.len() + 1);
    let mut knots = Vec::with_capacity(trims.len() + 1);
    for (i, trim) in trims.iter().enumerate() {
        if i == 0 {
            knots.push(trim.phase[0]);
        }
        knots.push(trim.phase[1]);
        for pole in usize::from(i != 0)..4 {
            points.push(component(trim, pole));
        }
    }
    (knots, points)
}

/// Axial controls over the actual spline domain, including a positive start.
/// The legacy helical helper intentionally assumes its first knot is zero.
pub(super) struct Heights {
    pub values: Vec<f64>,
    pub error_mm: f64,
}
pub(super) fn affine_heights(
    spline: &helical::Spline,
    ends: [f64; 2],
    slope: f64,
) -> Result<Heights, Error> {
    use crate::thread_entry_chamfer::trim::I;
    let Some((&a, tail)) = spline.knots.split_first() else {
        return Err(Error::InvalidGeometry);
    };
    let b = tail.last().copied().unwrap_or(a);
    if !a.is_finite()
        || !b.is_finite()
        || a >= b
        || !slope.is_finite()
        || slope <= 0.
        || ends.into_iter().any(|x| !x.is_finite())
        || ends[0] >= ends[1]
    {
        return Err(Error::InvalidGeometry);
    }
    let ia = I::point(a);
    let ib = I::point(b);
    let dz = I::point(ends[1]) - I::point(ends[0]);
    let mut error = 0_f64;
    let mut values = vec![ends[0]];
    for pair in spline.knots.windows(2) {
        for i in 1..=helical::DEGREE {
            let parameter = if i == helical::DEGREE {
                I::point(pair[1])
            } else {
                I::point(pair[0])
                    + (I::point(pair[1]) - I::point(pair[0])) * I::point(i as f64)
                        / I::point(helical::DEGREE as f64)
            };
            let exact = I::point(ends[0]) + dz * (parameter - ia) / (ib - ia);
            let height = if i == helical::DEGREE && pair[1] == b {
                ends[1]
            } else {
                exact.lo.midpoint(exact.hi)
            };
            error = error.max((exact - I::point(height)).checked()?.magnitude());
            values.push(height);
        }
    }
    let endpoint_error = [a, b].into_iter().zip(ends).fold(0_f64, |bound, (t, z)| {
        bound.max((I::point(t) * I::point(slope) - I::point(z)).magnitude())
    });
    let error_mm = (I::point(error) + I::point(endpoint_error)).checked()?.hi;
    Ok(Heights { values, error_mm })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evaluate(mut values: Vec<f64>, t: f64) -> f64 {
        for n in (1..values.len()).rev() {
            for i in 0..n {
                values[i] = (1. - t) * values[i] + t * values[i + 1];
            }
        }
        values[0]
    }
    #[test]
    fn positive_domain_rails_match_support_pcurve_over_every_bezier_span() {
        let h = 0.5 / std::f64::consts::TAU;
        let z = 0.03;
        let f = 3.3;
        let curve = helical::Spline::new(z / h, f / h, [1.45; 2], 1., 1e-10).unwrap();
        let heights = affine_heights(&curve, [z, f], h).unwrap();
        assert_eq!(heights.values[0], z);
        assert_eq!(*heights.values.last().unwrap(), f);
        assert!(heights.error_mm < 1e-12);
        let wrong = curve.linear_poles(f - z);
        let mut caught_old_map = false;
        for (span, pair) in curve.knots.windows(2).enumerate() {
            let i = span * helical::DEGREE;
            let controls = &heights.values[i..=i + helical::DEGREE];
            let old = &wrong[i..=i + helical::DEGREE];
            for j in 0..=16 {
                let t = j as f64 / 16.;
                let parameter = (1. - t) * pair[0] + t * pair[1];
                let expected = h * parameter;
                assert!(
                    (evaluate(controls.to_vec(), t) - expected).abs() <= heights.error_mm + 3e-15
                );
                caught_old_map |= (z + evaluate(old.to_vec(), t) - expected).abs() > 0.02;
            }
        }
        assert!(
            caught_old_map,
            "the real nonzero-domain counterexample must remain discriminating"
        );
        let malformed = helical::Spline {
            knots: vec![],
            poles: vec![],
        };
        assert!(affine_heights(&malformed, [0., 1.], 1.).is_err());
        assert!(affine_heights(&curve, [1., 0.], h).is_err());
        assert!(affine_heights(&curve, [z, f], f64::NAN).is_err());
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    #[test]
    fn finite_parameter_records_check_their_own_limits_before_emitting_geometry() {
        let mut file = StepFile::new();
        let uv = UvContext::new(&mut file);
        assert!(matches!(
            uv.spline(
                &mut file,
                Entity(1),
                1,
                &[0., 1.],
                &[[0., 0.], [f64::NAN, 1.]]
            ),
            Err(Error::InvalidGeometry)
        ));
        assert!(matches!(
            record(&mut file, 1, &[], &[]),
            Err(Error::InvalidGeometry)
        ));
        let too_many = vec![0.; crate::step_read::MAX_STEP_ENTITIES / (helical::DEGREE + 1) + 1];
        assert!(matches!(
            record(&mut file, 1, &too_many, &[]),
            Err(Error::Limit)
        ));
    }
}
