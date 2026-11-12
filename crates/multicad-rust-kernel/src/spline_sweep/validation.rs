//! Shared source-rim proof for authored and decoded spline sweeps.
use super::{ConstructionError, Edge};
use crate::normal_sweep;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
mod roundoff;
type Point2 = [f64; 2];
type Result<T> = std::result::Result<T, ConstructionError>;
pub(crate) const MAX_PAIR_WORK: usize = 4_000_000;
const MAX_DEPTH: usize = 24;
pub(crate) struct Budget<'a> {
    pub(crate) remaining: usize,
    pub(crate) interrupt: Option<&'a dyn Fn() -> bool>,
}
pub(crate) struct Boundary {
    pub(crate) pieces: Vec<Piece>,
    pub(crate) bounds: [Point2; 2],
}
pub(crate) fn simple(section: &[Boundary], tolerance: f64, budget: &mut Budget<'_>) -> Result<()> {
    // Two consecutive edges may meet only at their common vertex. Prove
    // this by separating the entire pair on the direction of their common
    // travel. Spur joins have a consistent one-sided tangent, including
    // the one-sided limit of a base-circle cubic.
    for (a, b) in section.iter().zip(section.iter().cycle().skip(1)) {
        consume(budget)?;
        let a_last = a.pieces.last().ok_or(ConstructionError::Invalid)?;
        let b_first = b.pieces.first().ok_or(ConstructionError::Invalid)?;
        let [u, v] = [a_last.tangent(true), b_first.tangent(false)];
        let axis = unit2([u[0] + v[0], u[1] + v[1]]).ok_or(ConstructionError::Unresolved)?;
        if !a
            .pieces
            .iter()
            .chain(&b.pieces)
            .all(|piece| piece.forward_along(axis))
        {
            return Err(ConstructionError::Unresolved);
        }
    }
    // For every non-adjacent pair, use certified interval boxes. Refuse
    // touching, crossing or unresolved curves within the numerical budget.
    for (i, a) in section.iter().enumerate() {
        for (j, b) in section.iter().enumerate().skip(i + 2) {
            consume(budget)?;
            if i == 0 && j + 1 == section.len() || separated(a.bounds, b.bounds, tolerance) {
                continue;
            }
            for &x in &a.pieces {
                for &y in &b.pieces {
                    clear_pair(x, y, tolerance, 0, budget)?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn clear_pair(
    a: Piece,
    b: Piece,
    tolerance: f64,
    depth: usize,
    budget: &mut Budget<'_>,
) -> Result<()> {
    consume(budget)?;
    let [a_box, b_box] = [a.bounds(), b.bounds()];
    if separated(a_box, b_box, tolerance) {
        return Ok(());
    }
    if depth == MAX_DEPTH || (diameter(a_box).max(diameter(b_box)) <= tolerance) {
        return Err(ConstructionError::Unresolved);
    }
    if diameter(a_box) >= diameter(b_box) {
        for piece in a.split() {
            clear_pair(piece, b, tolerance, depth + 1, budget)?;
        }
    } else {
        for piece in b.split() {
            clear_pair(a, piece, tolerance, depth + 1, budget)?;
        }
    }
    Ok(())
}

pub(crate) fn clear_disk(
    piece: Piece,
    center: Point2,
    radius: f64,
    depth: usize,
    budget: &mut Budget<'_>,
) -> Result<f64> {
    consume(budget)?;
    let [a, b] = piece.bounds();
    let closest = std::array::from_fn(|i| center[i].clamp(a[i], b[i]));
    if norm(sub2(closest, center)) > radius {
        let [a, b] = piece.endpoints().map(|p| sub2(p, center));
        return Ok(cross2(a, b).atan2(dot2(a, b)));
    }
    if depth == MAX_DEPTH || diameter([a, b]) <= radius * 1e-8 {
        return Err(ConstructionError::Unresolved);
    }
    piece
        .split()
        .into_iter()
        .map(|piece| clear_disk(piece, center, radius, depth + 1, budget))
        .sum()
}

fn consume(budget: &mut Budget<'_>) -> Result<()> {
    if budget.interrupt.is_some_and(|interrupt| interrupt()) {
        return Err(ConstructionError::Cancelled);
    }
    budget.remaining = budget
        .remaining
        .checked_sub(1)
        .ok_or(ConstructionError::Limit)?;
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum Piece {
    Line([Point2; 2]),
    Arc(Arc),
    Bezier([Point2; 4]),
}

#[derive(Clone, Copy)]
pub(crate) struct Arc {
    pub(crate) center: Point2,
    pub(crate) radius: f64,
    pub(crate) start: f64,
    pub(crate) sweep: f64,
}

impl Piece {
    pub(crate) fn endpoints(self) -> [Point2; 2] {
        match self {
            Self::Line(points) => points,
            Self::Arc(arc) => [arc.point(arc.start), arc.point(arc.start + arc.sweep)],
            Self::Bezier([a, _, _, b]) => [a, b],
        }
    }

    pub(crate) fn bounds(self) -> [Point2; 2] {
        let [a, b] = self.endpoints();
        let mut bounds = merge([a; 2], [b; 2]);
        match self {
            Self::Line(_) => {}
            Self::Arc(arc) => {
                for angle in [0., FRAC_PI_2, PI, 3. * FRAC_PI_2] {
                    if arc.contains(angle) {
                        bounds = merge(bounds, [arc.point(angle); 2]);
                    }
                }
            }
            Self::Bezier([_, b, c, _]) => {
                bounds = merge(merge(bounds, [b; 2]), [c; 2]);
            }
        }
        bounds
    }

    fn split(self) -> [Self; 2] {
        match self {
            Self::Line([a, b]) => {
                let m = midpoint(a, b);
                [Self::Line([a, m]), Self::Line([m, b])]
            }
            Self::Arc(mut arc) => {
                arc.sweep *= 0.5;
                [
                    Self::Arc(arc),
                    Self::Arc(Arc {
                        start: arc.start + arc.sweep,
                        ..arc
                    }),
                ]
            }
            Self::Bezier([a, b, c, d]) => {
                let [ab, bc, cd] = [midpoint(a, b), midpoint(b, c), midpoint(c, d)];
                let [x, y] = [midpoint(ab, bc), midpoint(bc, cd)];
                let m = midpoint(x, y);
                [Self::Bezier([a, ab, x, m]), Self::Bezier([m, y, cd, d])]
            }
        }
    }

    fn tangent(self, end: bool) -> Point2 {
        match self {
            Self::Line([a, b]) => unit2(sub2(b, a)).unwrap_or([0.; 2]),
            Self::Arc(arc) => arc.tangent(arc.start + if end { arc.sweep } else { 0. }),
            Self::Bezier(mut p) => {
                if end {
                    p.reverse();
                }
                let tangent = p[1..]
                    .iter()
                    .find_map(|&b| unit2(sub2(b, p[0])))
                    .unwrap_or([0.; 2]);
                if end { tangent.map(|v| -v) } else { tangent }
            }
        }
    }

    fn certified_forward(self, axis: Point2) -> bool {
        if !self.forward_along(axis) {
            return false;
        }
        match self {
            Self::Line([a, b]) => roundoff::nonnegative_difference(b, a, axis),
            Self::Bezier(p) => p
                .windows(2)
                .all(|pair| roundoff::nonnegative_difference(pair[1], pair[0], axis)),
            Self::Arc(arc) => {
                // A less-than-semicircular tangent sweep with positive end
                // projections stays in one open projection half-plane.
                arc.sweep.abs() < PI - 128. * f64::EPSILON
                    && [arc.start, arc.start + arc.sweep]
                        .into_iter()
                        .all(|angle| dot2(arc.tangent(angle), axis) > 128. * f64::EPSILON)
            }
        }
    }

    pub(crate) fn forward_along(self, axis: Point2) -> bool {
        // Projection derivatives in Bernstein form are convex combinations
        // of the listed control differences. Require nonnegative projection
        // derivatives and a strictly positive total projection.
        let [a, b] = self.endpoints();
        let total = dot2(sub2(b, a), axis);
        let guard = 128. * f64::EPSILON * norm(a).max(norm(b)).max(1.);
        if total <= guard {
            return false;
        }
        match self {
            Self::Line(_) => true,
            Self::Arc(arc) => {
                let worst = axis[1].atan2(axis[0]) + arc.sweep.signum() * FRAC_PI_2;
                !arc.contains(worst)
                    && [arc.start, arc.start + arc.sweep]
                        .into_iter()
                        .all(|angle| dot2(arc.tangent(angle), axis) * arc.radius >= 0.)
            }
            Self::Bezier(p) => p
                .windows(2)
                .all(|pair| dot2(sub2(pair[1], pair[0]), axis) >= 0.),
        }
    }
}

impl Arc {
    fn point(self, angle: f64) -> Point2 {
        let (s, c) = angle.sin_cos();
        [
            self.radius.mul_add(c, self.center[0]),
            self.radius.mul_add(s, self.center[1]),
        ]
    }
    fn tangent(self, angle: f64) -> Point2 {
        let (s, c) = angle.sin_cos();
        let sense = self.sweep.signum();
        [-s * sense, c * sense]
    }
    fn contains(self, angle: f64) -> bool {
        ((angle - self.start) * self.sweep.signum()).rem_euclid(TAU) <= self.sweep.abs()
    }
}

pub(crate) fn dot2(a: Point2, b: Point2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub(crate) fn cross2(a: Point2, b: Point2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
pub(crate) fn norm(a: Point2) -> f64 {
    a[0].hypot(a[1])
}
pub(crate) fn unit2(a: Point2) -> Option<Point2> {
    let n = norm(a);
    (n > 0.).then(|| a.map(|v| v / n))
}
pub(crate) fn sub2(a: Point2, b: Point2) -> Point2 {
    [a[0] - b[0], a[1] - b[1]]
}
pub(crate) fn midpoint(a: Point2, b: Point2) -> Point2 {
    [a[0].midpoint(b[0]), a[1].midpoint(b[1])]
}
pub(crate) fn merge([a, b]: [Point2; 2], [c, d]: [Point2; 2]) -> [Point2; 2] {
    [
        [a[0].min(c[0]), a[1].min(c[1])],
        [b[0].max(d[0]), b[1].max(d[1])],
    ]
}
pub(crate) fn separated([a, b]: [Point2; 2], [c, d]: [Point2; 2], tolerance: f64) -> bool {
    (0..2).any(|i| b[i] + tolerance < c[i] || d[i] + tolerance < a[i])
}
pub(crate) fn diameter([a, b]: [Point2; 2]) -> f64 {
    norm(sub2(b, a))
}

#[cfg(test)]
mod tests;

pub(super) fn authored(
    outline: &[Edge],
    hole: Option<normal_sweep::Edge>,
    tolerance: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<()> {
    if matches!(outline, [Edge::Analytic(normal_sweep::Edge::Circle { .. })]) {
        return Err(ConstructionError::Invalid);
    }
    let mut budget = Budget {
        remaining: MAX_PAIR_WORK,
        interrupt,
    };
    let section = boundaries(outline, tolerance, &mut budget)?;
    if let Some(normal_sweep::Edge::Circle {
        center, radius_mm, ..
    }) = hole
    {
        check(center)?;
        let winding = section
            .iter()
            .flat_map(|boundary| &boundary.pieces)
            .map(|&piece| clear_disk(piece, center, radius_mm + tolerance, 0, &mut budget))
            .sum::<Result<f64>>()?;
        if (winding - TAU).abs() > 1e-7 {
            return Err(ConstructionError::Invalid);
        }
    }
    Ok(())
}

fn boundaries(outline: &[Edge], tolerance: f64, budget: &mut Budget<'_>) -> Result<Vec<Boundary>> {
    let mut section = Vec::with_capacity(outline.len());
    let mut scale: f64 = 1.;
    for edge in outline {
        consume(budget)?;
        let pieces = match edge {
            Edge::Cubic(cubic) => cubic.bezier_spans().map(Piece::Bezier).collect::<Vec<_>>(),
            Edge::Analytic(normal_sweep::Edge::Line { a, b }) => {
                check(*a)?;
                check(*b)?;
                if norm(sub2(*b, *a)) <= tolerance {
                    return Err(ConstructionError::Invalid);
                }
                vec![Piece::Line([*a, *b])]
            }
            Edge::Analytic(edge @ normal_sweep::Edge::Arc { center, a, b, .. }) => {
                for p in [*center, *a, *b] {
                    check(p)?;
                }
                let radius = norm(sub2(*a, *center));
                let max = center
                    .iter()
                    .chain(a)
                    .chain(b)
                    .map(|v| v.abs())
                    .fold(1., f64::max);
                if radius <= tolerance
                    || (norm(sub2(*b, *center)) - radius).abs()
                        > (64. * f64::EPSILON * max).min(tolerance / 64.)
                {
                    return Err(ConstructionError::Unresolved);
                }
                vec![Piece::Arc(Arc {
                    center: *center,
                    radius,
                    start: (a[1] - center[1]).atan2(a[0] - center[0]),
                    sweep: edge.signed_sweep_rad().ok_or(ConstructionError::Invalid)?,
                })]
            }
            Edge::Analytic(normal_sweep::Edge::Circle {
                center,
                radius_mm,
                clockwise,
            }) if outline.len() == 1 => {
                check(*center)?;
                if !radius_mm.is_finite() || *radius_mm <= tolerance {
                    return Err(ConstructionError::Invalid);
                }
                for index in 0..8 {
                    let arc = Arc {
                        center: *center,
                        radius: *radius_mm,
                        start: index as f64 * (FRAC_PI_2 / 2.) * if *clockwise { -1. } else { 1. },
                        sweep: (FRAC_PI_2 / 2.) * if *clockwise { -1. } else { 1. },
                    };
                    let piece = Piece::Arc(arc);
                    let bounds = piece.bounds();
                    scale = scale.max(
                        bounds
                            .into_iter()
                            .flatten()
                            .map(f64::abs)
                            .fold(1., f64::max),
                    );
                    section.push(Boundary {
                        pieces: vec![piece],
                        bounds,
                    });
                }
                continue;
            }
            Edge::Analytic(normal_sweep::Edge::Circle { .. }) => {
                return Err(ConstructionError::Invalid);
            }
        };
        let bounds = pieces
            .iter()
            .map(|piece| piece.bounds())
            .reduce(merge)
            .ok_or(ConstructionError::Invalid)?;
        scale = scale.max(
            bounds
                .into_iter()
                .flatten()
                .map(f64::abs)
                .fold(1., f64::max),
        );
        let [a, b] = [edge.point(0.), edge.point(1.)];
        let chord = unit2(sub2(b, a)).ok_or(ConstructionError::Unresolved)?;
        if pieces.iter().any(|piece| !piece.certified_forward(chord)) {
            return Err(ConstructionError::Unresolved);
        }
        section.push(Boundary { pieces, bounds });
    }
    // The retained subdivision proof uses the source float control values.
    // Refuse charts whose arithmetic enclosure could exhaust its clearance.
    if 128. * f64::EPSILON * scale > tolerance / 4. {
        return Err(ConstructionError::Unresolved);
    }
    for (a, b) in section.iter().zip(section.iter().cycle().skip(1)) {
        consume(budget)?;
        let first = a
            .pieces
            .last()
            .ok_or(ConstructionError::Invalid)?
            .tangent(true);
        let second = b
            .pieces
            .first()
            .ok_or(ConstructionError::Invalid)?
            .tangent(false);
        let axis = unit2([first[0] + second[0], first[1] + second[1]])
            .ok_or(ConstructionError::Unresolved)?;
        if a.pieces
            .iter()
            .chain(&b.pieces)
            .any(|piece| !piece.certified_forward(axis))
        {
            return Err(ConstructionError::Unresolved);
        }
    }
    simple(&section, tolerance, budget)?;
    Ok(section)
}

pub(super) fn authored_rings(
    outline: &[Edge],
    holes: &[Vec<Edge>],
    tolerance: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<()> {
    // Preserve the already-qualified single circular-bore path, including its
    // complete disk clearance certificate, without resetting a multi-ring budget.
    if holes.is_empty() {
        return authored(outline, None, tolerance, interrupt);
    }
    if let [ring] = holes
        && let [Edge::Analytic(circle @ normal_sweep::Edge::Circle { .. })] = ring.as_slice()
    {
        return authored(outline, Some(*circle), tolerance, interrupt);
    }
    let mut budget = Budget {
        remaining: MAX_PAIR_WORK,
        interrupt,
    };
    let rings = std::iter::once(outline)
        .chain(holes.iter().map(Vec::as_slice))
        .map(|ring| boundaries(ring, tolerance, &mut budget))
        .collect::<Result<Vec<_>>>()?;
    for i in 1..rings.len() {
        for j in 0..i {
            for a in &rings[i] {
                for b in &rings[j] {
                    consume(&mut budget)?;
                    if separated(a.bounds, b.bounds, tolerance) {
                        continue;
                    }
                    for &x in &a.pieces {
                        for &y in &b.pieces {
                            clear_pair(x, y, tolerance, 0, &mut budget)?;
                        }
                    }
                }
            }
        }
        let sample = holes[i - 1][0].point(0.);
        for (j, ring) in rings.iter().enumerate() {
            if j == i {
                continue;
            }
            let winding = ring
                .iter()
                .flat_map(|edge| &edge.pieces)
                .map(|&piece| clear_disk(piece, sample, tolerance, 0, &mut budget))
                .sum::<Result<f64>>()?;
            let expected = if j == 0 { TAU } else { 0. };
            if (winding - expected).abs() > 1e-7 {
                return Err(ConstructionError::Invalid);
            }
        }
    }
    Ok(())
}

fn check(point: Point2) -> Result<()> {
    if !point.into_iter().all(f64::is_finite) {
        return Err(ConstructionError::Invalid);
    }
    if point.into_iter().any(|v| v.abs() > 1e8) {
        return Err(ConstructionError::Limit);
    }
    Ok(())
}
