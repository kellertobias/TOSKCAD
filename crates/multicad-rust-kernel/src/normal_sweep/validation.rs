use super::*;

#[derive(Clone, Copy)]
struct Line {
    start: Point,
    unit: Point,
    length: f64,
}

impl Line {
    fn of(edge: Edge) -> Self {
        let start = edge.start();
        let delta = sub(edge.end(), start);
        let length = length(delta);
        Self {
            start,
            unit: scale(delta, 1. / length),
            length,
        }
    }

    fn point(self, travel: f64) -> Point {
        add(self.start, scale(self.unit, travel))
    }

    fn contains(self, travel: f64, slack: f64) -> bool {
        (-slack..=self.length + slack).contains(&travel)
    }
}

#[derive(Default)]
struct Hits {
    points: [Option<Point>; 2],
    overlap: bool,
}

impl Hits {
    fn one(point: Point) -> Self {
        Self {
            points: [Some(point), None],
            overlap: false,
        }
    }
}

fn line_hits(a: Line, b: Line, slack: f64) -> Hits {
    let delta = sub(b.start, a.start);
    let angle = cross(a.unit, b.unit);
    if angle.abs() <= 32. * f64::EPSILON {
        let transverse = cross(a.unit, delta).abs();
        if transverse > slack {
            return Hits::default();
        }
        let [p, q] = [b.start, b.point(b.length)].map(|point| dot(a.unit, sub(point, a.start)));
        let lo = p.min(q).max(0.);
        let hi = p.max(q).min(a.length);
        if hi - lo > slack {
            return Hits {
                overlap: true,
                ..Hits::default()
            };
        }
        if hi + slack >= lo {
            return Hits::one(a.point(lo.midpoint(hi)));
        }
        return Hits::default();
    }
    let along_a = cross(delta, b.unit) / angle;
    let along_b = cross(delta, a.unit) / angle;
    if a.contains(along_a, slack) && b.contains(along_b, slack) {
        Hits::one(a.point(along_a))
    } else {
        Hits::default()
    }
}

fn circle_chord(radius: f64, offset: f64, roundoff: f64) -> f64 {
    // A near-zero discriminant from a true tangent amplifies roundoff
    // under the square root. Collapse that roundoff interval only.
    if radius - offset.abs() <= roundoff {
        0.
    } else {
        ((radius - offset.abs()) * (radius + offset.abs())).sqrt()
    }
}

fn line_round_hits(line: Line, round: Round, slack: f64, roundoff: f64) -> Hits {
    let delta = sub(round.center, line.start);
    let offset = cross(line.unit, delta);
    if offset.abs() > round.radius + slack {
        return Hits::default();
    }
    let axis = dot(line.unit, delta);
    let half = circle_chord(round.radius, offset, roundoff);
    let points = [axis - half, axis + half].map(|travel| {
        let point = line.point(travel);
        (line.contains(travel, slack) && round.contains_direction(sub(point, round.center), slack))
            .then_some(point)
    });
    Hits {
        points,
        overlap: false,
    }
}

fn round_hits(a: Round, b: Round, slack: f64, roundoff: f64) -> Hits {
    let delta = sub(b.center, a.center);
    let distance = length(delta);
    if distance <= roundoff {
        if (a.radius - b.radius).abs() > slack {
            return Hits::default();
        }
        let overlap = |a: Round, b: Round| {
            [0., 0.5, 1.].into_iter().any(|fraction| {
                let angle = a.start + fraction * a.sweep;
                let travel = b.travel(angle);
                let margin = slack / b.radius;
                travel > margin && travel + margin < b.sweep.abs()
            })
        };
        return Hits {
            points: [None; 2],
            overlap: overlap(a, b) || overlap(b, a),
        };
    }
    if distance > a.radius + b.radius + slack || distance + slack < (a.radius - b.radius).abs() {
        return Hits::default();
    }
    let d = scale(delta, 1. / distance);
    let axis = ((a.radius - b.radius) * (a.radius + b.radius) / distance + distance) / 2.;
    let mid = add(a.center, scale(d, axis));
    let half = circle_chord(a.radius, axis, roundoff);
    let [x, y] = scale(d, half);
    let points = [add(mid, [-y, x]), add(mid, [y, -x])].map(|point| {
        (a.contains_direction(sub(point, a.center), slack)
            && b.contains_direction(sub(point, b.center), slack))
        .then_some(point)
    });
    Hits {
        points,
        overlap: false,
    }
}

