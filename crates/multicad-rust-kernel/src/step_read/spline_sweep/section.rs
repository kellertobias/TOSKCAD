use super::*;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const MAX_PAIR_WORK: usize = 4_000_000;
const MAX_DEPTH: usize = 24;

pub(super) fn verify(
    reader: &Shell<'_, '_>,
    edges: &Edges,
    plane: &Plane,
    face: &FaceWire,
) -> Result<f64> {
    if face.outer.len() < 3
        || face
            .outer
            .iter()
            .map(|edge| edge.start)
            .collect::<BTreeSet<_>>()
            .len()
            != face.outer.len()
        || !face
            .outer
            .iter()
            .any(|edge| matches!(edges[&edge.edge].geometry, Geometry::Cubic { .. }))
    {
        return Err(ReadError::Unsupported);
    }
    let tolerance = reader.tolerance;
    let section = face
        .outer
        .iter()
        .map(|edge| boundary(reader, &edges[&edge.edge].geometry, plane, edge))
        .collect::<Result<Vec<_>>>()?;
    let mut budget = MAX_PAIR_WORK;
    simple(&section, tolerance, &mut budget)?;
    let mut area = face
        .outer
        .iter()
        .map(|edge| section_area(reader, &edges[&edge.edge].geometry, plane, edge))
        .sum::<Result<f64>>()?;
    if let Some(hole) = &face.hole {
        let [edge] = hole.as_slice() else {
            return Err(ReadError::Unsupported);
        };
        let Geometry::Analytic(Curve::Circle(circle)) = edges[&edge.edge].geometry else {
            return Err(ReadError::Unsupported);
        };
        if !circle.within_plane(plane, tolerance)
            || (dot(circle.normal, plane.normal) > 0.) == edge.forward
        {
            return Err(ReadError::Invalid);
        }
        let center = project(plane, circle.center_mm);
        let winding: f64 = section
            .iter()
            .flat_map(|edge| &edge.pieces)
            .map(|&piece| clear_disk(piece, center, circle.radius_mm + tolerance, 0, &mut budget))
            .sum::<Result<f64>>()?;
        if (winding - TAU).abs() > 1e-7 {
            return Err(ReadError::Invalid);
        }
        area -= PI * circle.radius_mm.powi(2);
    }
    if !area.is_finite() || area <= tolerance.powi(2) {
        return Err(ReadError::Invalid);
    }
    Ok(area)
}

fn section_area(
    reader: &Shell<'_, '_>,
    geometry: &Geometry,
    plane: &Plane,
    edge: &Traversal,
) -> Result<f64> {
    let [a, b] = [edge.start, edge.end].map(|v| project(plane, reader.points[&v]));
    match geometry {
        Geometry::Analytic(Curve::Line) => Ok(cross2(a, b) / 2.),
        Geometry::Analytic(curve) => {
            let round = round_geometry(*curve).ok_or(ReadError::Invalid)?;
            let center = project(plane, round.center);
            let theta = directed_sweep(round, plane, edge.forward);
            Ok((cross2(center, sub2(b, a)) + round.radius.powi(2) * theta) / 2.)
        }
        Geometry::Cubic { curve, .. } => {
            let signed = curve.signed_area(plane.origin, plane.normal);
            Ok(if edge.forward { signed } else { -signed })
        }
    }
}

struct Boundary {
    pieces: Vec<Piece>,
    bounds: [Point2; 2],
}

fn boundary(
    reader: &Shell<'_, '_>,
    geometry: &Geometry,
    plane: &Plane,
    edge: &Traversal,
) -> Result<Boundary> {
    let points = [edge.start, edge.end].map(|id| reader.points[&id]);
    let tolerance = reader.tolerance;
    let on_plane = |p: Point| dot(sub(p, plane.origin), plane.normal).abs() <= tolerance;
    if !points.into_iter().all(on_plane) {
        return Err(ReadError::Invalid);
    }
    let [a, b] = points.map(|p| project(plane, p));
    let pieces = match geometry {
        Geometry::Analytic(Curve::Line) => vec![Piece::Line([a, b])],
        Geometry::Analytic(curve) => {
            let round = round_geometry(*curve).ok_or(ReadError::Invalid)?;
            if round.full
                || !on_plane(round.center)
                || length(cross(round.normal, plane.normal)) * round.radius > tolerance
            {
                return Err(ReadError::Invalid);
            }
            let center = project(plane, round.center);
            let [x, y] = sub2(a, center);
            let arc = Arc {
                center,
                radius: round.radius,
                start: y.atan2(x),
                sweep: directed_sweep(round, plane, edge.forward),
            };
            vec![Piece::Arc(arc)]
        }
        Geometry::Cubic { curve, .. } => {
            if !curve.poles_mm().iter().copied().all(on_plane) {
                return Err(ReadError::Invalid);
            }
            let mut spans: Vec<_> = curve
                .bezier_spans()
                .map(|span| span.map(|p| project(plane, p)))
                .collect();
            if !edge.forward {
                spans.reverse();
                for span in &mut spans {
                    span.reverse();
                }
            }
            let chord = unit2(sub2(b, a)).ok_or(ReadError::Invalid)?;
            if spans
                .iter()
                .any(|span| !Piece::Bezier(*span).forward_along(chord))
            {
                return Err(ReadError::Unsupported);
            }
            spans.into_iter().map(Piece::Bezier).collect()
        }
    };
    let bounds = pieces
        .iter()
        .map(|p| p.bounds())
        .reduce(merge)
        .ok_or(ReadError::Invalid)?;
    Ok(Boundary { pieces, bounds })
}

