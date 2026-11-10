//! Clip oriented section loops while preserving their exact conic trims.
use super::*;
use std::collections::BTreeSet;

const MAX_PLANES: usize = 4;

/// Keep the half-space into which `inward_normal` points. For a section
/// loop this normal must lie in the section's plane.
#[derive(Clone, Copy, Debug)]
pub struct SectionClipPlane {
    pub origin_mm: Point,
    pub inward_normal: Point,
}

/// Clip the closed, material-left loops returned by a shell section.
/// Transverse cuts preserve their line/ellipse geometry and reconnect the
/// boundary with straight, material-left connectors. Unresolved contact
/// with an existing trim is refused.
pub fn clip_section_loops(
    mut loops: Vec<Vec<SectionEdge>>,
    section_normal: Point,
    planes: &[SectionClipPlane],
    tolerance_mm: f64,
    segment_limit: usize,
) -> Result<Vec<Vec<SectionEdge>>, Error> {
    if !(1e-7..=1e-3).contains(&tolerance_mm) {
        return Err(Error::Invalid);
    }
    if planes.len() > MAX_PLANES || loops.iter().map(Vec::len).sum::<usize>() > segment_limit {
        return Err(Error::Limit);
    }
    let normal = unit(section_normal)?;
    let origin = loops
        .first()
        .and_then(|ring| ring.first())
        .map_or([0.; 3], |edge| edge.ends_mm[0]);
    validate(&loops, origin, normal, tolerance_mm)?;
    for plane in planes {
        let cut = Cut::new(*plane, normal, tolerance_mm)?;
        loops = cut.apply(loops, segment_limit)?;
    }
    Ok(loops)
}

fn bounded(point: Point) -> bool {
    point.into_iter().all(|x| x.is_finite() && x.abs() <= 1e8)
}

fn validate(
    loops: &[Vec<SectionEdge>],
    origin: Point,
    normal: Point,
    tolerance: f64,
) -> Result<(), Error> {
    let in_plane = |point| bounded(point) && dot(sub(point, origin), normal).abs() <= tolerance;
    for ring in loops {
        if ring.is_empty() {
            return Err(Error::Invalid);
        }
        for (edge, next) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let [a, b] = edge.ends_mm;
            if !in_plane(a) || !in_plane(b) || b != next.ends_mm[0] {
                return Err(Error::Invalid);
            }
            match edge.curve {
                Curve::Line if length3(sub(a, b)) <= tolerance => return Err(Error::Invalid),
                Curve::Ellipse(arc) => {
                    let ellipse = arc.ellipse();
                    if !in_plane(ellipse.center_mm())
                        || length3(cross(ellipse.normal(), normal)) > ANGULAR
                        || arc
                            .endpoints()
                            .into_iter()
                            .zip([a, b])
                            .any(|(p, q)| length3(sub(p, q)) > tolerance)
                    {
                        return Err(Error::Invalid);
                    }
                }
                Curve::Line => {}
            }
        }
    }
    Ok(())
}

struct Cut {
    origin: Point,
    inward: Point,
    tangent: Point,
    tolerance: f64,
    plane: Plane,
}

impl Cut {
    fn new(spec: SectionClipPlane, normal: Point, tolerance: f64) -> Result<Self, Error> {
        let inward = unit(spec.inward_normal)?;
        if !bounded(spec.origin_mm) || dot(normal, inward).abs() > ANGULAR {
            return Err(Error::Invalid);
        }
        let origin = spec.origin_mm;
        let plane = Plane::new(origin, inward).map_err(|_| Error::Invalid)?;
        Ok(Self {
            origin,
            inward,
            tangent: cross(inward, normal),
            tolerance,
            plane,
        })
    }

    fn distance(&self, point: Point) -> f64 {
        dot(sub(point, self.origin), self.inward)
    }

    fn range(&self, edge: SectionEdge) -> [f64; 2] {
        match edge.curve {
            Curve::Line => {
                let [a, b] = edge.ends_mm.map(|point| self.distance(point));
                [a.min(b), a.max(b)]
            }
            Curve::Ellipse(arc) => arc
                .range_along(self.inward)
                .map(|value| value - dot(self.origin, self.inward)),
        }
    }

