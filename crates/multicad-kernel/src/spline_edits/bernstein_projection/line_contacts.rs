//! Transverse contacts with finite straight boundaries and a monotonicity
//! certificate for higher-degree rational sketch curves.
use super::*;

const MAX_CONTACTS: usize = 512;

fn unresolved_contact() -> GeometryError {
    failure(
        GeometryErrorKind::InvalidResult,
        "Rust spline and line contact could not be separated from a tangent, overlap or finite endpoint",
    )
}

#[derive(Clone, Copy)]
struct Boundary {
    start: [f64; 2],
    delta: [Interval; 2],
    length_squared: Interval,
}

#[derive(PartialEq, Eq)]
enum Extent {
    Outside,
    Inside,
    Ambiguous,
}

impl Boundary {
    fn new([start, end]: [[f64; 2]; 2], policy: GeometryPolicy) -> Result<Self, GeometryError> {
        for p in [start, end] {
            policy.point([p[0], p[1], 0.])?;
        }
        policy.length((end[0] - start[0]).hypot(end[1] - start[1]))?;
        let [dx, dy] = std::array::from_fn(|i| Interval::point(end[i]) - Interval::point(start[i]));
        Ok(Self {
            start,
            delta: [dx, dy],
            length_squared: dx * dx + dy * dy,
        })
    }

    fn extent(self, hull: Hull) -> Extent {
        let [dx, dy] = self.delta;
        let [x, y] = hull.0;
        let along = (dx * x + dy * y) / self.length_squared;
        if along.hi < 0. || along.lo > 1. {
            Extent::Outside
        } else if along.lo >= 0. && along.hi <= 1. {
            Extent::Inside
        } else {
            Extent::Ambiguous
        }
    }

    fn span(self, knots: [f64; 2], curve: Curve, degree: usize) -> Span {
        let [dx, dy] = self.delta;
        let mut coefficients: Polynomial = [Interval::ZERO; coefficients::MAX_COEFFICIENTS];
        for (coefficient, [x, y, _]) in coefficients[..=degree].iter_mut().zip(curve) {
            // The positive rational denominator cannot change the sign of
            // the oriented distance to the supporting line.
            *coefficient = dx * y - dy * x;
        }
        Span {
            knots,
            curve,
            derivative: coefficients,
        }
    }
}

struct Search<'a> {
    spline: &'a BSpline2d,
    policy: GeometryPolicy,
    boundary: Boundary,
    steps: usize,
    points: Vec<[f64; 2]>,
}

