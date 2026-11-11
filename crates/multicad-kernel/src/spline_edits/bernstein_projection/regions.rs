//! Bounded predicates over the exact positive-weight Bernstein source spans.
//! Control hulls are proof enclosures, never an authoritative polygonal rim.
use super::*;

mod area;
mod contact;
mod pair;
mod round;
mod simple;

pub(crate) use area::signed_area;
pub(crate) use pair::contacts;
pub(crate) use round::round_contacts;
pub(crate) use simple::self_contacts;

const WORK: usize = 131_072;
const DEPTH: usize = 48;

struct Budget<'a> {
    remaining: usize,
    checkpoint: QueryCheckpoint<'a>,
}
impl Budget<'_> {
    fn step(&mut self) -> Result<(), GeometryError> {
        if let Some(checkpoint) = self.checkpoint {
            checkpoint()?;
        }
        self.remaining = self.remaining.checked_sub(1).ok_or_else(|| {
            failure(
                GeometryErrorKind::Limit,
                "Spline region proof exceeds its work budget",
            )
        })?;
        Ok(())
    }
}
fn uncertain() -> GeometryError {
    failure(
        GeometryErrorKind::InvalidResult,
        "Spline region predicate cannot be certified at the requested precision",
    )
}

#[derive(Clone)]
struct Piece {
    controls: Curve,
    degree: usize,
    range: [f64; 2],
    depth: usize,
}
impl Piece {
    fn split(&self) -> Result<[Self; 2], GeometryError> {
        if self.depth >= DEPTH {
            return Err(uncertain());
        }
        let middle = (self.range[0] + self.range[1]) * 0.5;
        if middle == self.range[0] || middle == self.range[1] {
            return Err(uncertain());
        }
        let controls = subdivide(self.controls, self.degree, |a, b| {
            std::array::from_fn(|i| (a[i] + b[i]) * Interval::point(0.5))
        });
        Ok(std::array::from_fn(|i| Self {
            controls: controls[i],
            degree: self.degree,
            range: if i == 0 {
                [self.range[0], middle]
            } else {
                [middle, self.range[1]]
            },
            depth: self.depth + 1,
        }))
    }
    fn hull(&self) -> Hull {
        Hull::new(&self.controls[..=self.degree])
    }
}
fn pieces(
    spline: &BSpline2d,
    origin: [f64; 2],
    policy: GeometryPolicy,
    checkpoint: QueryCheckpoint<'_>,
) -> Result<Vec<Piece>, GeometryError> {
    spline.validate(policy)?;
    if !(1..=3).contains(&spline.degree) {
        return Err(failure(
            GeometryErrorKind::Unsupported,
            "Spline region predicates require degree one, two or three",
        ));
    }
    let knots = expanded_knots(spline);
    let result: Vec<_> = spans(spline, &knots, origin)
        .map(|(range, controls)| {
            if let Some(checkpoint) = checkpoint {
                checkpoint()?;
            }
            Ok(Piece {
                controls,
                degree: spline.degree as usize,
                range,
                depth: 0,
            })
        })
        .collect::<Result<_, GeometryError>>()?;
    if result.iter().any(|piece| {
        piece.controls[..=piece.degree]
            .iter()
            .any(|p| p.iter().any(|x| !x.lo.is_finite() || !x.hi.is_finite()) || p[2].lo <= 0.)
    }) {
        return Err(uncertain());
    }
    Ok(result)
}

/// Prove an actual source point lies inside the boundary band, or that the
/// entire curve is separated from it. No stationary-root guess is required.
pub(crate) fn boundary_clearance(
    spline: &BSpline2d,
    point: [f64; 2],
    policy: GeometryPolicy,
    interrupt: QueryCheckpoint<'_>,
) -> Result<bool, GeometryError> {
    policy.point([point[0], point[1], 0.])?;
    let mut budget = Budget {
        remaining: WORK,
        checkpoint: interrupt,
    };
    struct Candidate {
        distance: Interval,
        piece: Piece,
    }
    impl PartialEq for Candidate {
        fn eq(&self, other: &Self) -> bool {
            self.distance.lo == other.distance.lo && self.distance.hi == other.distance.hi
        }
    }
    impl Eq for Candidate {}
    impl PartialOrd for Candidate {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Candidate {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .distance
                .lo
                .total_cmp(&self.distance.lo)
                .then_with(|| other.distance.hi.total_cmp(&self.distance.hi))
        }
    }
    let mut queue: std::collections::BinaryHeap<_> = pieces(spline, point, policy, interrupt)?
        .into_iter()
        .map(|piece| Candidate {
            distance: piece.hull().distance(),
            piece,
        })
        .collect();
    while let Some(Candidate { distance, piece }) = queue.pop() {
        budget.step()?;
        if distance.hi <= policy.linear_tolerance_mm * 0.25 {
            return Ok(true);
        }
        if distance.lo > policy.linear_tolerance_mm * 2. {
            return Ok(false);
        }
        for child in piece.split()? {
            queue.push(Candidate {
                distance: child.hull().distance(),
                piece: child,
            });
        }
        if queue.len() > 16_384 {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Spline boundary clearance exceeds its queue budget",
            ));
        }
    }
    Ok(false)
}

