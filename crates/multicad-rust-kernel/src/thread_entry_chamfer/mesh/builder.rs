//! Clip regular source cells; retain shared analytic boundary footpoints.
use super::cell::{Bound, Corner, Kind};
use super::*;
use std::collections::HashMap;
pub(super) enum Failure {
    Geometry(Error),
    Refine(Vec<usize>),
}
impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Self::Geometry(e)
    }
}
#[derive(Clone, Copy)]
pub(super) enum Station {
    Entry,
    Seam,
    Top,
    Angle(f64),
}
#[derive(Clone, Copy)]
struct Vertex {
    index: usize,
    station: Station,
    w: f64,
}
#[derive(Clone, Copy)]
struct SourceVertex {
    vertex: Vertex,
    phase: I,
    shift: I,
}
#[derive(Clone, Copy)]
struct Rim {
    vertex: usize,
    phase: I,
    nominal: f64,
    period: u32,
}
struct Node {
    q: f64,
    g: f64,
    r: f64,
    stations: [Vertex; 3],
    bounds: [I; 3],
}
struct Builder<'a> {
    body: &'a Chamfer,
    plan: &'a Plan,
    options: Tessellation,
    cycle: I,
    cycle_nom: f64,
    mesh: DisplayMesh,
    nodes: Vec<Node>,
    rails: HashMap<(usize, bool, i64), Vertex>,
    cuts: HashMap<(usize, Kind, i64), SourceVertex>,
    phase_cuts: HashMap<(usize, Kind, u64), SourceVertex>,
    bottom: Vec<Rim>,
    top: Vec<Rim>,
}
impl Builder<'_> {
    fn prepare_nodes(&mut self, work: &mut Work<'_>) -> Result<(), Failure> {
        let body = self.body;
        let plan = self.plan;
        for period in 0..body.tool.spec().starts {
            for leaf in &plan.leaves {
                work.step()?;
                let sample = leaf.ends[0];
                let q = sample.nominal + f64::from(period) * body.tool.spec().pitch_mm;
                let r = if sample.threshold {
                    body.entry_radius_mm
                } else {
                    body.tool.radius_at_phase(sample.nominal)
                };
                let g = if sample.threshold {
                    0.
                } else {
                    body.seam_station(sample.nominal)
                };
                let rate = body.tool.helical_rate();
                let cap = self.vertex(polar(r.min(body.entry_radius_mm), 0., q, rate))?;
                let seam = if g == 0. {
                    cap
                } else {
                    self.vertex(polar(r, g, q, rate))?
                };
                let top = self.vertex(polar(r, body.material_length_mm(), q, rate))?;
                let stations = [
                    Vertex {
                        index: cap,
                        station: Station::Entry,
                        w: -q / rate.abs(),
                    },
                    Vertex {
                        index: seam,
                        station: Station::Seam,
                        w: (g - q) / rate.abs(),
                    },
                    Vertex {
                        index: top,
                        station: Station::Top,
                        w: (body.material_length_mm() - q) / rate.abs(),
                    },
                ];
                let line = body.patch_line(leaf.segment, sample.phase, period)?;
                let bounds = stations.map(|v| line.parameter(v.station, I::point(0.)));
                self.nodes.push(Node {
                    q,
                    g,
                    r,
                    stations,
                    bounds,
                });
                self.bottom.push(Rim {
                    vertex: cap,
                    phase: sample.phase,
                    nominal: sample.nominal,
                    period,
                });
                self.top.push(Rim {
                    vertex: top,
                    phase: sample.phase,
                    nominal: sample.nominal,
                    period,
                });
            }
        }
        Ok(())
    }
    fn vertex(&mut self, p: [f64; 3]) -> Result<usize, Failure> {
        if p.into_iter().any(|v| !v.is_finite()) {
            return Err(Error::Unresolved.into());
        }
        let id = self.mesh.vertices.len() / 3;
        if id >= 3 * self.options.triangle_limit {
            return Err(Error::Limit.into());
        }
        self.mesh.vertices.extend(p);
        Ok(id)
    }
    fn point(&self, id: usize) -> [f64; 3] {
        self.mesh.vertices[3 * id..3 * id + 3].try_into().unwrap()
    }
    fn triangle(&mut self, mut ids: [usize; 3], face: u32, reverse: bool) -> Result<bool, Failure> {
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[2] == ids[0] {
            return Ok(false);
        }
        if self.mesh.face_indices.len() == self.options.triangle_limit {
            return Err(Error::Limit.into());
        }
        if reverse {
            ids.swap(1, 2);
        }
        self.mesh.triangle(ids, face);
        Ok(true)
    }
    fn check_point(&self, id: usize, source: [I; 3]) -> Result<(), Failure> {
        let p = self.point(id);
        let error = source
            .into_iter()
            .zip(p)
            .fold(I::point(0.), |sum, (s, p)| {
                sum + I::point((s - I::point(p)).magnitude())
            })
            .checked()?;
        if error.hi > self.options.deflection * 0.125 {
            return Err(Error::Unresolved.into());
        }
        Ok(())
    }
    fn normal_check(&self, ids: [usize; 3], normal: [I; 3], allowance: f64) -> Result<(), Failure> {
        if allowance <= 0. {
            return Err(Failure::Refine(Vec::new()));
        }
        let p = ids.map(|id| self.point(id).map(I::point));
        let cross = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        let length = dot(cross, cross).sqrt().checked()?;
        if length.lo <= 0. {
            return Err(Failure::Refine(Vec::new()));
        }
        for vertex in p {
            if (cross[0] * vertex[0] + cross[1] * vertex[1]).lo <= 0. {
                return Err(Failure::Refine(Vec::new()));
            }
        }
        let cosine = dot(cross.map(|x| x / length), normal).checked()?;
        if cosine.lo < I::point(allowance).cos().hi {
            return Err(Failure::Refine(Vec::new()));
        }
        Ok(())
    }
    fn endpoint(&self, strip: usize, end: usize, vertex: Vertex) -> SourceVertex {
        let count = self.plan.leaves.len();
        let leaf = &self.plan.leaves[strip % count];
        let wrapped = strip + end == self.nodes.len();
        let shift = if wrapped { self.cycle } else { I::point(0.) };
        SourceVertex {
            vertex,
            phase: leaf.ends[end].phase,
            shift,
        }
    }
    fn rail(
        &mut self,
        strip: usize,
        end: usize,
        kind: Option<Kind>,
        row: Option<i64>,
        cone: bool,
    ) -> Result<SourceVertex, Failure> {
        let node_index = (strip + end) % self.nodes.len();
        if let Some(kind) = kind {
            return Ok(self.endpoint(
                strip,
                end,
                self.nodes[node_index].stations[kind_index(kind)],
            ));
        }
        let row = row.ok_or(Error::Unresolved)?;
        let k = self.plan.angular_count as i64;
        let local_row = if strip + end == self.nodes.len() {
            row + k
        } else {
            row
        };
        let key = (node_index, cone, local_row);
        if let Some(&v) = self.rails.get(&key) {
            return Ok(self.endpoint(strip, end, v));
        }
        let node = &self.nodes[node_index];
        let w = local_row as f64 * self.plan.angular_step;
        let bounds = if cone { [0, 1] } else { [1, 2] };
        let alias = bounds
            .into_iter()
            .find(|&i| node.bounds[i].lo <= w && w <= node.bounds[i].hi);
        let v = if let Some(i) = alias {
            node.stations[i]
        } else {
            let z = self.body.tool.helical_rate().abs().mul_add(w, node.q);
            let low = if cone { 0. } else { node.g };
            let high = if cone {
                node.g
            } else {
                self.body.material_length_mm()
            };
            if z <= low {
                node.stations[if cone { 0 } else { 1 }]
            } else if z >= high {
                node.stations[if cone { 1 } else { 2 }]
            } else {
                let r = if cone {
                    self.body.cone_radius(z)
                } else {
                    node.r
                };
                let (s, c) = (self.body.tool.helical_rate().signum() * w).sin_cos();
                let index = self.vertex([r * c, r * s, z])?;
                Vertex {
                    index,
                    station: Station::Angle(w),
                    w,
                }
            }
        };
        self.rails.insert(key, v);
        Ok(self.endpoint(strip, end, v))
    }
    fn corner(
        &mut self,
        strip: usize,
        corner: Corner,
        cone: bool,
    ) -> Result<SourceVertex, Failure> {
        let count = self.plan.leaves.len();
        let leaf = &self.plan.leaves[strip % count];
        let q =
            (leaf.ends[1].nominal - leaf.ends[0].nominal).mul_add(corner.u, leaf.ends[0].nominal);
        if corner.u == 0. || q <= leaf.ends[0].phase.hi {
            return self.rail(strip, 0, corner.boundary, corner.row, cone);
        }
        if corner.u == 1. || q >= leaf.ends[1].phase.lo {
            return self.rail(strip, 1, corner.boundary, corner.row, cone);
        }
        let kind = corner.boundary.ok_or(Error::Unresolved)?;
        let row = corner.row.ok_or(Error::Unresolved)?;
        let key = (strip, kind, row);
        if let Some(&v) = self.cuts.get(&key) {
            return Ok(v);
        }
        let phase_key = (strip, kind, q.to_bits());
        if let Some(&v) = self.phase_cuts.get(&phase_key) {
            self.cuts.insert(key, v);
            return Ok(v);
        }
        let period = (strip / count) as u32;
        let global_q = q + f64::from(period) * self.body.tool.spec().pitch_mm;
        let r = self.body.tool.radius_at_phase(q);
        let g = self.body.seam_station(q);
        let (z, r, station) = match kind {
            Kind::Entry => (0., r.min(self.body.entry_radius_mm), Station::Entry),
            Kind::Seam => (g, r, Station::Seam),
            Kind::Top => (self.body.material_length_mm(), r, Station::Top),
        };
        // Zero-height seam and entry footpoints are physically identical.
        // Keep their source stations distinct, but share the mesh vertex.
        let alias = if g == 0. {
            match kind {
                Kind::Entry => Some(Kind::Seam),
                Kind::Seam => Some(Kind::Entry),
                Kind::Top => None,
            }
        } else {
            None
        };
        if let Some(other) =
            alias.and_then(|other| self.phase_cuts.get(&(strip, other, q.to_bits())).copied())
        {
            let value = SourceVertex {
                vertex: Vertex {
                    station,
                    ..other.vertex
                },
                ..other
            };
            self.cuts.insert(key, value);
            self.phase_cuts.insert(phase_key, value);
            return Ok(value);
        }
        let index = self.vertex(polar(r, z, global_q, self.body.tool.helical_rate()))?;
        let vertex = Vertex {
            index,
            station,
            w: (z - global_q) / self.body.tool.helical_rate().abs(),
        };
        let v = SourceVertex {
            vertex,
            phase: I::point(q),
            shift: I::point(0.),
        };
        self.cuts.insert(key, v);
        self.phase_cuts.insert(phase_key, v);
        let rim = Rim {
            vertex: index,
            phase: I::point(q),
            nominal: q,
            period,
        };
        match kind {
            Kind::Entry => self.bottom.push(rim),
            Kind::Top => self.top.push(rim),
            Kind::Seam => {
                if g == 0. {
                    self.bottom.push(rim);
                }
            }
        }
        Ok(v)
    }
    fn side(
        &mut self,
        strip: usize,
        cone: bool,
        face: u32,
        work: &mut Work<'_>,
    ) -> Result<(), Failure> {
        let count = self.plan.leaves.len();
        let leaf = self.plan.leaves[strip % count];
        let a = &self.nodes[strip];
        let b = &self.nodes[(strip + 1) % self.nodes.len()];
        let wrapped = strip + 1 == self.nodes.len();
        let offset = if wrapped { self.cycle_nom } else { 0. };
        let ends = if cone {
            [Kind::Entry, Kind::Seam]
        } else {
            [Kind::Seam, Kind::Top]
        };
        let [lower, upper] = ends.map(|kind| Bound {
            kind,
            ends: [
                a.stations[kind_index(kind)].w,
                b.stations[kind_index(kind)].w - offset,
            ],
        });
        let low = lower.ends[0].min(lower.ends[1]);
        let high = upper.ends[0].max(upper.ends[1]);
        let step = self.plan.angular_step;
        let lo = (low / step).floor();
        let hi = (high / step).ceil();
        if lo.abs().max(hi.abs()) > 4_503_599_627_370_496. {
            return Err(Error::Unresolved.into());
        }
        if hi - lo > self.options.triangle_limit as f64 {
            return Err(Error::Limit.into());
        }
        let bounds = if cone {
            leaf.cone.unwrap()
        } else {
            leaf.thread
        };
        let trim = leaf.trim;
        if trim[0] > self.options.deflection * 0.125 || trim[1] > self.options.angle * 0.1 {
            return Err(Failure::Refine(Vec::new()));
        }
        let glue = self.body.glue_residual(leaf.segment)?;
        let period = (strip / count) as u32;
        let lines = leaf
            .ends
            .map(|end| self.body.patch_line(leaf.segment, end.phase, period))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let field = leaf.normals[usize::from(cone)];
        let mut prepared: HashMap<(usize, u8, u64), I> = HashMap::new();
        let mut emitted = 0;
        for row in lo as i64 - 1..=hi as i64 + 1 {
            work.step()?;
            let polygon = cell::polygon(row, step, lower, upper)?;
            let mut vertices = Vec::with_capacity(6);
            for corner in polygon {
                let v = self.corner(strip, corner, cone)?;
                if vertices
                    .last()
                    .is_none_or(|last: &SourceVertex| last.vertex.index != v.vertex.index)
                {
                    vertices.push(v);
                }
            }
            if vertices.len() > 1
                && vertices.first().unwrap().vertex.index == vertices.last().unwrap().vertex.index
            {
                vertices.pop();
            }
            if vertices.len() < 3 {
                continue;
            }
            let mut accepted = None;
            for pivot in 0..vertices.len() {
                let candidate = (|| -> Result<Vec<[usize; 3]>, Failure> {
                    let mut triangles = Vec::new();
                    for k in 2..vertices.len() {
                        work.step()?;
                        let source = [
                            vertices[pivot],
                            vertices[(pivot + k - 1) % vertices.len()],
                            vertices[(pivot + k) % vertices.len()],
                        ];
                        let mut params = [I::point(0.); 3];
                        for (v, w) in source.into_iter().zip(&mut params) {
                            let (kind, bits) = match v.vertex.station {
                                Station::Entry => (0, 0),
                                Station::Seam => (1, 0),
                                Station::Top => (2, 0),
                                Station::Angle(value) => (3, value.to_bits()),
                            };
                            let key = (v.vertex.index, kind, bits);
                            if let Some(&old) = prepared.get(&key) {
                                *w = old;
                                continue;
                            }
                            let line = if v.phase == leaf.ends[0].phase {
                                lines[0]
                            } else if v.phase == leaf.ends[1].phase {
                                lines[1]
                            } else {
                                self.body.patch_line(leaf.segment, v.phase, period)?
                            };
                            self.check_point(
                                v.vertex.index,
                                line.point(v.vertex.station, v.shift, cone)?,
                            )?;
                            *w = line.parameter(v.vertex.station, v.shift);
                            prepared.insert(key, *w);
                        }
                        let range = params.into_iter().reduce(I::hull).unwrap();
                        let dw = I::point(range.width());
                        let chord = (I::point(bounds.hessian[0]) * leaf.dq.square()
                            + I::point(2.) * I::point(bounds.hessian[1]) * leaf.dq * dw
                            + I::point(bounds.hessian[2]) * dw.square())
                            / I::point(8.);
                        if chord.hi > self.options.deflection * 0.4 {
                            return Err(Failure::Refine(Vec::new()));
                        }
                        let center = range.lo.midpoint(range.hi);
                        let variation = I::point(bounds.angular_q) * leaf.dq / I::point(2.)
                            + I::point((I::point(center) - range).magnitude());
                        let angle = (I::point(self.options.angle)
                            - variation
                            - I::point(trim[1])
                            - I::point(glue[1]))
                        .lo;
                        let normal = field.at(I::point(center))?;
                        let mut ids = source.map(|v| v.vertex.index);
                        let reverse = !self.body.tool.spec().left_hand;
                        if reverse {
                            ids.swap(1, 2);
                        }
                        self.normal_check(ids, normal, angle)?;
                        if reverse {
                            ids.swap(1, 2);
                        }
                        triangles.push(ids);
                    }
                    Ok(triangles)
                })();
                match candidate {
                    Ok(triangles) => {
                        accepted = Some(triangles);
                        break;
                    }
                    Err(Failure::Refine(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            let triangles = accepted.ok_or_else(|| Failure::Refine(Vec::new()))?;
            for triangle in triangles {
                if self.triangle(triangle, face, !self.body.tool.spec().left_hand)? {
                    emitted += 1;
                }
            }
        }
        if emitted == 0 {
            return Err(Error::Unresolved.into());
        }
        Ok(())
    }
    fn finish(mut self, work: &mut Work<'_>) -> Result<DisplayMesh, Failure> {
        let lower = ordered(std::mem::take(&mut self.bottom))?;
        let upper = ordered(std::mem::take(&mut self.top))?;
        let left = self.body.tool.spec().left_hand;
        let bottom = self.vertex([0., 0., 0.])?;
        for pair in lower.iter().zip(lower.iter().cycle().skip(1)) {
            work.step()?;
            self.triangle([bottom, pair.0.vertex, pair.1.vertex], 0, left)?;
        }
        if let Some(tail) = self.body.terminal {
            let f = self.body.material_length_mm();
            let h = self.body.full_length_mm();
            let r = tail.neck_radius_mm();
            let mut inner = Vec::with_capacity(upper.len());
            for rim in &upper {
                work.step()?;
                let q = rim.nominal + f64::from(rim.period) * self.body.tool.spec().pitch_mm;
                let theta = (f - q) / self.body.tool.helical_rate();
                let (s, c) = theta.sin_cos();
                let points = [[r * c, r * s, f], [r * c, r * s, h]];
                let ids = [self.vertex(points[0])?, self.vertex(points[1])?];
                let t = (I::point(f)
                    - rim.phase
                    - I::point(f64::from(rim.period)) * I::point(self.body.tool.spec().pitch_mm))
                    / self.body.rate_interval()?;
                for (id, z) in ids.into_iter().zip([f, h]) {
                    self.check_point(
                        id,
                        [I::point(r) * t.cos(), I::point(r) * t.sin(), I::point(z)],
                    )?;
                }
                inner.push(ids);
            }
            let top = self.vertex([0., 0., h])?;
            for i in 0..upper.len() {
                work.step()?;
                let next = (i + 1) % upper.len();
                let a = upper[i];
                let b = upper[next];
                let [low, high] = inner[i];
                let [low_next, high_next] = inner[next];
                self.triangle([a.vertex, b.vertex, low_next], 1, !left)?;
                self.triangle([a.vertex, low_next, low], 1, !left)?;
                let qa = a.phase
                    + I::point(f64::from(a.period)) * I::point(self.body.tool.spec().pitch_mm);
                let qb = if next == 0 {
                    b.phase
                        + I::point(f64::from(self.body.tool.spec().starts))
                            * I::point(self.body.tool.spec().pitch_mm)
                } else {
                    b.phase
                        + I::point(f64::from(b.period)) * I::point(self.body.tool.spec().pitch_mm)
                };
                let angle = (I::point(f) - (qa + qb) / I::point(2.)) / self.body.rate_interval()?;
                let normal = [angle.cos(), angle.sin(), I::point(0.)];
                let allowance = (I::point(self.options.angle)
                    - (qb - qa).abs() / self.body.rate_interval()?.abs() / I::point(2.))
                .lo;
                for mut tri in [[low, low_next, high_next], [low, high_next, high]] {
                    self.triangle(tri, 2 + 6 * self.body.tool.spec().starts, !left)?;
                    if !left {
                        tri.swap(1, 2);
                    }
                    self.normal_check(tri, normal, allowance)?;
                }
                self.triangle(
                    [top, high, high_next],
                    3 + 6 * self.body.tool.spec().starts,
                    !left,
                )?;
            }
        } else {
            let top = self.vertex([0., 0., self.body.full_length_mm()])?;
            for (a, b) in upper.iter().zip(upper.iter().cycle().skip(1)) {
                work.step()?;
                self.triangle([top, a.vertex, b.vertex], 1, !left)?;
            }
        }
        crate::manufacturing::certify(&self.mesh)
            .map_err(|_| Failure::Geometry(Error::Unresolved))?;
        Ok(self.mesh)
    }
}
fn kind_index(kind: Kind) -> usize {
    match kind {
        Kind::Entry => 0,
        Kind::Seam => 1,
        Kind::Top => 2,
    }
}
fn ordered(mut ring: Vec<Rim>) -> Result<Vec<Rim>, Failure> {
    ring.sort_unstable_by(|a, b| {
        a.period
            .cmp(&b.period)
            .then(a.nominal.total_cmp(&b.nominal))
    });
    ring.dedup_by_key(|v| v.vertex);
    if ring.len() < 3
        || !ring
            .windows(2)
            .all(|p| (p[0].period, p[0].nominal) < (p[1].period, p[1].nominal))
    {
        return Err(Error::Unresolved.into());
    }
    Ok(ring)
}
fn sub(a: [I; 3], b: [I; 3]) -> [I; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: [I; 3], b: [I; 3]) -> [I; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [I; 3], b: [I; 3]) -> I {
    a.into_iter()
        .zip(b)
        .fold(I::point(0.), |s, (a, b)| s + a * b)
}
fn polar(radius: f64, z: f64, q: f64, rate: f64) -> [f64; 3] {
    let (s, c) = ((z - q) / rate).sin_cos();
    [radius * c, radius * s, z]
}
pub(super) fn build(
    body: &Chamfer,
    plan: &Plan,
    options: Tessellation,
    work: &mut Work<'_>,
) -> Result<DisplayMesh, Failure> {
    let mut b = Builder {
        body,
        plan,
        options,
        cycle: (I::point(body.tool.spec().pitch_mm) * I::point(f64::from(body.tool.spec().starts))
            / body.rate_interval()?.abs())
        .checked()?,
        cycle_nom: body.tool.spec().pitch_mm * f64::from(body.tool.spec().starts)
            / body.tool.helical_rate().abs(),
        mesh: DisplayMesh {
            vertices: Vec::new(),
            indices: Vec::new(),
            face_indices: Vec::new(),
        },
        nodes: Vec::new(),
        rails: HashMap::new(),
        cuts: HashMap::new(),
        phase_cuts: HashMap::new(),
        bottom: Vec::new(),
        top: Vec::new(),
    };
    b.prepare_nodes(work)?;
    let mut failed = vec![false; plan.leaves.len()];
    for strip in 0..b.nodes.len() {
        let leaf = plan.leaves[strip % plan.leaves.len()];
        let period = (strip / plan.leaves.len()) as u32;
        let result = (|| -> Result<(), Failure> {
            b.side(
                strip,
                false,
                body.tool.face_index(i64::from(period), leaf.segment.band),
                work,
            )?;
            if leaf.segment.active {
                let crest = i64::from(period) + leaf.segment.crest_shift;
                let start = if body.tool.spec().left_hand {
                    crest
                } else {
                    -crest
                }
                .rem_euclid(i64::from(body.tool.spec().starts)) as u32;
                let face = 2
                    + 4 * body.tool.spec().starts
                    + 2 * start
                    + u32::from(leaf.segment.half == ConeHalf::Falling);
                b.side(strip, true, face, work)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(Failure::Refine(_)) => failed[strip % plan.leaves.len()] = true,
            Err(e) => return Err(e),
        }
    }
    if failed.iter().any(|&v| v) {
        return Err(Failure::Refine(
            failed
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| v.then_some(i))
                .collect(),
        ));
    }
    b.finish(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn require<T>(value: Result<T, Failure>) -> T {
        value.ok().expect("expected a proved geometric primitive")
    }
    fn fresh<'a>(body: &'a Chamfer, plan: &'a Plan, options: Tessellation) -> Builder<'a> {
        Builder {
            body,
            plan,
            options,
            cycle: (I::point(body.tool.spec().pitch_mm)
                * I::point(f64::from(body.tool.spec().starts))
                / body.rate_interval().unwrap().abs()),
            cycle_nom: body.tool.spec().pitch_mm * f64::from(body.tool.spec().starts)
                / body.tool.helical_rate().abs(),
            mesh: DisplayMesh {
                vertices: Vec::new(),
                indices: Vec::new(),
                face_indices: Vec::new(),
            },
            nodes: Vec::new(),
            rails: HashMap::new(),
            cuts: HashMap::new(),
            phase_cuts: HashMap::new(),
            bottom: Vec::new(),
            top: Vec::new(),
        }
    }
    #[test]
    fn facet_certificates_refuse_reversed_degenerate_and_misplaced_actual_crest_samples() {
        let body = Chamfer::new(
            crate::thread_entry_chamfer::tests::tool(false, 1, false),
            0.3,
            None,
        )
        .unwrap();
        let plan = Plan {
            leaves: Vec::new(),
            angular_step: 0.1,
            angular_count: 63,
        };
        let options = Tessellation {
            deflection: 0.01,
            angle: 0.3,
            triangle_limit: 2,
        };
        let mut b = Builder {
            body: &body,
            plan: &plan,
            options,
            cycle: I::point(std::f64::consts::TAU),
            cycle_nom: std::f64::consts::TAU,
            mesh: DisplayMesh {
                vertices: Vec::new(),
                indices: Vec::new(),
                face_indices: Vec::new(),
            },
            nodes: Vec::new(),
            rails: HashMap::new(),
            cuts: HashMap::new(),
            phase_cuts: HashMap::new(),
            bottom: Vec::new(),
            top: Vec::new(),
        };
        let theta = 0.1_f64;
        let p = [
            [3., 0., 1.],
            [3. * theta.cos(), 3. * theta.sin(), 1.],
            [3., 0., 2.],
        ];
        for v in p {
            assert_eq!(body.radius(v[1].atan2(v[0]), v[2]), 3.);
        }
        let ids = p.map(|p| require(b.vertex(p)));
        let normal = [
            I::point((theta / 2.).cos()),
            I::point((theta / 2.).sin()),
            I::point(0.),
        ];
        assert!(b.normal_check(ids, normal, 0.1).is_ok());
        assert!(matches!(
            b.normal_check([ids[0], ids[2], ids[1]], normal, 0.1),
            Err(Failure::Refine(_))
        ));
        assert!(matches!(
            b.normal_check(ids, normal.map(|v| -v), 0.1),
            Err(Failure::Refine(_))
        ));
        assert!(matches!(
            b.normal_check(ids, normal, 0.),
            Err(Failure::Refine(_))
        ));
        let coincident = require(b.vertex(p[0]));
        assert!(matches!(
            b.normal_check([ids[0], coincident, ids[1]], normal, 0.1),
            Err(Failure::Refine(_))
        ));
        assert!(b.check_point(ids[0], p[0].map(I::point)).is_ok());
        assert!(matches!(
            b.check_point(ids[0], [I::point(4.), I::point(0.), I::point(1.)]),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        assert!(matches!(
            b.vertex([f64::NAN, 0., 0.]),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        assert!(!require(b.triangle([0, 0, 1], 0, false)));
        assert!(require(b.triangle(ids, 0, false)));
        assert!(require(b.triangle(ids, 0, true)));
        assert!(matches!(
            b.triangle(ids, 0, false),
            Err(Failure::Geometry(Error::Limit))
        ));
        require(b.vertex([3., 0., 3.]));
        require(b.vertex([3., 0., 4.]));
        assert!(matches!(
            b.vertex([3., 0., 5.]),
            Err(Failure::Geometry(Error::Limit))
        ));
    }
    #[test]
    fn empty_or_conflicting_physical_rims_are_never_published_as_closed_caps() {
        let a = Rim {
            vertex: 0,
            phase: I::point(0.),
            nominal: 0.,
            period: 0,
        };
        let b = Rim {
            vertex: 1,
            phase: I::point(0.01),
            nominal: 0.01,
            period: 0,
        };
        let c = Rim {
            vertex: 2,
            phase: I::point(0.02),
            nominal: 0.02,
            period: 0,
        };
        assert!(ordered(vec![c, a, b, a]).is_ok());
        assert!(matches!(
            ordered(vec![a, b]),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        assert!(matches!(
            ordered(vec![a, Rim { vertex: 3, ..a }, b, c]),
            Err(Failure::Geometry(Error::Unresolved))
        ));
    }
    #[test]
    fn source_station_keys_budget_guards_and_final_closed_shell_proof_are_independent() {
        let source = crate::thread_entry_chamfer::tests::tool(false, 1, false);
        let body = Chamfer::new(source, source.spec().depth_mm, None).unwrap();
        let options = Tessellation {
            deflection: 0.04,
            angle: 0.4,
            triangle_limit: 500_000,
        };
        let mut work = Work {
            remaining: 16_000_000,
            interrupt: None,
        };
        let plan = make_plan(
            &body,
            &mesh_segments(&body, options).unwrap(),
            options,
            1.,
            &mut work,
        )
        .unwrap();
        let mut b = fresh(&body, &plan, options);
        require(b.prepare_nodes(&mut work));
        assert!(matches!(
            b.rail(0, 0, None, None, false),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        let lower = require(b.rail(0, 0, None, Some(-10000), true));
        let upper = require(b.rail(0, 0, None, Some(10000), false));
        assert!(matches!(lower.vertex.station, Station::Entry));
        assert!(matches!(upper.vertex.station, Station::Top));
        assert!(matches!(
            b.corner(
                0,
                Corner {
                    u: 0.5,
                    w: 0.,
                    row: None,
                    boundary: Some(Kind::Top)
                },
                false
            ),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        assert!(matches!(
            b.corner(
                0,
                Corner {
                    u: 0.5,
                    w: 0.,
                    row: Some(0),
                    boundary: None
                },
                false
            ),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        let a = require(b.corner(
            0,
            Corner {
                u: 0.5,
                w: 0.,
                row: Some(1),
                boundary: Some(Kind::Top),
            },
            false,
        ));
        let c = require(b.corner(
            0,
            Corner {
                u: 0.5,
                w: 0.,
                row: Some(2),
                boundary: Some(Kind::Top),
            },
            false,
        ));
        assert_eq!(a.vertex.index, c.vertex.index);
        b.options.triangle_limit = 1;
        assert!(matches!(
            b.side(0, false, 2, &mut work),
            Err(Failure::Geometry(Error::Limit))
        ));
        let mut invalid_grid = Plan {
            leaves: plan.leaves.clone(),
            angular_step: 1e-300,
            angular_count: plan.angular_count,
        };
        let mut b = fresh(&body, &invalid_grid, options);
        require(b.prepare_nodes(&mut work));
        assert!(matches!(
            b.side(0, false, 2, &mut work),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        let root = plan
            .leaves
            .iter()
            .position(|leaf| leaf.segment.band == 2 && !leaf.segment.active)
            .unwrap();
        invalid_grid.angular_step = plan.angular_step;
        let mut b = fresh(&body, &plan, options);
        require(b.prepare_nodes(&mut work));
        let mut absent = Plan {
            leaves: plan.leaves.clone(),
            angular_step: plan.angular_step,
            angular_count: plan.angular_count,
        };
        let leaf = &mut absent.leaves[root];
        leaf.cone = Some(
            body.patch_bounds(
                leaf.segment,
                I::new(leaf.ends[0].phase.lo, leaf.ends[1].phase.hi),
                true,
            )
            .unwrap(),
        );
        let mut b = fresh(&body, &absent, options);
        require(b.prepare_nodes(&mut work));
        assert!(matches!(
            b.side(root, true, 2, &mut work),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        let mut b = fresh(&body, &plan, options);
        require(b.prepare_nodes(&mut work));
        assert!(matches!(
            b.finish(&mut work),
            Err(Failure::Geometry(Error::Unresolved))
        ));
    }
    #[test]
    fn stale_source_points_and_stricter_terminal_requests_cannot_reuse_old_proofs() {
        let source = crate::thread_entry_chamfer::tests::tool(false, 1, true);
        let body = Chamfer::new(
            source,
            0.3,
            Some(TerminalIntent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            }),
        )
        .unwrap();
        let options = Tessellation {
            deflection: 0.04,
            angle: 0.4,
            triangle_limit: 500_000,
        };
        let mut work = Work {
            remaining: 16_000_000,
            interrupt: None,
        };
        let plan = make_plan(
            &body,
            &mesh_segments(&body, options).unwrap(),
            options,
            1.,
            &mut work,
        )
        .unwrap();
        let mut stale = fresh(&body, &plan, options);
        require(stale.prepare_nodes(&mut work));
        let vertex = stale.nodes[0].stations[1].index;
        stale.mesh.vertices[3 * vertex] += 0.1;
        assert!(matches!(
            stale.side(0, false, 2, &mut work),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        let mut precise = fresh(&body, &plan, options);
        require(precise.prepare_nodes(&mut work));
        precise.options.deflection = 1e-30;
        assert!(matches!(
            precise.finish(&mut work),
            Err(Failure::Geometry(Error::Unresolved))
        ));
        let mut budget = fresh(&body, &plan, options);
        require(budget.prepare_nodes(&mut work));
        let limit = budget.bottom.len() + 5 * (budget.top.len() / 2) + 4;
        budget.options.triangle_limit = limit;
        assert!(matches!(
            budget.finish(&mut work),
            Err(Failure::Geometry(Error::Limit))
        ));
        assert_eq!(body.source_thread(), &source);
    }
}