impl Search<'_> {
    fn record(&mut self, hull: Hull, parameter: f64) -> Result<(), GeometryError> {
        let point = self.spline.evaluate(parameter, self.policy)?.point;
        let tolerance = self.policy.linear_tolerance_mm;
        if hull.sample_error(point, self.boundary.start) > tolerance * 0.25
            || self
                .points
                .iter()
                .any(|other| (point[0] - other[0]).hypot(point[1] - other[1]) <= tolerance)
        {
            return Err(unresolved_contact());
        }
        if self.points.len() == MAX_CONTACTS {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Rust spline and line intersection exceeds the contact budget",
            ));
        }
        self.points.push(point);
        Ok(())
    }

    fn endpoint(&mut self, span: &Span, at_end: bool, degree: usize) -> Result<(), GeometryError> {
        let i = if at_end { degree } else { 0 };
        let hull = Hull::new(&span.curve[i..=i]);
        match self.boundary.extent(hull) {
            Extent::Outside => Ok(()),
            Extent::Ambiguous => Err(unresolved_contact()),
            Extent::Inside => {
                let next = if at_end { degree - 1 } else { 1 };
                if !matches!(span.derivative[next].sign(), Some(Less | Greater)) {
                    return Err(unresolved_contact());
                }
                self.record(hull, span.knots[usize::from(at_end)])
            }
        }
    }

    fn interior(&mut self, span: &Span, degree: usize) -> Result<(), GeometryError> {
        let mut stack = vec![Node {
            range: [0., 1.],
            curve: span.curve,
            derivative: span.derivative,
            depth: 0,
        }];
        while let Some(node) = stack.pop() {
            let coefficients = &node.derivative[..=degree];
            let variation = variation_upper(coefficients);
            if variation == Some(0) {
                continue;
            }
            let hull = Hull::new(&node.curve[..=degree]);
            let extent = self.boundary.extent(hull);
            if extent == Extent::Outside {
                continue;
            }
            if coefficients.iter().all(|c| c.sign() == Some(Equal)) {
                return Err(unresolved_contact());
            }
            if variation == Some(1) {
                let first = near_endpoint(coefficients.iter().copied());
                let last = near_endpoint(coefficients.iter().rev().copied());
                let transverse = matches!(
                    (first, last),
                    (Some(Less), Some(Greater)) | (Some(Greater), Some(Less))
                );
                if transverse
                    && extent == Extent::Inside
                    && hull.diameter() <= self.policy.linear_tolerance_mm * 0.01
                {
                    let parameter = span.parameter(f64::midpoint(node.range[0], node.range[1]));
                    if !(span.knots[0] < parameter && parameter < span.knots[1]) {
                        return Err(unresolved_contact());
                    }
                    self.record(hull, parameter)?;
                    continue;
                }
            }
            let [left, right] = node
                .split(degree, degree, &mut self.steps)
                .map_err(|_| unresolved_contact())?;
            stack.push(right);
            stack.push(left);
        }
        Ok(())
    }

    fn join(
        &self,
        left: &mut Span,
        right: &mut Span,
        degree: usize,
    ) -> Result<bool, GeometryError> {
        if left.derivative[degree].sign() != Some(Equal)
            && right.derivative[0].sign() != Some(Equal)
        {
            return Ok(false);
        }
        // Clamped B-spline spans agree at each authored knot. An exact zero
        // on either side proves the common endpoint belongs to the line.
        left.derivative[degree] = Interval::ZERO;
        right.derivative[0] = Interval::ZERO;
        let hull = Hull::new(&right.curve[..1]);
        if self.boundary.extent(hull) == Extent::Outside {
            return Ok(false);
        }
        let before = left.derivative[degree - 1].sign();
        let after = right.derivative[1].sign();
        if !matches!(
            (before, after),
            (Some(Less), Some(Greater)) | (Some(Greater), Some(Less))
        ) {
            return Err(unresolved_contact());
        }
        Ok(true)
    }
}

fn validate(spline: &BSpline2d, policy: GeometryPolicy) -> Result<usize, GeometryError> {
    spline.validate(policy)?;
    if !(2..=3).contains(&spline.degree) {
        return Err(failure(
            GeometryErrorKind::Unsupported,
            "Rust curved spline contact certification supports degree two and three",
        ));
    }
    Ok(spline.degree as usize)
}

pub(crate) fn contacts(
    spline: &BSpline2d,
    line: [[f64; 2]; 2],
    policy: GeometryPolicy,
) -> Result<Vec<[f64; 2]>, GeometryError> {
    let degree = validate(spline, policy)?;
    let boundary = Boundary::new(line, policy)?;
    let mut search = Search {
        spline,
        policy,
        boundary,
        steps: MAX_STEPS,
        points: Vec::new(),
    };
    let knots = expanded_knots(spline);
    let mut spans = spans(spline, &knots, boundary.start)
        .map(|(knots, curve)| boundary.span(knots, curve, degree));
    let mut current = spans.next().expect("validated spline has a span");
    if current.derivative[0].sign() == Some(Equal) {
        search.endpoint(&current, false, degree)?;
    }
    for mut next in spans {
        let shared_contact = search.join(&mut current, &mut next, degree)?;
        search.interior(&current, degree)?;
        if shared_contact {
            search.endpoint(&next, false, degree)?;
        }
        current = next;
    }
    search.interior(&current, degree)?;
    if current.derivative[degree].sign() == Some(Equal) {
        search.endpoint(&current, true, degree)?;
    }
    Ok(search.points)
}

