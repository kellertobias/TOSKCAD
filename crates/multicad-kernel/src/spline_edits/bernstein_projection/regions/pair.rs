use super::*;

pub(crate) fn contacts(
    a: &BSpline2d,
    b: &BSpline2d,
    policy: GeometryPolicy,
    interrupt: QueryCheckpoint<'_>,
) -> Result<(Vec<[f64; 2]>, bool), GeometryError> {
    let origin = a.poles.first().copied().ok_or_else(uncertain)?;
    let left = pieces(a, origin, policy, interrupt)?;
    let right = pieces(b, origin, policy, interrupt)?;
    if a == b {
        return Ok((vec![], true));
    }
    let endpoint = |spline: &BSpline2d, piece: &Piece, index| {
        if index == 0 && piece.range[0] == spline.knots[0] {
            Some(spline.poles[0])
        } else if index == piece.degree
            && piece.range[1] == *spline.knots.last().expect("validated knots")
        {
            spline.poles.last().copied()
        } else {
            None
        }
    };
    let mut budget = Budget {
        remaining: WORK,
        checkpoint: interrupt,
    };
    let mut result: Vec<[f64; 2]> = Vec::new();
    let mut shared = std::collections::BTreeSet::new();
    for x in left {
        for y in &right {
            let mut stack = vec![(x.clone(), y.clone())];
            while let Some((x, y)) = stack.pop() {
                budget.step()?;
                if simple::disjoint(&x, &y) {
                    continue;
                }
                let mut separated = false;
                for xi in [0, x.degree] {
                    for yi in [0, y.degree] {
                        if let (Some(p), Some(q)) = (endpoint(a, &x, xi), endpoint(b, &y, yi))
                            && p == q
                            && simple::endpoint_separation(&x, xi, &y, yi)
                        {
                            let key = p.map(|value| if value == 0. { 0 } else { value.to_bits() });
                            if shared.insert(key) {
                                if result.iter().any(|old| {
                                    (old[0] - p[0]).hypot(old[1] - p[1])
                                        <= policy.linear_tolerance_mm * 2.
                                }) {
                                    return Err(uncertain());
                                }
                                result.push(p);
                            }
                            separated = true;
                        }
                    }
                }
                if separated {
                    continue;
                }
                if let Some(point) =
                    contact::transverse(x.clone(), y.clone(), origin, policy, &mut budget)?
                {
                    if result.iter().any(|old| {
                        (old[0] - point[0]).hypot(old[1] - point[1])
                            <= policy.linear_tolerance_mm * 2.
                    }) {
                        return Err(uncertain());
                    }
                    result.push(point);
                    if result.len() > 512 {
                        return Err(failure(
                            GeometryErrorKind::Limit,
                            "Spline contact query exceeds its contact budget",
                        ));
                    }
                    continue;
                }
                if x.hull().diameter() >= y.hull().diameter() {
                    for child in x.split()? {
                        stack.push((child, y.clone()));
                    }
                } else {
                    for child in y.split()? {
                        stack.push((x.clone(), child));
                    }
                }
            }
        }
    }
    Ok((result, false))
}