pub(crate) struct PolynomialPatch {
    pub poles: [[f64; 2]; 4],
}
/// Exact basis conversion, with every rounded coefficient enclosed. The
/// caller's canonical basis is retained; these are manufacturing span data.
pub(crate) fn polynomial_patches(
    spline: &BSpline2d,
    depth: u32,
    policy: GeometryPolicy,
    interrupt: QueryCheckpoint<'_>,
) -> Result<Vec<PolynomialPatch>, GeometryError> {
    if spline
        .weights
        .as_ref()
        .is_some_and(|weights| weights.windows(2).any(|pair| pair[0] != pair[1]))
    {
        return Err(failure(
            GeometryErrorKind::Unsupported,
            "Authored rational spline extrusion requires a rational surface provider",
        ));
    }
    let source = pieces(spline, [0., 0.], policy, interrupt)?;
    let count = source.len().checked_shl(depth).ok_or_else(|| {
        failure(
            GeometryErrorKind::Limit,
            "Spline manufacturing has too many source spans",
        )
    })?;
    if count > 1024 {
        return Err(failure(
            GeometryErrorKind::Limit,
            "Spline manufacturing exceeds its exact span budget",
        ));
    }
    let mut budget = Budget {
        remaining: WORK,
        checkpoint: interrupt,
    };
    let mut result = Vec::with_capacity(count);
    for piece in source {
        let raw: Vec<[Interval; 2]> = piece.controls[..=piece.degree]
            .iter()
            .map(|p| [p[0] / p[2], p[1] / p[2]])
            .collect();
        let elevated: [[Interval; 2]; 4] = match piece.degree {
            1 => [
                raw[0],
                std::array::from_fn(|i| {
                    (raw[0][i] * Interval::point(2.) + raw[1][i]) / Interval::point(3.)
                }),
                std::array::from_fn(|i| {
                    (raw[0][i] + raw[1][i] * Interval::point(2.)) / Interval::point(3.)
                }),
                raw[1],
            ],
            2 => [
                raw[0],
                std::array::from_fn(|i| {
                    (raw[0][i] + raw[1][i] * Interval::point(2.)) / Interval::point(3.)
                }),
                std::array::from_fn(|i| {
                    (raw[1][i] * Interval::point(2.) + raw[2][i]) / Interval::point(3.)
                }),
                raw[2],
            ],
            3 => [raw[0], raw[1], raw[2], raw[3]],
            _ => unreachable!("validated degree"),
        };
        let controls = elevated.map(|p| [p[0], p[1], Interval::point(1.)]);
        let mut stack = vec![(controls, 0_u32)];
        while let Some((controls, level)) = stack.pop() {
            budget.step()?;
            if level == depth {
                let poles = controls.map(|p| std::array::from_fn(|i| (p[i].lo + p[i].hi) * 0.5));
                let maximum = controls
                    .iter()
                    .zip(poles)
                    .map(|(bounds, point)| {
                        (0..2)
                            .fold(Interval::ZERO, |sum, i| {
                                let delta = Interval::point(point[i]) - bounds[i];
                                sum + Interval::point(delta.lo.abs().max(delta.hi.abs()))
                            })
                            .hi
                    })
                    .fold(0., f64::max);
                if !maximum.is_finite() || maximum > policy.linear_tolerance_mm / 128. {
                    return Err(uncertain());
                }
                // Bound the derivative perturbation separately: positional
                // closeness alone says nothing about a nearly stationary
                // source tangent. Bernstein derivative controls enclose the
                // full span, including endpoints.
                let derivatives: [[Interval; 2]; 3] = std::array::from_fn(|i| {
                    std::array::from_fn(|axis| {
                        (controls[i + 1][axis] - controls[i][axis]) * Interval::point(3.)
                    })
                });
                let error = (0..3)
                    .map(|i| {
                        (0..2)
                            .fold(Interval::ZERO, |sum, axis| {
                                let published = (Interval::point(poles[i + 1][axis])
                                    - Interval::point(poles[i][axis]))
                                    * Interval::point(3.);
                                let delta = derivatives[i][axis] - published;
                                sum + Interval::point(delta.lo.abs().max(delta.hi.abs()))
                            })
                            .hi
                    })
                    .fold(0., f64::max);
                let speed = (0..2)
                    .map(|axis| {
                        let lo = derivatives
                            .iter()
                            .map(|d| d[axis].lo)
                            .fold(f64::INFINITY, f64::min);
                        let hi = derivatives
                            .iter()
                            .map(|d| d[axis].hi)
                            .fold(f64::NEG_INFINITY, f64::max);
                        lo.max(-hi).max(0.)
                    })
                    .fold(0., f64::max);
                // For |delta|/|source derivative| <= e, the two unit
                // tangents differ by at most 2*asin(e). This stricter
                // algebraic bound reserves 1e-6 radians without depending
                // on correctly rounded trigonometric functions.
                if !error.is_finite()
                    || error > 0.
                        && (speed <= 0.
                            || (Interval::point(error) / Interval::point(speed)).hi > 2e-7)
                {
                    return Err(uncertain());
                }
                result.push(PolynomialPatch { poles });
            } else {
                let [a, b] = subdivide(controls, 3, |a, b| {
                    std::array::from_fn(|i| (a[i] + b[i]) * Interval::point(0.5))
                });
                stack.push((b, level + 1));
                stack.push((a, level + 1));
            }
        }
    }
    Ok(result)
}
