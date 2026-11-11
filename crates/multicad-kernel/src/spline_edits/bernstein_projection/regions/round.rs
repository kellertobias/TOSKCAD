use super::*;

pub(crate) fn round_contacts(
    spline: &BSpline2d,
    center: [f64; 2],
    radius: f64,
    policy: GeometryPolicy,
    interrupt: QueryCheckpoint<'_>,
) -> Result<Vec<[f64; 2]>, GeometryError> {
    policy.length(radius)?;
    policy.point([center[0], center[1], 0.])?;
    let mut budget = Budget {
        remaining: WORK,
        checkpoint: interrupt,
    };
    let mut points: Vec<[f64; 2]> = Vec::new();
    let mut endpoints = std::collections::BTreeSet::new();
    for piece in pieces(spline, center, policy, interrupt)? {
        let degree = piece.degree;
        let [x, y, w] = std::array::from_fn::<_, 3, _>(|axis| {
            piece.controls[..=degree]
                .iter()
                .map(|p| p[axis])
                .collect::<Vec<_>>()
        });
        let mut coefficients = [Interval::ZERO; coefficients::MAX_COEFFICIENTS];
        coefficients::product(&x, &x, &mut coefficients, false);
        coefficients::product(&y, &y, &mut coefficients, false);
        let w: Vec<_> = w
            .iter()
            .map(|&weight| weight * Interval::point(radius))
            .collect();
        coefficients::product(&w, &w, &mut coefficients, true);
        let mut stack = vec![Node {
            range: piece.range,
            curve: piece.controls,
            derivative: coefficients,
            depth: 0,
        }];
        while let Some(node) = stack.pop() {
            budget.step()?;
            for index in [0, 2 * degree] {
                if node.derivative[index].sign() == Some(Equal) {
                    let control_index = if index == 0 { 0 } else { degree };
                    let near = if index == 0 { 1 } else { 2 * degree - 1 };
                    if !matches!(node.derivative[near].sign(), Some(Less | Greater)) {
                        return Err(uncertain());
                    }
                    let endpoint = Hull::new(&node.curve[control_index..=control_index]);
                    let point = std::array::from_fn(|i| {
                        center[i] + (endpoint.0[i].lo + endpoint.0[i].hi) * 0.5
                    });
                    if endpoint.sample_error(point, center) > policy.linear_tolerance_mm * 0.125 {
                        return Err(uncertain());
                    }
                    let parameter = node.range[usize::from(index != 0)];
                    let parameter = if parameter == 0. {
                        0
                    } else {
                        parameter.to_bits()
                    };
                    if endpoints.insert(parameter) {
                        if points.iter().any(|p| {
                            (p[0] - point[0]).hypot(p[1] - point[1])
                                <= policy.linear_tolerance_mm * 2.
                        }) {
                            return Err(uncertain());
                        }
                        points.push(point);
                        if points.len() > 512 {
                            return Err(failure(
                                GeometryErrorKind::Limit,
                                "Spline circle query exceeds its contact budget",
                            ));
                        }
                    }
                    // Descartes' rule ignores exact endpoint zeros; the
                    // remaining coefficients still certify every interior root.
                }
            }
            let source = &node.derivative[..=2 * degree];
            if variation_upper(source) == Some(0) {
                continue;
            }
            let bracket = matches!(
                (source[0].sign(), source[2 * degree].sign()),
                (Some(Less), Some(Greater)) | (Some(Greater), Some(Less))
            );
            let hull = Hull::new(&node.curve[..=degree]);
            if bracket
                && variation_upper(source) == Some(1)
                && hull.diameter() <= policy.linear_tolerance_mm * 0.0625
            {
                let point =
                    std::array::from_fn(|i| center[i] + (hull.0[i].lo + hull.0[i].hi) * 0.5);
                if hull.sample_error(point, center) > policy.linear_tolerance_mm * 0.125
                    || points.iter().any(|p| {
                        (p[0] - point[0]).hypot(p[1] - point[1]) <= policy.linear_tolerance_mm * 2.
                    })
                {
                    return Err(uncertain());
                }
                points.push(point);
                continue;
            }
            if points.len() > 512 {
                return Err(failure(
                    GeometryErrorKind::Limit,
                    "Spline circle query exceeds its contact budget",
                ));
            }
            let children = node.split(degree, 2 * degree, &mut budget.remaining)?;
            stack.extend(children);
        }
    }
    Ok(points)
}