pub(crate) fn certify_simple(
    spline: &BSpline2d,
    policy: GeometryPolicy,
) -> Result<(), GeometryError> {
    let degree = validate(spline, policy)?;
    let knots = expanded_knots(spline);
    let mut orders = [(true, true); 2];
    for (_, curve) in spans(spline, &knots, spline.poles[0]) {
        for (axis, (ascending, descending)) in orders.iter_mut().enumerate() {
            if *ascending || *descending {
                let sign = control_order(&curve[..=degree], axis);
                *ascending &= sign == Some(Greater);
                *descending &= sign == Some(Less);
            }
        }
        if orders
            .iter()
            .all(|&(ascending, descending)| !ascending && !descending)
        {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust spline self-intersection query requires a strictly monotone coordinate",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quadratic(poles: Vec<[f64; 2]>) -> BSpline2d {
        BSpline2d {
            degree: 2,
            poles,
            knots: vec![2., 9.],
            multiplicities: vec![3, 3],
            weights: Some(vec![1., 0.5, 1.]),
        }
    }

    #[test]
    fn transverse_contacts_obey_finite_bounds_and_authored_curve_order() {
        let policy = GeometryPolicy::default();
        let arch = quadratic(vec![[0., 0.], [10., 10.], [20., 0.]]);
        certify_simple(&arch, policy).unwrap();
        let line = [[10., -10.], [10., 10.]];
        let hits = contacts(&arch, line, policy).unwrap();
        assert_eq!(hits.len(), 1);
        assert!((hits[0][0] - 10.).abs() < 1e-8);
        assert!((hits[0][1] - 10. / 3.).abs() < 1e-8);
        assert_eq!(
            contacts(&arch, [line[1], line[0]], policy).unwrap().len(),
            1
        );
        assert!(
            contacts(&arch, [[10., -10.], [10., 2.]], policy)
                .unwrap()
                .is_empty()
        );
        let hits = contacts(&arch, [[-1., 2.], [21., 2.]], policy).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0][0] < hits[1][0]);
        assert!(hits.iter().all(|p| (p[1] - 2.).abs() < 1e-8));
        assert!(contacts(&arch, [[1., 1.], [1., 1.]], policy).is_err());
    }

    #[test]
    fn exact_endpoint_and_transverse_join_each_produce_one_contact() {
        let policy = GeometryPolicy::default();
        let arch = quadratic(vec![[0., 0.], [10., 10.], [20., 0.]]);
        assert_eq!(
            contacts(&arch, [[0., 0.], [0., -10.]], policy).unwrap(),
            [[0., 0.]]
        );
        assert_eq!(
            contacts(&arch, [[20., -10.], [20., 0.]], policy).unwrap(),
            [[20., 0.]]
        );
        assert!(
            contacts(&arch, [[0., -20.], [0., -10.]], policy)
                .unwrap()
                .is_empty()
        );
        let piecewise = BSpline2d {
            degree: 2,
            poles: vec![[0., 0.], [1., 1.], [2., 1.], [3., 2.], [4., 3.]],
            knots: vec![1., 2., 5.],
            multiplicities: vec![3, 2, 3],
            weights: None,
        };
        certify_simple(&piecewise, policy).unwrap();
        assert_eq!(
            contacts(&piecewise, [[2., 0.], [2., 4.]], policy).unwrap(),
            [[2., 1.]]
        );
        assert!(
            contacts(&piecewise, [[2., -5.], [2., -2.]], policy)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn overlap_tangent_and_nonmonotone_self_queries_refuse_ambiguous_geometry() {
        let policy = GeometryPolicy::default();
        let parabola = BSpline2d {
            degree: 2,
            poles: vec![[-1., 1.], [0., -1.], [1., 1.]],
            knots: vec![0., 1.],
            multiplicities: vec![3, 3],
            weights: None,
        };
        assert!(contacts(&parabola, [[-2., 0.], [2., 0.]], policy).is_err());
        let straight = quadratic(vec![[0., 0.], [2., 0.], [4., 0.]]);
        assert!(contacts(&straight, [[-1., 0.], [5., 0.]], policy).is_err());
        assert!(
            contacts(&straight, [[8., 0.], [9., 0.]], policy)
                .unwrap()
                .is_empty()
        );
        let retraced = quadratic(vec![[0., 0.], [2., 0.], [0., 0.]]);
        assert_eq!(
            certify_simple(&retraced, policy).unwrap_err().kind,
            GeometryErrorKind::Unsupported
        );
        let reversed = quadratic(vec![[4., 0.], [2., -1.], [0., 2.]]);
        certify_simple(&reversed, policy).unwrap();
        let unsupported = BSpline2d {
            degree: 1,
            poles: vec![[0., 0.], [3., 2.]],
            knots: vec![0., 1.],
            multiplicities: vec![2, 2],
            weights: None,
        };
        assert_eq!(
            certify_simple(&unsupported, policy).unwrap_err().kind,
            GeometryErrorKind::Unsupported
        );
    }

    fn cubic() -> BSpline2d {
        BSpline2d {
            degree: 3,
            poles: vec![[0., 0.], [1., 4.], [2., 2.], [4., -1.], [5., 0.]],
            knots: vec![2., 3.5, 8.],
            multiplicities: vec![4, 1, 4],
            weights: Some(vec![1., 0.5, 2., 1., 1.]),
        }
    }

    #[test]
    fn cubic_contacts_cross_multiple_authored_spans() {
        let policy = GeometryPolicy::default();
        let curve = cubic();
        certify_simple(&curve, policy).unwrap();
        let points = contacts(&curve, [[-1., 0.75], [6., 0.75]], policy).unwrap();
        assert_eq!(points.len(), 2);
        assert!(points[0][0] < points[1][0]);
    }

    #[test]
    fn unresolved_finite_endpoints_and_knot_touches_are_not_returned_as_crossings() {
        let policy = GeometryPolicy::default();
        let curve = quadratic(vec![[0., 0.], [2., 1.], [4., 0.]]);
        let boundary = Boundary::new([[0., 0.], [4., 0.]], policy).unwrap();
        let mut search = Search {
            spline: &curve,
            policy,
            boundary,
            steps: MAX_STEPS,
            points: Vec::new(),
        };
        let coordinate = Interval {
            lo: -1e-12,
            hi: 1e-12,
        };
        let span = Span {
            knots: [2., 9.],
            curve: [[coordinate, Interval::ZERO, Interval::point(1.)]; 4],
            derivative: [Interval::ZERO; 9],
        };
        assert!(search.endpoint(&span, false, 2).is_err());
        assert!(search.interior(&span, 2).is_err());

        let tangent_join = BSpline2d {
            degree: 2,
            poles: vec![[0., 1.], [1., 1.], [2., 0.], [3., 1.], [4., 1.]],
            knots: vec![1., 2., 5.],
            multiplicities: vec![3, 2, 3],
            weights: None,
        };
        assert!(contacts(&tangent_join, [[0., 0.], [4., 0.]], policy).is_err());
    }

    #[test]
    fn contact_budgets_and_unrepresentable_authored_parameters_fail_explicitly() {
        let policy = GeometryPolicy::default();
        let curve = quadratic(vec![[0., 0.], [2., 1.], [4., 0.]]);
        let boundary = Boundary::new([[0., 0.], [4., 0.]], policy).unwrap();
        let mut search = Search {
            spline: &curve,
            policy,
            boundary,
            steps: MAX_STEPS,
            points: vec![[0., 0.]],
        };
        let first = Hull([Interval::ZERO; 2]);
        assert_eq!(
            search.record(first, 2.).unwrap_err().kind,
            GeometryErrorKind::InvalidResult
        );
        search.points = vec![[1., 1.]; MAX_CONTACTS];
        assert_eq!(
            search.record(first, 2.).unwrap_err().kind,
            GeometryErrorKind::Limit
        );

        let start: f64 = 100_000_000.;
        let mut narrow = curve;
        narrow.knots = vec![start, start.next_up()];
        assert_eq!(
            contacts(&narrow, [[2., -1.], [2., 2.]], policy)
                .unwrap_err()
                .kind,
            GeometryErrorKind::InvalidResult
        );
    }

    #[cfg(feature = "occt")]
    #[test]
    fn transverse_cubic_contacts_match_independent_occt() {
        let policy = GeometryPolicy::default();
        let cubic = cubic();
        certify_simple(&cubic, policy).unwrap();
        let line = [[-1., 0.75], [6., 0.75]];
        let rust = contacts(&cubic, line, policy).unwrap();
        let native = crate::CurvedEdge2d::BSpline(cubic)
            .intersections(
                &crate::CurvedEdge2d::Line {
                    a: line[0],
                    b: line[1],
                },
                policy,
            )
            .unwrap();
        assert_eq!(rust.len(), 2);
        assert_eq!(rust.len(), native.points.len());
        for p in rust {
            assert!(
                native
                    .points
                    .iter()
                    .any(|h| { (h.point[0] - p[0]).hypot(h.point[1] - p[1]) < 1e-7 })
            );
        }
    }
}
