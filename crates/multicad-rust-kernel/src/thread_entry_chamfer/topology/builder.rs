//! Semantic source strips, before any adaptive approximation or display mesh.
use super::*;
type Key = (u32, u64);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum VertexKey {
    Join(Key),
    Base(Key),
    Top(Key),
    Neck(bool),
}
struct Builder<'a, 'b> {
    source: Chamfer,
    tolerance: f64,
    binding_error: f64,
    work: &'a mut Work<'b>,
    vertices: Vec<[I; 3]>,
    vertex_keys: BTreeMap<VertexKey, usize>,
    edges: Vec<BoundaryEdge>,
    rails: BTreeMap<(bool, Key), (usize, i32)>,
    faces: Vec<Vec<EdgeUse>>,
}
pub(super) fn endpoint(
    body: &Chamfer,
    segment: phase::Segment,
    knot: phase::Knot,
) -> PhaseEndpoint {
    let breaks = body.tool.profile_breaks();
    let constraint = if let Some(index) = breaks.iter().position(|&q| q == knot.nominal) {
        PhaseConstraint::ProfileKnot(index)
    } else if knot.threshold {
        PhaseConstraint::EntryIntersection {
            band: segment.band,
            half: segment.half,
        }
    } else {
        PhaseConstraint::Valley
    };
    PhaseEndpoint {
        interval: enclosure(knot.phase),
        nominal: knot.nominal,
        constraint,
        contact: knot.threshold,
        exact_contact: knot.threshold && !knot.glued,
    }
}
fn reverse(mut edges: Vec<EdgeUse>) -> Vec<EdgeUse> {
    edges.reverse();
    for e in &mut edges {
        e.forward = !e.forward;
    }
    edges
}
impl Builder<'_, '_> {
    fn key(&self, start: u32, phase: PhaseEndpoint) -> (Key, i32) {
        let n = self.source.tool.spec().starts;
        if phase.nominal == self.source.tool.spec().pitch_mm {
            (
                ((start + 1) % n, 0_f64.to_bits()),
                i32::from(start + 1 == n),
            )
        } else {
            ((start, phase.nominal.to_bits()), 0)
        }
    }
    fn vertex(&mut self, key: VertexKey, point: [I; 3]) -> Result<usize, Error> {
        self.work.step()?;
        for p in point {
            p.checked()?;
        }
        if let Some(&id) = self.vertex_keys.get(&key) {
            for (i, p) in point.into_iter().enumerate() {
                self.vertices[id][i] = self.vertices[id][i].hull(p);
            }
            return Ok(id);
        }
        let id = self.vertices.len();
        self.vertices.push(point);
        self.vertex_keys.insert(key, id);
        Ok(id)
    }
    fn edge(
        &mut self,
        source: SourceCurve,
        keys: [VertexKey; 2],
        seam: bool,
    ) -> Result<usize, Error> {
        self.work.step()?;
        let a = curve::bounds(&self.source, source, [0.; 2])?;
        let b = curve::bounds(&self.source, source, [1.; 2])?;
        let vertices = [self.vertex(keys[0], a)?, self.vertex(keys[1], b)?];
        let id = self.edges.len();
        self.edges.push(BoundaryEdge {
            vertices,
            source,
            periodic_seam: seam,
        });
        Ok(id)
    }
    fn rail(
        &mut self,
        start: u32,
        band: usize,
        end: PhaseEndpoint,
        active: bool,
        cone: bool,
    ) -> Result<Option<EdgeUse>, Error> {
        if cone && end.contact {
            return Ok(None);
        }
        let (key, lift) = self.key(start, end);
        let requested = if cone {
            SourceCurve::ConeRail {
                start,
                band,
                phase: end,
            }
        } else {
            SourceCurve::WallRail {
                start,
                band,
                phase: end,
                active,
            }
        };
        if let Some(&(id, base)) = self.rails.get(&(cone, key)) {
            let turn = (lift - base)
                * if self.source.tool.spec().left_hand {
                    1
                } else {
                    -1
                };
            let error =
                curve::rail_binding_error(&self.source, self.edges[id].source, requested, turn)?;
            self.binding_error = self.binding_error.max(error);
            return Ok(Some(EdgeUse {
                edge: id,
                forward: true,
                angular_turns: turn,
            }));
        }
        let source = if cone {
            SourceCurve::ConeRail {
                start,
                band,
                phase: end,
            }
        } else {
            SourceCurve::WallRail {
                start,
                band,
                phase: end,
                active,
            }
        };
        let keys = if cone {
            [VertexKey::Base(key), VertexKey::Join(key)]
        } else {
            [VertexKey::Join(key), VertexKey::Top(key)]
        };
        let id = self.edge(source, keys, cone && key == (0, 0_f64.to_bits()))?;
        self.rails.insert((cone, key), (id, lift));
        Ok(Some(EdgeUse {
            edge: id,
            forward: true,
            angular_turns: 0,
        }))
    }
    fn strip(&mut self, start: u32, segment: phase::Segment) -> Result<(), Error> {
        self.work.step()?;
        let ends = segment.ends.map(|k| endpoint(&self.source, segment, k));
        let keys = ends.map(|p| self.key(start, p).0);
        let join_keys = keys.map(VertexKey::Join);
        let top_keys = keys.map(VertexKey::Top);
        let seam = self.edge(
            SourceCurve::Intersection {
                start,
                band: segment.band,
                phase: ends,
                active: segment.active,
            },
            join_keys,
            false,
        )?;
        let top = self.edge(
            SourceCurve::EndProfile {
                start,
                band: segment.band,
                phase: ends,
            },
            top_keys,
            false,
        )?;
        let left = self
            .rail(start, segment.band, ends[0], segment.active, false)?
            .ok_or(Error::Unresolved)?;
        let right = self
            .rail(start, segment.band, ends[1], segment.active, false)?
            .ok_or(Error::Unresolved)?;
        let use_edge = |edge, forward| EdgeUse {
            edge,
            forward,
            angular_turns: 0,
        };
        let mut wall = vec![
            use_edge(seam, true),
            right,
            use_edge(top, false),
            EdgeUse {
                forward: false,
                ..left
            },
        ];
        if segment.band == 0 && start == 0 {
            let turn = if self.source.tool.spec().left_hand {
                1
            } else {
                -1
            };
            for u in &mut wall {
                u.angular_turns += turn;
            }
        }
        if !self.source.tool.spec().left_hand {
            wall = reverse(wall);
        }
        let face = self
            .source
            .wall_face_index(start, segment.band)
            .ok_or(Error::Unresolved)?;
        self.faces[face as usize].extend(wall);
        self.faces[1].push(use_edge(top, self.source.tool.spec().left_hand));
        if segment.active {
            // A true threshold joins the plane, wall and cone at the same
            // source constraint; no coordinate-nearness merge is performed.
            let base_keys = std::array::from_fn(|i| {
                if ends[i].contact {
                    join_keys[i]
                } else {
                    VertexKey::Base(keys[i])
                }
            });
            let bottom = self.edge(
                SourceCurve::EntryArc { start, phase: ends },
                base_keys,
                false,
            )?;
            let mut cone = vec![use_edge(bottom, true)];
            if let Some(r) = self.rail(start, segment.band, ends[1], true, true)? {
                cone.push(r);
            }
            cone.push(use_edge(seam, false));
            if let Some(l) = self.rail(start, segment.band, ends[0], true, true)? {
                cone.push(EdgeUse {
                    forward: false,
                    ..l
                });
            }
            if !self.source.tool.spec().left_hand {
                cone = reverse(cone);
            }
            let face = self
                .source
                .cone_face_index(start, segment.half)
                .ok_or(Error::Unresolved)?;
            self.faces[face as usize].extend(cone);
            self.faces[0].push(use_edge(bottom, !self.source.tool.spec().left_hand));
        } else {
            self.faces[0].push(use_edge(seam, !self.source.tool.spec().left_hand));
        }
        Ok(())
    }
}
/// Cancel only opposite strip-interior uses. Periodic seam uses remain explicit.
fn exterior(
    edges: &[BoundaryEdge],
    uses: Vec<EdgeUse>,
    work: &mut Work<'_>,
) -> Result<Vec<EdgeUse>, Error> {
    let mut map: BTreeMap<usize, Vec<EdgeUse>> = BTreeMap::new();
    for u in uses {
        work.step()?;
        map.entry(u.edge).or_default().push(u);
    }
    let mut output = Vec::new();
    for (edge, mut uses) in map {
        if uses.len() == 2 && !edges[edge].periodic_seam {
            if uses[0].forward == uses[1].forward {
                return Err(Error::Unresolved);
            }
        } else {
            output.append(&mut uses);
        }
    }
    Ok(output)
}
fn loops(
    edges: &[BoundaryEdge],
    uses: Vec<EdgeUse>,
    work: &mut Work<'_>,
) -> Result<Vec<Vec<EdgeUse>>, Error> {
    let mut remaining: BTreeMap<usize, EdgeUse> = BTreeMap::new();
    let vertices = |u: EdgeUse| {
        let mut v = edges[u.edge].vertices;
        if !u.forward {
            v.reverse();
        }
        v
    };
    for u in uses {
        work.step()?;
        let start = vertices(u)[0];
        if remaining.insert(start, u).is_some() {
            return Err(Error::Unresolved);
        }
    }
    let mut result = Vec::new();
    while let Some((&start, &first)) = remaining.first_key_value() {
        let mut wire = vec![first];
        remaining.remove(&start);
        let mut end = vertices(first)[1];
        while end != start {
            work.step()?;
            let edge = remaining.remove(&end).ok_or(Error::Unresolved)?;
            end = vertices(edge)[1];
            wire.push(edge);
        }
        result.push(wire);
    }
    Ok(result)
}
pub(super) fn candidate(
    source: Chamfer,
    tolerance: f64,
    work: &mut Work<'_>,
) -> Result<Candidate, Error> {
    work.step()?;
    let mut b = Builder {
        source,
        tolerance,
        binding_error: 0.,
        work,
        vertices: Vec::new(),
        vertex_keys: BTreeMap::new(),
        edges: Vec::new(),
        rails: BTreeMap::new(),
        faces: vec![Vec::new(); source.face_count() as usize],
    };
    for start in 0..source.tool.spec().starts {
        for segment in source.phase_segments()? {
            b.strip(start, segment)?;
        }
    }
    let mut faces = Vec::new();
    for (index, uses) in std::mem::take(&mut b.faces).into_iter().enumerate() {
        let uses = exterior(&b.edges, uses, b.work)?;
        faces.push(BoundaryFace {
            role: source.face(index as u32).ok_or(Error::Unresolved)?,
            loops: loops(&b.edges, uses, b.work)?,
        });
    }
    if let Some(neck) = source.terminal {
        let radius = neck.neck_radius_mm();
        let stations = [source.material_length_mm(), source.full_length_mm()];
        let low = b.edge(
            SourceCurve::Circle {
                radius_mm: radius,
                z_mm: stations[0],
            },
            [VertexKey::Neck(false); 2],
            false,
        )?;
        let high = b.edge(
            SourceCurve::Circle {
                radius_mm: radius,
                z_mm: stations[1],
            },
            [VertexKey::Neck(true); 2],
            false,
        )?;
        let seam = b.edge(
            SourceCurve::NeckSeam {
                radius_mm: radius,
                stations_mm: stations,
            },
            [VertexKey::Neck(false), VertexKey::Neck(true)],
            true,
        )?;
        let use_edge = |edge, forward, angular_turns| EdgeUse {
            edge,
            forward,
            angular_turns,
        };
        faces[1].loops.push(vec![use_edge(low, false, 0)]);
        let count = source.tool.spec().starts as usize;
        faces[2 + 6 * count].loops = vec![vec![
            use_edge(low, true, 0),
            use_edge(seam, true, 1),
            use_edge(high, false, 0),
            use_edge(seam, false, 0),
        ]];
        faces[3 + 6 * count].loops = vec![vec![use_edge(high, true, 0)]];
    }
    // Strip-interior edges were never boundary authority. Remove them and
    // their now-unused vertices before assigning the published dense IDs.
    let mut used = BTreeSet::new();
    for face in &faces {
        for wire in &face.loops {
            for u in wire {
                used.insert(u.edge);
            }
        }
    }
    let mut edges = Vec::new();
    let mut ids = BTreeMap::new();
    for old in used {
        b.work.step()?;
        ids.insert(old, edges.len());
        edges.push(b.edges[old].clone());
    }
    for face in &mut faces {
        for wire in &mut face.loops {
            for u in wire {
                u.edge = ids[&u.edge];
            }
        }
    }
    let mut used = BTreeSet::new();
    for edge in &edges {
        used.extend(edge.vertices);
    }
    let mut vertices = Vec::new();
    let mut ids = BTreeMap::new();
    let mut maximum = 0_f64;
    for old in used {
        b.work.step()?;
        let value = sample(b.vertices[old])?;
        if value.maximum_error_mm > b.tolerance {
            return Err(Error::Unresolved);
        }
        maximum = maximum.max(value.maximum_error_mm);
        ids.insert(old, vertices.len());
        vertices.push(value);
    }
    for edge in &mut edges {
        edge.vertices = edge.vertices.map(|id| ids[&id]);
    }
    let result = Candidate {
        source,
        vertices,
        edges,
        faces,
        maximum_vertex_error_mm: maximum,
        maximum_curve_binding_error_mm: b.binding_error,
    };
    if (I::point(maximum) + I::point(b.binding_error)).hi > tolerance {
        return Err(Error::Unresolved);
    }
    Ok(result)
}