fn directed_sweep(round: Round, plane: &Plane, forward: bool) -> f64 {
    if (dot(round.normal, plane.normal) > 0.) == forward {
        round.sweep
    } else {
        -round.sweep
    }
}

fn simple(section: &[Boundary], tolerance: f64, budget: &mut usize) -> Result<()> {
    // Two consecutive edges may meet only at their common vertex. Prove
    // this by separating the entire pair on the direction of their common
    // travel. Spur joins have a consistent one-sided tangent, including
    // the one-sided limit of a base-circle cubic.
    for (a, b) in section.iter().zip(section.iter().cycle().skip(1)) {
        let a_last = a.pieces.last().ok_or(ReadError::Invalid)?;
        let b_first = b.pieces.first().ok_or(ReadError::Invalid)?;
        let [u, v] = [a_last.tangent(true), b_first.tangent(false)];
        let axis = unit2([u[0] + v[0], u[1] + v[1]]).ok_or(ReadError::Unsupported)?;
        if !a
            .pieces
            .iter()
            .chain(&b.pieces)
            .all(|piece| piece.forward_along(axis))
        {
            return Err(ReadError::Unsupported);
        }
    }
    // For every non-adjacent pair, use certified interval boxes. Refuse
    // touching, crossing or unresolved curves within the numerical budget.
    for (i, a) in section.iter().enumerate() {
        for (j, b) in section.iter().enumerate().skip(i + 2) {
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

fn clear_pair(a: Piece, b: Piece, tolerance: f64, depth: usize, budget: &mut usize) -> Result<()> {
    consume(budget)?;
    let [a_box, b_box] = [a.bounds(), b.bounds()];
    if separated(a_box, b_box, tolerance) {
        return Ok(());
    }
    if depth == MAX_DEPTH || (diameter(a_box).max(diameter(b_box)) <= tolerance) {
        return Err(ReadError::Unsupported);
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

fn clear_disk(
    piece: Piece,
    center: Point2,
    radius: f64,
    depth: usize,
    budget: &mut usize,
) -> Result<f64> {
    consume(budget)?;
    let [a, b] = piece.bounds();
    let closest = std::array::from_fn(|i| center[i].clamp(a[i], b[i]));
    if norm(sub2(closest, center)) > radius {
        let [a, b] = piece.endpoints().map(|p| sub2(p, center));
        return Ok(cross2(a, b).atan2(dot2(a, b)));
    }
    if depth == MAX_DEPTH || diameter([a, b]) <= radius * 1e-8 {
        return Err(ReadError::Unsupported);
    }
    piece
        .split()
        .into_iter()
        .map(|piece| clear_disk(piece, center, radius, depth + 1, budget))
        .sum()
}

fn consume(budget: &mut usize) -> Result<()> {
    *budget = budget.checked_sub(1).ok_or(ReadError::Limit)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum Piece {
    Line([Point2; 2]),
    Arc(Arc),
    Bezier([Point2; 4]),
}

#[derive(Clone, Copy)]
struct Arc {
    center: Point2,
    radius: f64,
    start: f64,
    sweep: f64,
}

impl Piece {
    fn endpoints(self) -> [Point2; 2] {
        match self {
            Self::Line(points) => points,
            Self::Arc(arc) => [arc.point(arc.start), arc.point(arc.start + arc.sweep)],
            Self::Bezier([a, _, _, b]) => [a, b],
        }
    }

    fn bounds(self) -> [Point2; 2] {
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

    fn forward_along(self, axis: Point2) -> bool {
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

fn project(plane: &Plane, p: Point) -> Point2 {
    let d = sub(p, plane.origin);
    [dot(d, plane.u), dot(d, plane.v)]
}
fn dot2(a: Point2, b: Point2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross2(a: Point2, b: Point2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn norm(a: Point2) -> f64 {
    a[0].hypot(a[1])
}
fn unit2(a: Point2) -> Option<Point2> {
    let n = norm(a);
    (n > 0.).then(|| a.map(|v| v / n))
}
fn sub2(a: Point2, b: Point2) -> Point2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn midpoint(a: Point2, b: Point2) -> Point2 {
    [a[0].midpoint(b[0]), a[1].midpoint(b[1])]
}
fn merge([a, b]: [Point2; 2], [c, d]: [Point2; 2]) -> [Point2; 2] {
    [
        [a[0].min(c[0]), a[1].min(c[1])],
        [b[0].max(d[0]), b[1].max(d[1])],
    ]
}
fn separated([a, b]: [Point2; 2], [c, d]: [Point2; 2], tolerance: f64) -> bool {
    (0..2).any(|i| b[i] + tolerance < c[i] || d[i] + tolerance < a[i])
}
fn diameter([a, b]: [Point2; 2]) -> f64 {
    norm(sub2(b, a))
}

#[cfg(test)]
mod tests;
