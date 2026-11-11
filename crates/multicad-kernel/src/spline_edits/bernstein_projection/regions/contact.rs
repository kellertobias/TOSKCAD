use super::*;

fn hull(values: impl Iterator<Item = Interval>) -> Interval {
    values.fold(
        Interval {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY,
        },
        |a, b| Interval {
            lo: a.lo.min(b.lo),
            hi: a.hi.max(b.hi),
        },
    )
}
fn evaluate(piece: &Piece, t: f64) -> [Interval; 2] {
    let mut work = piece.controls;
    for count in (1..=piece.degree).rev() {
        for i in 0..count {
            work[i] = std::array::from_fn(|axis| {
                work[i][axis] * (Interval::point(1.) - Interval::point(t))
                    + work[i + 1][axis] * Interval::point(t)
            });
        }
    }
    [work[0][0] / work[0][2], work[0][1] / work[0][2]]
}
fn derivative(piece: &Piece) -> [Interval; 2] {
    let w = hull(piece.controls[..=piece.degree].iter().map(|p| p[2]));
    let d = |axis| {
        hull(
            piece.controls[..=piece.degree]
                .windows(2)
                .map(|pair| (pair[1][axis] - pair[0][axis]) * Interval::point(piece.degree as f64)),
        )
    };
    std::array::from_fn(|axis| {
        (d(axis) * w - hull(piece.controls[..=piece.degree].iter().map(|p| p[axis])) * d(2))
            / (w * w)
    })
}
fn krawczyk(a: &Piece, b: &Piece) -> Option<[Interval; 2]> {
    let da = derivative(a);
    let db = derivative(b);
    let j = [
        [da[0], Interval::ZERO - db[0]],
        [da[1], Interval::ZERO - db[1]],
    ];
    let midpoint = |v: Interval| (v.lo + v.hi) * 0.5;
    let c = j.map(|row| row.map(midpoint));
    let determinant = c[0][0] * c[1][1] - c[0][1] * c[1][0];
    if !determinant.is_finite() || determinant == 0. {
        return None;
    }
    let inverse = [
        [c[1][1] / determinant, -c[0][1] / determinant],
        [-c[1][0] / determinant, c[0][0] / determinant],
    ];
    if !inverse.iter().flatten().all(|x| x.is_finite()) {
        return None;
    }
    let [a0, b0] = [evaluate(a, 0.5), evaluate(b, 0.5)];
    let f: [Interval; 2] = std::array::from_fn(|i| a0[i] - b0[i]);
    let m: [[Interval; 2]; 2] = std::array::from_fn(|i| {
        std::array::from_fn(|k| {
            Interval::point(if i == k { 1. } else { 0. })
                - (0..2).fold(Interval::ZERO, |sum, l| {
                    sum + Interval::point(inverse[i][l]) * j[l][k]
                })
        })
    });
    if m.iter().any(|row| {
        row.iter()
            .fold(Interval::ZERO, |sum, v| {
                sum + Interval::point(v.lo.abs().max(v.hi.abs()))
            })
            .hi
            >= 1.
    }) {
        return None;
    }
    let range: [Interval; 2] = std::array::from_fn(|i| {
        Interval::point(0.5)
            - (0..2).fold(Interval::ZERO, |sum, k| {
                sum + Interval::point(inverse[i][k]) * f[k]
            })
            + m[i].into_iter().fold(Interval::ZERO, |sum, v| {
                sum + v * Interval { lo: -0.5, hi: 0.5 }
            })
    });
    range
        .iter()
        .all(|r| r.lo > 0. && r.hi < 1.)
        .then_some(range)
}
fn restrict(piece: &Piece, range: Interval) -> Piece {
    let split = |controls: Curve, t: f64| {
        subdivide(controls, piece.degree, |a, b| {
            std::array::from_fn(|i| {
                a[i] * (Interval::point(1.) - Interval::point(t)) + b[i] * Interval::point(t)
            })
        })
    };
    let left = split(piece.controls, range.hi)[0];
    let controls = split(
        left,
        (Interval::point(range.lo) / Interval::point(range.hi)).lo,
    )[1];
    Piece {
        controls,
        degree: piece.degree,
        range: piece.range,
        depth: piece.depth,
    }
}

// Interval Krawczyk inclusion proves a unique actual transverse root. A close
// pair of curve hulls alone is never accepted as an intersection.
pub(super) fn transverse(
    mut a: Piece,
    mut b: Piece,
    origin: [f64; 2],
    policy: GeometryPolicy,
    budget: &mut Budget<'_>,
) -> Result<Option<[f64; 2]>, GeometryError> {
    for _ in 0..DEPTH {
        budget.step()?;
        let Some(ranges) = krawczyk(&a, &b) else {
            return Ok(None);
        };
        let h = a.hull();
        let point = std::array::from_fn(|i| origin[i] + (h.0[i].lo + h.0[i].hi) * 0.5);
        if h.sample_error(point, origin) <= policy.linear_tolerance_mm * 0.125 {
            return Ok(Some(point));
        }
        let old = h.diameter() + b.hull().diameter();
        a = restrict(&a, ranges[0]);
        b = restrict(&b, ranges[1]);
        if a.hull().diameter() + b.hull().diameter() >= old {
            return Err(uncertain());
        }
    }
    Err(uncertain())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_dyadic_restriction_encloses_the_exact_complement() {
        let mut controls = [[Interval::ZERO; 3]; 4];
        controls[0] = [Interval::point(1e17), Interval::ZERO, Interval::point(1.)];
        controls[1] = [Interval::ZERO, Interval::ZERO, Interval::point(1.)];
        let piece = Piece {
            controls,
            degree: 1,
            range: [0., 1.],
            depth: 0,
        };
        let value = evaluate(&piece, 1e-17)[0];
        assert!(value.lo < 1e17 && value.hi >= 1e17);
        let restricted = restrict(&piece, Interval { lo: 1e-17, hi: 0.5 });
        assert!(restricted.controls[0][0].lo < 1e17);
        assert!(restricted.controls[0][0].hi >= 1e17);
    }
}