fn near_interiors(a: Edge, b: Edge, clearance: f64) -> bool {
    fn test(round: Round, other: Edge, vector: Point, clearance: f64) -> bool {
        let axis_length = length(vector);
        if axis_length == 0. {
            // Coincident centers: endpoints also test angular overlap; the
            // midpoint covers the equal full-circle parameter seam.
            return other.distance(round.point(0.5)) <= clearance;
        }
        let offset = scale(vector, round.radius / axis_length);
        [offset, scale(offset, -1.)].into_iter().any(|radial| {
            round.contains_direction(radial, 0.)
                && other.distance(add(round.center, radial)) <= clearance
        })
    }
    match (a.round(), b.round()) {
        (None, None) => false,
        (Some(round), None) => {
            let [x, y] = sub(b.end(), b.start());
            test(round, b, [-y, x], clearance)
        }
        (None, Some(round)) => {
            let [x, y] = sub(a.end(), a.start());
            test(round, a, [-y, x], clearance)
        }
        (Some(a_round), Some(b_round)) => {
            let delta = sub(a_round.center, b_round.center);
            test(a_round, b, delta, clearance) || test(b_round, a, delta, clearance)
        }
    }
}

fn clear_pair(a: Edge, b: Edge, joins: &[Point], slack: f64, roundoff: f64) -> bool {
    let hits = match (a.round(), b.round()) {
        (None, None) => line_hits(Line::of(a), Line::of(b), slack),
        (None, Some(round)) => line_round_hits(Line::of(a), round, slack, roundoff),
        (Some(round), None) => line_round_hits(Line::of(b), round, slack, roundoff),
        (Some(a), Some(b)) => round_hits(a, b, slack, roundoff),
    };
    if hits.overlap
        || hits
            .points
            .into_iter()
            .flatten()
            .any(|point| !joins.iter().any(|&join| length(sub(point, join)) <= slack))
    {
        return false;
    }
    !joins.is_empty()
        || ([a.start(), a.end()]
            .into_iter()
            .all(|p| b.distance(p) > slack)
            && [b.start(), b.end()]
                .into_iter()
                .all(|p| a.distance(p) > slack)
            && !near_interiors(a, b, slack))
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Point,
    max: Point,
}

impl Bounds {
    fn of(edge: Edge) -> Self {
        Self {
            min: [-edge.support([-1., 0.]), -edge.support([0., -1.])],
            max: [edge.support([1., 0.]), edge.support([0., 1.])],
        }
    }

    fn overlaps(self, other: Self, slack: f64) -> bool {
        (0..2).all(|i| self.min[i] <= other.max[i] + slack && other.min[i] <= self.max[i] + slack)
    }
}

fn coordinates(edge: Edge) -> [Point; 3] {
    let [a, b] = [edge.start(), edge.end()];
    let center = match edge {
        Edge::Line { .. } => a,
        Edge::Arc { center, .. } | Edge::Circle { center, .. } => center,
    };
    [a, b, center]
}

fn normalize(ring: &mut [Edge], slack: f64) -> Result<(), Error> {
    if ring.is_empty()
        || (ring.len() == 1) != ring.iter().any(|edge| matches!(edge, Edge::Circle { .. }))
    {
        return Err(Error::InvalidProfile);
    }
    for i in 0..ring.len() {
        let j = (i + 1) % ring.len();
        let [end, start] = [ring[i].end(), ring[j].start()];
        if length(sub(end, start)) > slack {
            return Err(Error::InvalidProfile);
        }
        // Retain an arc endpoint in preference to an adjacent straight edge.
        let join = if matches!(ring[i], Edge::Arc { .. }) {
            end
        } else {
            start
        };
        match &mut ring[i] {
            Edge::Line { b, .. } | Edge::Arc { b, .. } => *b = join,
            Edge::Circle { .. } => {}
        }
        match &mut ring[j] {
            Edge::Line { a, .. } | Edge::Arc { a, .. } => *a = join,
            Edge::Circle { .. } => {}
        }
    }
    Ok(())
}

