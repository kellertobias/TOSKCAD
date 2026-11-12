//! Shared side-cycle ownership and certified single angular cap wraps.
use super::*;
#[cfg(test)]
mod tests;
use std::collections::{BTreeMap, BTreeSet};
use trims::{binomial, split};
type Group = (usize, Vec<(usize, bool)>);
#[derive(Clone)]
pub struct CapBoundary {
    pub(super) axial: AxialBoundary,
    pub(super) sides: Vec<usize>,
    pub(super) kinds: Vec<SideKind>,
    pub(super) rails: Vec<(usize, Vec<[I; 3]>)>,
    pub(super) rims: [Vec<Vec<(usize, bool)>>; 2],
}
#[derive(Clone, Copy)]
pub(super) enum SideKind {
    Tensor(usize),
    Cone,
}
impl CapBoundary {
    pub fn input(&self) -> &BoundaryInput {
        self.axial.input()
    }
}
impl AxialBoundary {
    pub fn certify_caps(self, interrupt: Option<&dyn Fn() -> bool>) -> Result<CapBoundary, Error> {
        let mut work = Work {
            remaining: 2_000_000,
            interrupt,
        };
        work.step()?;
        let input = self.input();
        let mut tensor_rims = [BTreeMap::new(), BTreeMap::new()];
        for t in &self.tensors {
            for (end, rims) in tensor_rims.iter_mut().enumerate() {
                let (edge, direction) = t.rims[end];
                insert_rim(rims, edge, t.face, direction)?;
            }
        }
        let mut rings = [Vec::new(), Vec::new()];
        let mut cycles = [Vec::new(), Vec::new()];
        let tensor_faces = self.tensors.iter().map(|t| t.face).collect::<BTreeSet<_>>();
        let cone_faces = self.cones.iter().copied().collect::<BTreeSet<_>>();
        for end in 0..2 {
            let face = self.caps[end];
            let cap = &input.faces[face];
            let wire = if end == 0 {
                cap.outer
                    .iter()
                    .rev()
                    .map(|&(e, d)| (e, !d))
                    .collect::<Vec<_>>()
            } else {
                cap.outer.clone()
            };
            let mut owners = Vec::with_capacity(wire.len());
            for &(edge, direction) in &wire {
                work.step()?;
                let [a, b] = self.boundary.ownership.topology.edge_faces[edge];
                // This cap.outer use was one of the two owners certified
                // from this same immutable face/edge graph.
                let other = if a == face { b } else { a };
                check_cap_owner(input, &tensor_rims[end], other, edge, direction)?;
                owners.push(other);
            }
            let start = (0..wire.len())
                .find(|&i| owners[i] != owners[(i + wire.len() - 1) % wire.len()])
                .ok_or(Error::Invalid)?;
            let mut groups = Vec::<Group>::new();
            for j in 0..wire.len() {
                let i = (start + j) % wire.len();
                let owner = owners[i];
                if groups.last().is_none_or(|g| g.0 != owner) {
                    groups.push((owner, Vec::new()));
                }
                groups.last_mut().ok_or(Error::Invalid)?.1.push(wire[i]);
            }
            check_groups(&groups, &tensor_faces, &cone_faces, &mut work)?;
            let mut angle = I::point(0.);
            for (side, edges) in groups {
                for &(edge, direction) in &edges {
                    angle = angle + cap_curve(&self, edge, direction, &mut work)?;
                }
                cycles[end].push(side);
                rings[end].push(edges);
            }
            let pi = I::new(
                std::f64::consts::PI.next_down(),
                std::f64::consts::PI.next_up(),
            );
            if angle.lo <= pi.hi || angle.hi >= (I::point(3.) * pi).lo {
                return Err(Error::Unresolved);
            }
        }
        let count = cycles[0].len();
        let offset = align_cycles(&cycles[0], &cycles[1])?;
        rings[1].rotate_left(offset);
        let sides = cycles[0].clone();
        // Group completeness proved every side belongs to exactly one of
        // the typed tensor/cone collections. Keep that kind for consumers.
        let kinds = sides
            .iter()
            .map(|side| {
                self.tensors
                    .iter()
                    .position(|t| t.face == *side)
                    .map_or(SideKind::Cone, SideKind::Tensor)
            })
            .collect();
        let mut rails = Vec::with_capacity(count);
        for i in 0..count {
            work.step()?;
            let previous = sides[(i + count - 1) % count];
            let side = sides[i];
            let a = ends(input, rings[0][i][0]).0;
            let b = ends(input, rings[1][i][0]).0;
            let (edge, points) = connecting_rail(
                input,
                &self.boundary.ownership.topology.edge_faces,
                [a, b],
                [previous, side],
                &mut work,
            )?;
            rails.push((edge, project_rail(points, &self.chart, &mut work)?));
        }
        for &cone in &self.cones {
            let i = sides
                .iter()
                .position(|s| *s == cone)
                .ok_or(Error::Invalid)?;
            let expected = std::iter::once(rails[i].0)
                .chain(std::iter::once(rails[(i + 1) % count].0))
                .chain(rings[0][i].iter().chain(&rings[1][i]).map(|e| e.0))
                .collect::<BTreeSet<_>>();
            let mut run = 0;
            for offset in 1..=count {
                work.step()?;
                if cone_faces.contains(&sides[(i + offset) % count]) {
                    break;
                }
                run += 1;
            }
            check_cone_partition(&input.faces[cone].outer, &expected, run)?;
        }
        Ok(CapBoundary {
            axial: self,
            sides,
            kinds,
            rails,
            rims: rings,
        })
    }
}
fn check_groups(
    groups: &[Group],
    tensors: &BTreeSet<usize>,
    cones: &BTreeSet<usize>,
    work: &mut Work<'_>,
) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for (side, edges) in groups {
        work.step()?;
        if !seen.insert(*side) {
            return Err(Error::Invalid);
        }
        let expected = if tensors.contains(side) { 1 } else { 2 };
        if edges.len() != expected {
            return Err(Error::Invalid);
        }
    }
    if seen != tensors.union(cones).copied().collect() {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn align_cycles(a: &[usize], b: &[usize]) -> Result<usize, Error> {
    let first = a.first().ok_or(Error::Invalid)?;
    let offset = b.iter().position(|v| v == first).ok_or(Error::Invalid)?;
    if a.len() != b.len() || (0..a.len()).any(|i| a[i] != b[(offset + i) % a.len()]) {
        return Err(Error::Invalid);
    }
    Ok(offset)
}
fn check_cone_partition(
    wire: &[(usize, bool)],
    expected: &BTreeSet<usize>,
    run: usize,
) -> Result<(), Error> {
    if wire.iter().map(|e| e.0).collect::<BTreeSet<_>>() != *expected {
        return Err(Error::Invalid);
    }
    if run != 3 {
        return Err(Error::Unsupported);
    }
    Ok(())
}
pub(super) fn ends(input: &BoundaryInput, (edge, forward): (usize, bool)) -> (usize, usize) {
    let [a, b] = input.edges[edge].vertices;
    if forward { (a, b) } else { (b, a) }
}
pub(super) fn phase(a: [I; 3], b: [I; 3]) -> Option<I> {
    let c = (a[0] * b[1] - a[1] * b[0]).positive().ok()?;
    let d = a[0] * b[0] + a[1] * b[1];
    let norm = (c.square() + d.square()).sqrt();
    let two = I::point(2.);
    let value = if d.midpoint() >= 0. {
        two * (c / (norm + d).positive().ok()?).atan()
    } else {
        I::new(
            std::f64::consts::PI.next_down(),
            std::f64::consts::PI.next_up(),
        ) - two * (c / (norm - d).positive().ok()?).atan()
    };
    value.checked().ok()
}
fn insert_rim(
    rims: &mut BTreeMap<usize, (usize, bool)>,
    edge: usize,
    face: usize,
    direction: bool,
) -> Result<(), Error> {
    if rims.insert(edge, (face, direction)).is_some() {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn check_cap_owner(
    input: &BoundaryInput,
    rims: &BTreeMap<usize, (usize, bool)>,
    other: usize,
    edge: usize,
    direction: bool,
) -> Result<(), Error> {
    match &input.surfaces[input.faces[other].surface] {
        Surface::Tensor(_) => {
            if rims.get(&edge) != Some(&(other, direction)) {
                return Err(Error::Invalid);
            }
        }
        Surface::Cone { .. } => {
            if !matches!(input.curves[input.edges[edge].curve], Curve::Circle { .. }) {
                return Err(Error::Invalid);
            }
        }
        Surface::Plane(_) => return Err(Error::Invalid),
    }
    Ok(())
}
fn project_rail(
    points: &[Point],
    chart: &axial::Chart,
    work: &mut Work<'_>,
) -> Result<Vec<[I; 3]>, Error> {
    let mut projected = Vec::with_capacity(points.len());
    for chunk in points.chunks(128) {
        work.charge(chunk.len())?;
        projected.extend(chunk.iter().map(|&p| chart.project(p)));
    }
    Ok(projected)
}
fn connecting_rail<'a>(
    input: &'a BoundaryInput,
    owners: &[[usize; 2]],
    vertices: [usize; 2],
    sides: [usize; 2],
    work: &mut Work<'_>,
) -> Result<(usize, &'a [Point]), Error> {
    let mut candidates = Vec::new();
    for (e, v) in input.edges.iter().enumerate() {
        work.step()?;
        if (v.vertices == vertices || v.vertices == [vertices[1], vertices[0]])
            && sides.iter().all(|s| owners[e].contains(s))
        {
            candidates.push(e);
        }
    }
    let [edge] = candidates[..] else {
        return Err(Error::Unresolved);
    };
    let Curve::Spline { poles, .. } = &input.curves[input.edges[edge].curve] else {
        return Err(Error::Invalid);
    };
    Ok((edge, poles))
}

fn cap_curve(
    boundary: &AxialBoundary,
    edge: usize,
    forward: bool,
    work: &mut Work<'_>,
) -> Result<I, Error> {
    let input = boundary.input();
    let e = input.edges[edge];
    match &input.curves[e.curve] {
        Curve::Spline { spans, poles } => {
            let mut points = poles
                .iter()
                .map(|&p| boundary.chart.project(p))
                .collect::<Vec<_>>();
            if e.forward != forward {
                points.reverse();
            }
            positive_spline(&points, *spans, work)
        }
        Curve::Circle { axis, radius_mm } => {
            let (a, b) = ends(input, (edge, forward));
            circle_angle(
                &boundary.chart,
                boundary.boundary.error_mm,
                *axis,
                *radius_mm,
                [input.vertices[a], input.vertices[b]],
                e.forward == forward,
            )
        }
    }
}
fn positive_spline(points: &[[I; 3]], spans: usize, work: &mut Work<'_>) -> Result<I, Error> {
    let mut sum = I::point(0.);
    for span in 0..spans {
        let mut pending = vec![(points[span * 9..=span * 9 + 9].to_vec(), 0)];
        while let Some((p, depth)) = pending.pop() {
            work.step()?;
            if let Some(angle) = positive_curve(&p, work)? {
                sum = sum + angle;
                continue;
            }
            if depth == 32 {
                return Err(Error::Unresolved);
            }
            let (a, b) = split(&p);
            pending.push((b, depth + 1));
            pending.push((a, depth + 1));
        }
    }
    Ok(sum)
}
fn magnitude(p: [I; 3]) -> Result<f64, Error> {
    Ok(p.into_iter()
        .fold(I::point(0.), |s, v| s + I::point(v.magnitude()))
        .checked()?
        .hi)
}
fn positive_curve(p: &[[I; 3]], work: &mut Work<'_>) -> Result<Option<I>, Error> {
    let h = [
        p[0][0].midpoint() + p[9][0].midpoint(),
        p[0][1].midpoint() + p[9][1].midpoint(),
    ];
    if p.iter()
        .any(|p| (p[0] * I::point(h[0]) + p[1] * I::point(h[1])).lo <= 0.)
    {
        return Ok(None);
    }
    let mut coefficients = vec![I::point(0.); 18];
    for i in 0..=9 {
        work.charge(9)?;
        for j in 0..9 {
            let weight = I::point(binomial(9, i) * binomial(8, j)) / I::point(binomial(17, i + j));
            coefficients[i + j] = coefficients[i + j]
                + weight * (p[i][0] * (p[j + 1][1] - p[j][1]) - p[i][1] * (p[j + 1][0] - p[j][0]));
        }
    }
    if coefficients.iter().any(|c| !c.valid() || c.lo <= 0.) {
        return Ok(None);
    }
    Ok(phase(p[0], p[9]))
}

fn circle_angle(
    chart: &axial::Chart,
    sewing_error: f64,
    axis: Axis,
    radius_mm: f64,
    endpoints: [Point; 2],
    positive: bool,
) -> Result<I, Error> {
    let normal = chart.axis_normal;
    if axis.normal != normal && axis.normal != normal.map(|v| -v) {
        return Err(Error::Unsupported);
    }
    let orientation =
        dot(axis.normal.map(I::point), chart.axes[2]) * I::point(if positive { 1. } else { -1. });
    orientation.positive()?;
    let center = chart.project(axis.origin);
    if magnitude([center[0], center[1], I::point(0.)])? > radius_mm / 16.
        || sewing_error > radius_mm / 16.
    {
        return Err(Error::Unresolved);
    }
    let [a, b] = endpoints;
    let aa = std::array::from_fn(|i| I::point(a[i]) - I::point(axis.origin[i]));
    let bb = std::array::from_fn(|i| I::point(b[i]) - I::point(axis.origin[i]));
    (dot(cross(aa, bb), axis.normal.map(I::point)) * I::point(if positive { 1. } else { -1. }))
        .positive()?;
    phase(chart.project(a), chart.project(b)).ok_or(Error::Unresolved)
}
