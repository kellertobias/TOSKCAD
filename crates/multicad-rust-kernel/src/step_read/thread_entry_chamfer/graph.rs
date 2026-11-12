//! Exclusively owned, connected, consistently oriented source wires.
use super::*;
pub(super) type Wire = Vec<(u32, bool)>;
pub(super) struct Face {
    pub id: u32,
    pub surface: u32,
    pub forward: bool,
    pub wires: Vec<Wire>,
}
pub(super) struct Edge {
    pub vertices: [u32; 2],
    pub curve: u32,
    pub forward: bool,
    pub pcurves: Vec<(u32, u32)>,
    pub seam: bool,
}
pub(super) struct Graph {
    pub faces: Vec<Face>,
    pub edges: BTreeMap<u32, Edge>,
}
impl Edge {
    pub(super) fn read(shell: &Shell<'_, '_>, id: u32, work: &mut Work) -> Result<Self> {
        work.charge(1)?;
        let [Value::String(_), a, b, geometry, forward] = shell.args(id, "EDGE_CURVE")? else {
            return Err(ReadError::Invalid);
        };
        let mut geometry = reference(geometry)?;
        let wrapped = shell.simple(geometry)?;
        let seam = wrapped.name == "SEAM_CURVE";
        let mut pcurves = Vec::new();
        if seam || wrapped.name == "SURFACE_CURVE" {
            let [Value::String(_), curve, associated, Value::Enum(master)] =
                wrapped.args.as_slice()
            else {
                return Err(ReadError::Invalid);
            };
            require(*master == if seam { "PCURVE_S1" } else { "CURVE_3D" })?;
            let ids = refs(associated)?;
            require(
                (1..=2).contains(&ids.len())
                    && ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
            )?;
            for id in ids {
                pcurves.push(shell.pcurve_definition(id)?)
            }
            require(!seam || pcurves.len() == 2 && pcurves[0].0 == pcurves[1].0)?;
            geometry = reference(curve)?;
        }
        Ok(Self {
            vertices: [reference(a)?, reference(b)?],
            curve: geometry,
            forward: sense(forward)?,
            pcurves,
            seam,
        })
    }
}
impl Graph {
    pub fn read(shell: &mut Shell<'_, '_>, id: u32, work: &mut Work) -> Result<Self> {
        let ids = shell.member_faces(id)?;
        require(!ids.is_empty() && ids.iter().collect::<BTreeSet<_>>().len() == ids.len())?;
        let mut faces = Vec::with_capacity(ids.len());
        let mut edges = BTreeMap::new();
        let mut uses: BTreeMap<u32, Vec<(usize, bool)>> = BTreeMap::new();
        let mut owned = BTreeSet::new();
        for (i, id) in ids.into_iter().enumerate() {
            let (surface, forward, bounds) = face(shell, id)?;
            require(!bounds.is_empty())?;
            let mut wires = Vec::new();
            for (j, bound) in bounds.into_iter().enumerate() {
                work.charge(1)?;
                require(owned.insert(bound))?;
                let wire = ring(shell, bound, j == 0)?;
                require(!wire.is_empty())?;
                let mut ends = Vec::with_capacity(wire.len());
                for &(edge, forward) in &wire {
                    let entry = uses.entry(edge).or_default();
                    require(entry.len() < 2)?;
                    entry.push((i, forward));
                    if let std::collections::btree_map::Entry::Vacant(e) = edges.entry(edge) {
                        e.insert(Edge::read(shell, edge, work)?);
                    }
                    let mut vertices = edges[&edge].vertices;
                    for v in vertices {
                        shell.vertex(v)?;
                    }
                    if !forward {
                        vertices.reverse()
                    }
                    ends.push(vertices);
                }
                require((0..ends.len()).all(|k| ends[k][1] == ends[(k + 1) % ends.len()][0]))?;
                wires.push(wire);
            }
            faces.push(Face {
                id,
                surface,
                forward,
                wires,
            });
        }
        let mut adjacency = vec![BTreeSet::new(); faces.len()];
        let mut fans: BTreeMap<u32, Vec<[usize; 2]>> = BTreeMap::new();
        for (edge, u) in uses {
            require(u.len() == 2 && u[0].1 != u[1].1)?;
            let [a, b] = [u[0].0, u[1].0];
            if a == b {
                let e = &edges[&edge];
                require(
                    e.seam
                        || shell.simple(faces[a].surface)?.name == "CYLINDRICAL_SURFACE"
                            && shell.simple(e.curve)?.name == "LINE"
                            && e.vertices[0] != e.vertices[1],
                )?;
                continue;
            }
            adjacency[a].insert(b);
            adjacency[b].insert(a);
            for v in edges[&edge].vertices {
                fans.entry(v).or_default().push([a, b]);
            }
        }
        require(super::super::connected(0, |face| adjacency[face].iter().copied()) == faces.len())?;
        for pairs in fans.values() {
            let mut fan: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for &[a, b] in pairs {
                fan.entry(a).or_default().push(b);
                fan.entry(b).or_default().push(a);
            }
            require(
                fan.values().all(|a| a.len() == 2)
                    && super::super::connected(pairs[0][0], |f| fan[&f].iter().copied())
                        == fan.len(),
            )?;
        }
        Ok(Self { faces, edges })
    }
    pub fn polynomial(&self, shell: &mut Shell<'_, '_>, id: u32) -> Result<polynomial::Curve<3>> {
        self.edges[&id].polynomial(shell)
    }
}
impl Edge {
    fn polynomial(&self, shell: &mut Shell<'_, '_>) -> Result<polynomial::Curve<3>> {
        let e = self;
        let curve = polynomial::Curve::<3>::read(shell, e.curve)?;
        let ends = [
            curve.poles[0],
            *curve.poles.last().expect("validated curve"),
        ];
        let mut vertices = e.vertices;
        if !e.forward {
            vertices.reverse()
        }
        for (expected, id) in ends.into_iter().zip(vertices) {
            require(expected == shell.vertex(id)?.map(I::point))?;
        }
        Ok(curve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn work() -> Work<'static> {
        Work {
            remaining: 10_000_000,
            interrupt: None,
            cancelled: false,
        }
    }
    fn source(neck: bool) -> Chamfer {
        let tool = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        Chamfer::new(
            tool,
            0.4,
            neck.then_some(crate::thread_end_relief::Intent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            }),
        )
        .unwrap()
    }
    #[test]
    fn untrusted_edge_wrappers_and_reversed_polynomial_endpoints_are_checked_independently() {
        with_shell("#1=EDGE_CURVE('');", |s| {
            assert!(Edge::read(s, 1, &mut work()).is_err())
        });
        for wrapper in ["SURFACE_CURVE('')", "SURFACE_CURVE('',#2,(),.CURVE_3D.)"] {
            with_shell(
                &format!("#1=EDGE_CURVE('',#3,#4,#5,.T.);#2=LINE();#5={wrapper};"),
                |s| assert!(Edge::read(s, 1, &mut work()).is_err()),
            );
        }
        with_shell(
            "#1=EDGE_CURVE('',#2,#3,#4,.F.);#2=VERTEX_POINT('',#5);#3=VERTEX_POINT('',#6);#4=B_SPLINE_CURVE_WITH_KNOTS('',1,(#6,#5),.UNSPECIFIED.,.F.,.F.,(2,2),(0.,1.),.UNSPECIFIED.);#5=CARTESIAN_POINT('',(0.,0.,0.));#6=CARTESIAN_POINT('',(1.,0.,0.));",
            |s| {
                let edge = Edge::read(s, 1, &mut work()).unwrap();
                assert_eq!(
                    edge.polynomial(s).unwrap().poles[0],
                    [I::point(1.), I::point(0.), I::point(0.)]
                );
            },
        );
    }
    #[test]
    fn graph_rejects_pinched_vertices_false_periodic_planes_and_disconnected_material_members() {
        let body = source(true);
        let text = crate::step::write_thread_entry_chamfer(
            &body,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7,
        )
        .unwrap();
        let mut entities = parse_entities(&text, 1e-7).unwrap();
        let (id, a, b) = {
            let mut shell = Shell::new(&entities, 1e-7, true);
            let id = shell.represented_solids(true).unwrap()[0];
            let graph = Graph::read(&mut shell, id, &mut work()).unwrap();
            let a = graph.edges.values().next().unwrap().vertices[0];
            let b = graph
                .edges
                .values()
                .flat_map(|e| e.vertices)
                .find(|&b| {
                    b != a
                        && !graph
                            .edges
                            .values()
                            .any(|e| e.vertices.contains(&a) && e.vertices.contains(&b))
                })
                .unwrap();
            (id, a, b)
        };
        for entity in entities
            .values_mut()
            .flatten()
            .filter(|e| e.name == "EDGE_CURVE")
        {
            for value in &mut entity.args[1..3] {
                if *value == Value::Reference(b) {
                    *value = Value::Reference(a);
                }
            }
        }
        assert!(Graph::read(&mut Shell::new(&entities, 1e-7, true), id, &mut work()).is_err());
        let mut entities = parse_entities(&text, 1e-7).unwrap();
        let cylinder = entities
            .values_mut()
            .flatten()
            .find(|e| e.name == "CYLINDRICAL_SURFACE")
            .unwrap();
        cylinder.name = "PLANE";
        assert!(Graph::read(&mut Shell::new(&entities, 1e-7, true), id, &mut work()).is_err());
        let body = source(false);
        let mut writer = crate::step::Writer::new();
        for x in [0., 20.] {
            writer
                .append_thread_entry_chamfer(&body, [x, 0., 0.], [1., 0., 0.], [0., 1., 0.], 1e-7)
                .unwrap();
        }
        let text = writer.finish().unwrap();
        let mut entities = parse_entities(&text, 1e-7).unwrap();
        let (id, shell_id, faces) = {
            let s = Shell::new(&entities, 1e-7, true);
            let ids = s.represented_solids(false).unwrap();
            let mut faces = s.member_faces(ids[0]).unwrap();
            faces.extend(s.member_faces(ids[1]).unwrap());
            (
                ids[0],
                reference(&s.args(ids[0], "MANIFOLD_SOLID_BREP").unwrap()[1]).unwrap(),
                faces,
            )
        };
        entities.get_mut(&shell_id).unwrap()[0].args[1] =
            Value::List(faces.into_iter().map(Value::Reference).collect());
        assert!(Graph::read(&mut Shell::new(&entities, 1e-7, true), id, &mut work()).is_err());
    }
}