pub(super) fn validate(rings: &mut [Vec<Edge>], tolerance: f64, minimum: f64) -> Result<(), Error> {
    let mut scale = 1_f64;
    for edge in rings.iter().flatten() {
        for component in coordinates(*edge).into_iter().flatten() {
            if !component.is_finite() {
                return Err(Error::InvalidProfile);
            }
            if component.abs() > MAX_DIMENSION_MM {
                return Err(Error::Limit);
            }
            scale = scale.max(component.abs());
        }
    }
    let roundoff = 32. * f64::EPSILON * scale;
    let slack = tolerance.max(roundoff);
    for ring in rings.iter_mut() {
        normalize(ring, slack)?;
        for &edge in ring.iter() {
            if edge.length_mm() < minimum {
                return Err(Error::InvalidProfile);
            }
            if let Some(round) = edge.round()
                && (round.radius < minimum
                    || (length(sub(edge.end(), round.center)) - round.radius).abs() > slack
                    || round.sweep.abs() < TAU && length(sub(edge.start(), edge.end())) < minimum)
            {
                return Err(Error::InvalidProfile);
            }
            let bounds = Bounds::of(edge);
            if bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .any(|v| v.abs() > MAX_DIMENSION_MM)
            {
                return Err(Error::Limit);
            }
        }
    }
    let bounds: Vec<Vec<Bounds>> = rings
        .iter()
        .map(|ring| ring.iter().copied().map(Bounds::of).collect())
        .collect();
    for (i, ring) in rings.iter().enumerate() {
        for (j, &edge) in ring.iter().enumerate() {
            for (p, earlier) in rings[..=i].iter().enumerate() {
                let end = if p == i { j } else { earlier.len() };
                for (q, &other) in earlier[..end].iter().enumerate() {
                    if !bounds[i][j].overlaps(bounds[p][q], slack) {
                        continue;
                    }
                    let mut joins = [[0.; 2]; 2];
                    let mut count = 0;
                    if p == i && q + 1 == j {
                        joins[count] = edge.start();
                        count += 1;
                    }
                    if p == i && q == 0 && j + 1 == ring.len() {
                        joins[count] = edge.end();
                        count += 1;
                    }
                    if !clear_pair(edge, other, &joins[..count], slack, roundoff) {
                        return Err(Error::InvalidProfile);
                    }
                }
            }
        }
        if i > 0
            && (!inside(&rings[0], ring[0].start())
                || rings[1..i]
                    .iter()
                    .any(|other| inside(other, ring[0].start()) || inside(ring, other[0].start())))
        {
            return Err(Error::InvalidProfile);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: Point, b: Point) -> Edge {
        Edge::Line { a, b }
    }

    fn circle(center: Point, radius_mm: f64) -> Edge {
        Edge::Circle {
            center,
            radius_mm,
            clockwise: false,
        }
    }

    fn clear(a: Edge, b: Edge, joins: &[Point]) -> bool {
        clear_pair(a, b, joins, 1e-7, 1e-14)
    }

    #[test]
    fn parallel_wires_must_not_overlap_and_joins_are_local() {
        let segment = line([0., 0.], [4., 0.]);
        assert!(clear(segment, line([0., 1.], [4., 1.]), &[]));
        assert!(!clear(segment, line([2., 0.], [8., 0.]), &[[4., 0.]]));
        assert!(clear(segment, line([4., 0.], [8., 0.]), &[[4., 0.]]));
        assert!(clear(segment, line([5., 0.], [8., 0.]), &[]));
        assert!(clear(segment, line([5., -1.], [5., 1.]), &[]));
        assert!(!clear(segment, line([2., -1.], [2., 1.]), &[]));
    }

    #[test]
    fn radial_clearance_checks_finite_arcs_and_concentric_curves() {
        let disk = circle([0., 0.], 2.);
        assert!(clear(line([-3., 3.], [3., 3.]), disk, &[]));
        assert!(clear(disk, circle([0., 0.], 1.), &[]));
        assert!(!clear(disk, circle([0., 0.], 2.), &[]));
        let a = [2., 0.];
        let b = [-2., 0.];
        let half = Edge::Arc {
            center: [0.; 2],
            a,
            b,
            clockwise: false,
        };
        let other_half = Edge::Arc {
            center: [0.; 2],
            a: b,
            b: a,
            clockwise: false,
        };
        assert!(clear(half, other_half, &[a, b]));
        assert!(clear(half, line([-3., -1.], [3., -1.]), &[]));
        assert!(!clear(half, line([-3., 1.], [3., 1.]), &[]));
        assert!(clear(disk, circle([0.25, 0.], 1.), &[]));
        assert!(!clear(disk, circle([1.25, 0.], 1.), &[]));
    }
}