    fn apply(
        &self,
        loops: Vec<Vec<SectionEdge>>,
        segment_limit: usize,
    ) -> Result<Vec<Vec<SectionEdge>>, Error> {
        if loops.is_empty() {
            return Ok(loops);
        }
        let [lo, hi] =
            loops
                .iter()
                .flatten()
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], &edge| {
                    let [a, b] = self.range(edge);
                    [lo.min(a), hi.max(b)]
                });
        if lo > self.tolerance {
            return Ok(loops);
        }
        if hi < -self.tolerance {
            return Ok(Vec::new());
        }
        let mut graph = Graph::new(segment_limit);
        let mut crossings = Vec::new();
        for edge in loops.into_iter().flatten() {
            self.split(edge, &mut graph, &mut crossings)?;
        }
        let along = |p| dot(sub(p, self.origin), self.tangent);
        crossings.sort_unstable_by(|&a, &b| along(a).total_cmp(&along(b)));
        let (pairs, remainder) = crossings.as_chunks::<2>();
        if !remainder.is_empty()
            || crossings
                .windows(2)
                .any(|pair| along(pair[1]) - along(pair[0]) <= self.tolerance)
        {
            return Err(Error::Unsupported);
        }
        for &[a, b] in pairs {
            // The direction section-normal × outward-normal keeps material
            // to the left. A reversed or unmatched port cannot close.
            graph.add(SectionEdge {
                ends_mm: [a, b],
                curve: Curve::Line,
            })?;
        }
        graph.loops()
    }

    fn split(
        &self,
        edge: SectionEdge,
        graph: &mut Graph,
        crossings: &mut Vec<Point>,
    ) -> Result<(), Error> {
        let [a, b] = edge.ends_mm;
        let [da, db] = edge.ends_mm.map(|p| self.distance(p));
        if da.abs().min(db.abs()) <= self.tolerance {
            return Err(Error::Unsupported);
        }
        let events = match edge.curve {
            Curve::Line => {
                let event = ((da > 0.) != (db > 0.)).then(|| {
                    let t = da / (da - db);
                    (t, translate(a, sub(b, a), t))
                });
                [event, None]
            }
            Curve::Ellipse(arc) => self.conic_events(arc)?,
        };
        let mut start = (0., a);
        for (t, point) in events.into_iter().flatten() {
            let offset = self.distance(point);
            if offset.abs() > self.tolerance {
                return Err(Error::Unsupported);
            }
            // Reuse one canonical root for both the curve and connector.
            let point = translate(point, self.inward, -offset);
            self.fragment(edge, start, (t, point), graph)?;
            crossings.push(point);
            start = (t, point);
        }
        self.fragment(edge, start, (1., b), graph)
    }

    fn conic_events(&self, arc: EllipseArc) -> Result<[Option<(f64, Point)>; 2], Error> {
        let ellipse = arc.ellipse();
        let sweep = arc.sweep_rad();
        let locate = |angle| {
            let t = travel(arc, angle) / sweep.abs();
            (0. < t && t < 1.).then(|| (t, ellipse.point_at(angle)))
        };
        match ellipse
            .plane_crossings(self.plane, self.tolerance)
            .map_err(|_| Error::Unsupported)?
        {
            PlaneCrossings::None => Ok([None; 2]),
            PlaneCrossings::Tangent(angle) if locate(angle).is_none() => Ok([None; 2]),
            PlaneCrossings::Tangent(_) | PlaneCrossings::Coincident => Err(Error::Unsupported),
            PlaneCrossings::Crossing(angles) => {
                let mut events = angles.map(locate);
                events.sort_unstable_by(|a, b| {
                    a.map_or(f64::INFINITY, |(t, _)| t)
                        .total_cmp(&b.map_or(f64::INFINITY, |(t, _)| t))
                });
                Ok(events)
            }
        }
    }

    fn fragment(
        &self,
        edge: SectionEdge,
        (start, a): (f64, Point),
        (end, b): (f64, Point),
        graph: &mut Graph,
    ) -> Result<(), Error> {
        let mid = start.midpoint(end);
        let point = match edge.curve {
            Curve::Line => translate(a, sub(b, a), 0.5),
            Curve::Ellipse(arc) => arc
                .ellipse()
                .point_at(arc.sweep_rad().mul_add(mid, arc.start_rad())),
        };
        let distance = self.distance(point);
        if distance.abs() <= self.tolerance {
            return Err(Error::Unsupported);
        }
        if distance < 0. {
            return Ok(());
        }
        let curve = match edge.curve {
            Curve::Ellipse(arc) if start != 0. || end != 1. => {
                let sweep = arc.sweep_rad();
                Curve::Ellipse(
                    EllipseArc::new(
                        arc.ellipse(),
                        sweep.mul_add(start, arc.start_rad()),
                        sweep * (end - start),
                    )
                    .map_err(|_| Error::Unsupported)?,
                )
            }
            curve => curve,
        };
        graph.add(SectionEdge {
            ends_mm: [a, b],
            curve,
        })
    }
}

type Key = [u64; 3];

fn key(point: Point) -> Key {
    point.map(|value| if value == 0. { 0 } else { value.to_bits() })
}

struct Graph {
    remaining: usize,
    outgoing: BTreeMap<Key, SectionEdge>,
    incoming: BTreeSet<Key>,
}

impl Graph {
    fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            outgoing: BTreeMap::new(),
            incoming: BTreeSet::new(),
        }
    }

    fn add(&mut self, edge: SectionEdge) -> Result<(), Error> {
        self.remaining = self.remaining.checked_sub(1).ok_or(Error::Limit)?;
        let [a, b] = edge.ends_mm.map(key);
        if !self.incoming.insert(b) || self.outgoing.insert(a, edge).is_some() {
            return Err(Error::Unsupported);
        }
        Ok(())
    }

    fn loops(mut self) -> Result<Vec<Vec<SectionEdge>>, Error> {
        let mut loops = Vec::new();
        while let Some((&start, _)) = self.outgoing.first_key_value() {
            let mut ring = Vec::new();
            let mut cursor = start;
            loop {
                let edge = self.outgoing.remove(&cursor).ok_or(Error::Unsupported)?;
                cursor = key(edge.ends_mm[1]);
                ring.push(edge);
                if cursor == start {
                    break;
                }
            }
            loops.push(ring);
        }
        Ok(loops)
    }
}

#[cfg(test)]
mod tests;