pub(super) fn build(
    source: Chamfer,
    tolerance: f64,
    work: &mut Work<'_>,
) -> Result<BoundaryTopology, Error> {
    let raw = candidate(source, tolerance, work)?;
    validate::check(&raw, work)?;
    Ok(BoundaryTopology {
        source: raw.source,
        vertices: raw.vertices,
        edges: raw.edges,
        faces: raw.faces,
        maximum_vertex_error_mm: raw.maximum_vertex_error_mm,
        maximum_curve_binding_error_mm: raw.maximum_curve_binding_error_mm,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_strip_loops_refuse_duplicate_uses_and_disconnected_endpoint_chains() {
        let body =
            Chamfer::new(super::super::super::tests::tool(false, 1, false), 0.3, None).unwrap();
        let raw = candidate(
            body,
            1e-7,
            &mut Work::new(QueryOptions::default(), None).unwrap(),
        )
        .unwrap();
        let edge = raw
            .edges
            .iter()
            .position(|e| e.vertices[0] != e.vertices[1])
            .unwrap();
        let u = EdgeUse {
            edge,
            forward: true,
            angular_turns: 0,
        };
        let work = &mut Work::new(QueryOptions::default(), None).unwrap();
        assert!(matches!(
            exterior(&raw.edges, vec![u, u], work),
            Err(Error::Unresolved)
        ));
        assert!(matches!(
            loops(&raw.edges, vec![u, u], work),
            Err(Error::Unresolved)
        ));
        assert!(matches!(
            loops(&raw.edges, vec![u], work),
            Err(Error::Unresolved)
        ));
    }
}
