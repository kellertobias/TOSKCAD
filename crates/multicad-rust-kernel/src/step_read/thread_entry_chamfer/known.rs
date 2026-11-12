//! Known-source proof; actual trim coupling is independent of source proposals.
use super::*;
use crate::step::thread_entry_chamfer::plan::{Group, Plan};
use polynomial::Curve;

struct Bindings<'a> {
    graph: &'a graph::Graph,
    wall: (u32, &'a surface::Surface),
    cone: (u32, &'a cone::Cone),
}
fn tail_edges(uses: &[(u32, bool)]) -> Result<[u32; 3]> {
    match uses {
        [(next, true), (top, false), (previous, false)] => Ok([*next, *top, *previous]),
        _ => Err(ReadError::Invalid),
    }
}
fn one_edge(wire: &[(u32, bool)], forward: bool) -> Result<u32> {
    match wire {
        [(edge, sense)] if *sense == forward => Ok(*edge),
        _ => Err(ReadError::Invalid),
    }
}
fn cap_layout(entry: usize, cap: usize, neck: bool) -> Result<()> {
    require(entry == 1 && cap == 1 + usize::from(neck))
}
fn circle_binding(edge: &graph::Edge, forward: bool, surface: u32) -> Result<u32> {
    require(
        edge.forward == forward
            && !edge.seam
            && edge.pcurves.len() == 1
            && edge.pcurves[0].0 == surface,
    )?;
    Ok(edge.pcurves[0].1)
}
pub(super) struct Verify<'a> {
    pub body: &'a Chamfer,
    pub frame: [Point; 3],
    pub tolerance: f64,
    pub maximum: f64,
    pub bound_curves: usize,
}
impl Verify<'_> {
    fn world(&self, [x, y, z]: Point) -> Point {
        let [o, u, v] = self.frame;
        let n = cross(u, v);
        std::array::from_fn(|i| o[i] + u[i] * x + v[i] * y + n[i] * z)
    }
    fn expected_curve(
        &self,
        actual: &Curve<3>,
        degree: usize,
        knots: &[f64],
        points: impl Iterator<Item = Point>,
    ) -> Result<()> {
        require(actual.degree == degree && actual.knots == knots)?;
        let mut count = 0;
        for (i, p) in points.enumerate() {
            require(actual.poles.get(i) == Some(&p.map(I::point)))?;
            count += 1;
        }
        require(actual.poles.len() == count)
    }
    fn expected_uv(
        &self,
        shell: &Shell<'_, '_>,
        id: u32,
        degree: usize,
        knots: &[f64],
        points: impl Iterator<Item = [f64; 2]>,
    ) -> Result<Curve<2>> {
        let actual = Curve::<2>::read(shell, id)?;
        require(actual.degree == degree && actual.knots == knots)?;
        let mut count = 0;
        for (i, p) in points.enumerate() {
            require(actual.poles.get(i) == Some(&p.map(I::point)))?;
            count += 1;
        }
        require(actual.poles.len() == count)?;
        Ok(actual)
    }
    fn plane(&self, shell: &Shell<'_, '_>, face: &graph::Face, z: f64, up: bool) -> Result<()> {
        require(face.forward)?;
        let [Value::String(_), axis] = shell.args(face.surface, "PLANE")? else {
            return Err(ReadError::Invalid);
        };
        let [_, u, v] = self.frame;
        let expected = frame::Frame::expected([
            self.world([0., 0., z]),
            u,
            v.map(|x| if up { x } else { -x }),
        ]);
        let actual = frame::Frame::read(shell, reference(axis)?)?;
        require(
            actual.support_error(
                &expected,
                [self.body.source_thread().spec().major_radius_mm; 3],
            )? <= self.tolerance / 16.,
        )
    }
    pub(super) fn cone(&self, shell: &Shell<'_, '_>, surface: u32) -> Result<cone::Cone> {
        let [
            Value::String(_),
            axis,
            Value::Number(radius),
            Value::Number(angle),
        ] = shell.args(surface, "CONICAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        let r = self.body.entry_radius_mm();
        let k =
            (self.body.source_thread().spec().major_radius_mm - r) / self.body.entry_length_mm();
        require(*radius == r && *angle == k.atan())?;
        require(
            frame::Frame::read(shell, reference(axis)?)?.support_error(
                &frame::Frame::expected(self.frame),
                [
                    self.body.source_thread().spec().major_radius_mm,
                    self.body.source_thread().spec().major_radius_mm,
                    self.body.full_length_mm(),
                ],
            )? <= self.tolerance / 16.,
        )?;
        cone::Cone::read(shell, surface)
    }
    fn bind(&mut self, error: f64) -> Result<()> {
        require(error <= self.tolerance / 2.)?;
        self.maximum = self.maximum.max(error);
        self.bound_curves += 1;
        Ok(())
    }
    fn group(
        &mut self,
        shell: &mut Shell<'_, '_>,
        id: u32,
        g: &Group,
        bindings: Bindings<'_>,
        work: &mut Work,
    ) -> Result<()> {
        let Bindings {
            graph,
            wall: (surface, wall),
            cone: (cone_id, cone),
        } = bindings;
        let e = &graph.edges[&id];
        require(e.forward && !e.seam && e.pcurves.len() == if g.active { 2 } else { 1 })?;
        require(e.pcurves[0].0 == surface && (!g.active || e.pcurves[1].0 == cone_id))?;
        let knots = std::iter::once(g.trims[0].phase[0])
            .chain(g.trims.iter().map(|t| t.phase[1]))
            .collect::<Vec<_>>();
        let points = g.trims.iter().enumerate().flat_map(|(j, t)| {
            t.poles
                .iter()
                .skip(usize::from(j != 0))
                .map(|&p| self.world(p))
        });
        let curve = graph.polynomial(shell, id)?;
        self.expected_curve(&curve, 3, &knots, points)?;
        let h = self.body.source_thread().helical_rate().abs();
        let points = g.trims.iter().enumerate().flat_map(|(j, t)| {
            (usize::from(j != 0)..4).map(move |i| [t.surface_u_poles[i], t.height_poles[i] / h])
        });
        let uv = self.expected_uv(shell, e.pcurves[0].1, 3, &knots, points)?;
        self.bind(wall.verify(&curve, &uv, self.tolerance / 2., work)?)?;
        if g.active {
            let k = (self.body.source_thread().spec().major_radius_mm
                - self.body.entry_radius_mm())
                / self.body.entry_length_mm();
            let cosine = k.atan().cos();
            let points = g.trims.iter().enumerate().flat_map(|(j, t)| {
                (usize::from(j != 0)..4)
                    .map(move |i| [t.angle_poles[i], t.height_poles[i] / cosine])
            });
            let uv = self.expected_uv(shell, e.pcurves[1].1, 3, &knots, points)?;
            self.bind(cone.verify(&curve, &uv, self.tolerance / 2., work)?)?;
        }
        Ok(())
    }
    pub fn walls(
        &mut self,
        shell: &mut Shell<'_, '_>,
        graph: &graph::Graph,
        plan: &Plan,
        cone: &cone::Cone,
        work: &mut Work,
    ) -> Result<(Vec<u32>, Vec<u32>, Vec<u32>)> {
        let count = plan.source.bands.len();
        let cone_id = graph.faces[count + 2].surface;
        let mut groups = vec![0; plan.groups.len()];
        let mut rails = vec![0; count];
        let mut tops = vec![0; count];
        let mut next = vec![0; count];
        let source = self.body.source_thread();
        let rate = source.helical_rate();
        let f = self.body.material_length_mm();
        let spin = [(f / rate).cos(), (f / rate).sin()];
        for i in 0..count {
            let face = &graph.faces[i];
            require(face.forward == source.spec().left_hand && face.wires.len() == 1)?;
            let exact = helical::Proof {
                error: None,
                extent: source.spec().major_radius_mm,
            };
            let radial = &plan.source.bands[i];
            let axial = &plan.source.axial;
            let heights = &plan.source.heights;
            helical::tensor_surface(
                exact,
                shell,
                face.surface,
                radial,
                axial,
                heights,
                self.frame,
            )?;
            let actual = surface::Surface::read(shell, face.surface, work)?;
            let uses = helical::orient(&face.wires[0], source.spec().left_hand);
            let mut expected = plan
                .groups
                .iter()
                .enumerate()
                .filter(|(_, g)| g.wall == i)
                .collect::<Vec<_>>();
            expected.sort_by(|(_, a), (_, b)| {
                a.trims[0].surface_u_poles[0].total_cmp(&b.trims[0].surface_u_poles[0])
            });
            require(uses.len() == expected.len() + 3)?;
            for (&(edge, forward), (j, g)) in uses.iter().zip(expected) {
                require(forward)?;
                groups[j] = edge;
                self.group(
                    shell,
                    edge,
                    g,
                    Bindings {
                        graph,
                        wall: (face.surface, &actual),
                        cone: (cone_id, cone),
                    },
                    work,
                )?;
            }
            let [n, t, p] = tail_edges(&uses[uses.len() - 3..])?;
            rails[i] = p;
            tops[i] = t;
            next[i] = n;
            let band = &plan.source.bands[i];
            let top = graph.polynomial(shell, t)?;
            require(graph.edges[&t].pcurves.is_empty() && graph.edges[&t].forward)?;
            let points = band.poles.iter().map(|&r| {
                let [x, y] = crate::step::helical::product(r, spin);
                self.world([x, y, f])
            });
            let degree = crate::step::helical::DEGREE;
            self.expected_curve(&top, degree, &band.knots, points)?;
            // Source knots are strictly ordered and finite; F/h is within the
            // proved complete original axial domain, so this literal isoline
            // needs no second fallible parser-constructor admission.
            let [a, b] = [band.knots[0], *band.knots.last().expect("source")];
            let v = I::point(f / rate.abs());
            let uv = Curve {
                degree: 1,
                knots: vec![a, b],
                poles: vec![[I::point(a), v], [I::point(b), v]],
            };
            self.bind(actual.verify(&top, &uv, self.tolerance / 2., work)?)?;
        }
        for i in 0..count {
            require(next[i] == rails[(i + 1) % count])?;
        }
        // Every remaining source-side rail is bound on both actual tensors.
        for i in 0..count {
            let e = &graph.edges[&rails[i]];
            require(e.forward && !e.seam && e.pcurves.len() == 2)?;
            let previous = (i + count - 1) % count;
            let supports = [e.pcurves[0].0, e.pcurves[1].0];
            let expected = [graph.faces[i].surface, graph.faces[previous].surface];
            require(supports == expected)?;
            let curve = graph.polynomial(shell, rails[i])?;
            let h = rate.abs();
            let q = source.profile_breaks()[[1, 2, 3, 4][i % 4]];
            let z_start = plan.points[&((i / 4) as u32, q.to_bits())][2];
            let a = z_start / h;
            require(curve.domain() == [a, f / h])?;
            for (j, (surface, pc)) in e.pcurves.iter().enumerate() {
                let band = &plan.source.bands[if j == 0 { i } else { previous }];
                let u = if j == 0 {
                    band.knots[0]
                } else {
                    *band.knots.last().expect("source")
                };
                let uv =
                    self.expected_uv(shell, *pc, 1, &[a, f / h], [[u, a], [u, f / h]].into_iter())?;
                let actual = surface::Surface::read(shell, *surface, work)?;
                self.bind(actual.verify(&curve, &uv, self.tolerance / 2., work)?)?;
            }
        }
        Ok((groups, rails, tops))
    }
}
impl Verify<'_> {
    fn circle(
        &self,
        shell: &mut Shell<'_, '_>,
        edge: &graph::Edge,
        radius: f64,
        z: f64,
    ) -> Result<frame::Frame> {
        let [Value::String(_), axis, Value::Number(r)] = shell.args(edge.curve, "CIRCLE")? else {
            return Err(ReadError::Invalid);
        };
        require(*r == radius)?;
        let [_, u, v] = self.frame;
        let frame = frame::Frame::read(shell, reference(axis)?)?;
        require(
            frame.support_error(
                &frame::Frame::expected([self.world([0., 0., z]), u, v]),
                [radius, radius, 0.],
            )? <= self.tolerance / 16.,
        )?;
        if edge.vertices[0] == edge.vertices[1] {
            let point = frame.point([I::point(radius), I::point(0.), I::point(0.)])?;
            let actual = shell.vertex(edge.vertices[0])?.map(I::point);
            let bound = (0..3).fold(I::point(0.), |sum, i| sum + (point[i] - actual[i]).abs());
            require(bound.hi <= self.tolerance / 16.)?;
        }
        Ok(frame)
    }
    fn line(&self, shell: &mut Shell<'_, '_>, edge: &graph::Edge) -> Result<()> {
        let [Value::String(_), origin, vector] = shell.args(edge.curve, "LINE")? else {
            return Err(ReadError::Invalid);
        };
        let origin = shell.point(reference(origin)?)?.map(I::point);
        let [Value::String(_), direction, Value::Number(scale)] =
            shell.args(reference(vector)?, "VECTOR")?
        else {
            return Err(ReadError::Invalid);
        };
        require(scale.is_finite() && *scale > 0.)?;
        let direction = frame::unit(frame::direction(shell, reference(direction)?)?)?;
        let mut points = [[I::point(0.); 3]; 2];
        for (point, id) in points.iter_mut().zip(edge.vertices) {
            *point = shell.vertex(id)?.map(I::point);
            let d = std::array::from_fn::<_, 3, _>(|i| point[i] - origin[i]);
            let cross = [
                d[1] * direction[2] - d[2] * direction[1],
                d[2] * direction[0] - d[0] * direction[2],
                d[0] * direction[1] - d[1] * direction[0],
            ];
            require(
                polynomial::finite(cross.into_iter().fold(I::point(0.), |sum, v| sum + v.abs()))?
                    .hi
                    <= self.tolerance / 16.,
            )?;
        }
        let forward = (0..3).fold(I::point(0.), |sum, i| {
            sum + (points[1][i] - points[0][i]) * direction[i]
        });
        require(
            forward.lo.is_finite()
                && forward.hi.is_finite()
                && if edge.forward {
                    forward.lo > 0.
                } else {
                    forward.hi < 0.
                },
        )
    }
    fn cone_rail(
        &mut self,
        shell: &mut Shell<'_, '_>,
        graph: &graph::Graph,
        id: u32,
        at: (&Group, usize),
        support: (u32, &cone::Cone),
        work: &mut Work,
    ) -> Result<()> {
        let (group, end) = at;
        let (cone_id, cone) = support;
        let trim = if end == 0 {
            &group.trims[0]
        } else {
            group.trims.last().expect("source group")
        };
        let i = if end == 0 { 0 } else { 3 };
        let z = trim.poles[i][2];
        let q = trim.phase[end];
        let start = trim.start;
        let key = if q == self.body.source_thread().spec().pitch_mm {
            ((start + 1) % self.body.source_thread().spec().starts, 0.)
        } else {
            (start, q)
        };
        let rate = self.body.source_thread().helical_rate();
        let theta = if key == (0, 0.) {
            0.
        } else {
            -(q + f64::from(start) * self.body.source_thread().spec().pitch_mm) / rate
        };
        let seam = key == (0, 0.);
        let e = &graph.edges[&id];
        require(e.forward && e.seam == seam && e.pcurves.len() == if seam { 2 } else { 1 })?;
        let curve = graph.polynomial(shell, id)?;
        require(curve.domain() == [0., z / rate.abs()])?;
        let cosine = ((self.body.source_thread().spec().major_radius_mm
            - self.body.entry_radius_mm())
            / self.body.entry_length_mm())
        .atan()
        .cos();
        for (i, &(surface, pc)) in e.pcurves.iter().enumerate() {
            require(surface == cone_id)?;
            let shift = if i == 0 {
                0.
            } else {
                -rate.signum() * std::f64::consts::TAU
            };
            let domain = [0., z / rate.abs()];
            let ends = [[theta + shift, 0.], [theta + z / rate + shift, z / cosine]];
            let uv = self.expected_uv(shell, pc, 1, &domain, ends.into_iter())?;
            self.bind(cone.verify(&curve, &uv, self.tolerance / 2., work)?)?;
        }
        Ok(())
    }
    pub fn caps(
        &mut self,
        shell: &mut Shell<'_, '_>,
        graph: &graph::Graph,
        plan: &Plan,
        rings: (&[u32], &[u32]),
        cone: &cone::Cone,
        work: &mut Work,
    ) -> Result<()> {
        let (groups, tops) = rings;
        let count = plan.source.bands.len();
        let n = self.body.source_thread().spec().starts as usize;
        let left = self.body.source_thread().spec().left_hand;
        let rate = self.body.source_thread().helical_rate();
        let entry = &graph.faces[count];
        let cap = &graph.faces[count + 1];
        self.plane(shell, entry, 0., false)?;
        self.plane(shell, cap, self.body.material_length_mm(), true)?;
        let has_neck = self.body.terminal_relief().is_some();
        cap_layout(entry.wires.len(), cap.wires.len(), has_neck)?;
        let entry = helical::orient(&entry.wires[0], !left);
        require(entry.len() == groups.len())?;
        let mut bottom = Vec::with_capacity(groups.len());
        let cone_id = graph.faces[count + 2].surface;
        for (i, &(edge, forward)) in entry.iter().enumerate() {
            require(forward)?;
            bottom.push(edge);
            let g = &plan.groups[i];
            let e = &graph.edges[&edge];
            if !g.active {
                require(edge == groups[i])?;
                continue;
            }
            require(edge != groups[i])?;
            let pc = circle_binding(e, rate < 0., cone_id)?;
            let circle =
                self.circle(shell, &graph.edges[&edge], self.body.entry_radius_mm(), 0.)?;
            let a = g.trims[0].phase[0];
            let b = g.trims.last().expect("source group").phase[1];
            let start = g.trims[0].start;
            let angles = [a, b].map(|q| {
                -(q + f64::from(start) * self.body.source_thread().spec().pitch_mm) / rate
            });
            let domain = if angles[0] < angles[1] {
                angles
            } else {
                [angles[1], angles[0]]
            };
            let ends = [[domain[0], 0.], [domain[1], 0.]];
            self.expected_uv(shell, pc, 1, &domain, ends.into_iter())?;
            for (vertex, theta) in e.vertices.into_iter().zip(angles) {
                let theta = I::point(theta);
                let r = I::point(self.body.entry_radius_mm());
                let point = circle.point([r * theta.cos(), r * theta.sin(), I::point(0.)])?;
                let actual = shell.vertex(vertex)?.map(I::point);
                let bound = (0..3).fold(I::point(0.), |sum, i| sum + (point[i] - actual[i]).abs());
                require(bound.hi <= self.tolerance / 16.)?;
            }
        }
        let outer = helical::orient(&cap.wires[0], left);
        let expected = tops.iter().map(|&e| (e, true)).collect::<Vec<_>>();
        require(outer == expected)?;
        for i in 0..2 * n {
            let face = &graph.faces[count + 2 + i];
            require(face.surface == cone_id && face.forward && face.wires.len() == 1)?;
            let ids = plan
                .groups
                .iter()
                .enumerate()
                .filter(|(_, g)| g.active && g.cone == i)
                .map(|(j, _)| j)
                .collect::<Vec<_>>();
            require(!ids.is_empty())?;
            let first = ids[0];
            let last = *ids.last().expect("nonempty");
            let first_group = &plan.groups[first];
            let last_group = &plan.groups[last];
            let start = first_group.trims[0].poles[0][2] > 0.;
            let end = last_group.trims.last().expect("source").poles[3][2] > 0.;
            let uses = helical::orient(&face.wires[0], left);
            require(uses.len() == 2 * ids.len() + usize::from(start) + usize::from(end))?;
            let mut k = 0;
            for &j in &ids {
                require(uses[k] == (bottom[j], true))?;
                k += 1;
            }
            if end {
                let (edge, forward) = uses[k];
                require(forward)?;
                self.cone_rail(shell, graph, edge, (last_group, 1), (cone_id, cone), work)?;
                k += 1;
            }
            for &j in ids.iter().rev() {
                require(uses[k] == (groups[j], false))?;
                k += 1;
            }
            if start {
                let (edge, forward) = uses[k];
                require(!forward)?;
                self.cone_rail(shell, graph, edge, (first_group, 0), (cone_id, cone), work)?;
            }
        }
        if let Some(relief) = self.body.terminal_relief() {
            let lower = one_edge(&cap.wires[1], false)?;
            return self.neck(shell, graph, lower, (count, n), relief);
        }
        Ok(())
    }
    fn neck(
        &mut self,
        shell: &mut Shell<'_, '_>,
        graph: &graph::Graph,
        lower: u32,
        position: (usize, usize),
        relief: &crate::thread_end_relief::Relief,
    ) -> Result<()> {
        let (count, n) = position;
        let far = &graph.faces[count + 3 + 2 * n];
        self.plane(shell, far, self.body.full_length_mm(), true)?;
        require(far.wires.len() == 1)?;
        let upper = one_edge(&far.wires[0], true)?;
        let radius = relief.neck_radius_mm();
        let f = self.body.material_length_mm();
        self.circle(shell, &graph.edges[&lower], radius, f)?;
        let h = self.body.full_length_mm();
        self.circle(shell, &graph.edges[&upper], radius, h)?;
        let wall = &graph.faces[count + 2 + 2 * n];
        let [Value::String(_), axis, Value::Number(actual_radius)] =
            shell.args(wall.surface, "CYLINDRICAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        require(*actual_radius == radius)?;
        let actual = frame::Frame::read(shell, reference(axis)?)?;
        let [_, u, v] = self.frame;
        let expected = frame::Frame::expected([self.world([0., 0., f]), u, v]);
        let extents = [radius, radius, h - f];
        require(actual.support_error(&expected, extents)? <= self.tolerance / 16.)?;
        require(wall.wires.len() == 1 && wall.wires[0].len() == 4)?;
        self.line(shell, &graph.edges[&wall.wires[0][1].0])?;
        shell.edge(lower)?;
        shell.edge(upper)?;
        let exact = helical::Proof {
            error: None,
            extent: h,
        };
        let face = graph.faces[count + 2 + 2 * n].id;
        let rims = [lower, upper];
        let frame = [
            self.world([0., 0., f]),
            cross(self.frame[1], self.frame[2]),
            self.frame[1],
        ];
        helical::check_cylinder_with_proof(exact, shell, face, rims, (radius, true), frame)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body() -> Chamfer {
        Chamfer::new(
            crate::helical_thread::Thread::new(crate::helical_thread::Spec {
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
            .unwrap(),
            0.4,
            None,
        )
        .unwrap()
    }
    fn verifier(body: &Chamfer) -> Verify<'_> {
        Verify {
            body,
            frame: [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
            tolerance: 1e-7,
            maximum: 0.,
            bound_curves: 0,
        }
    }
    fn work() -> Work<'static> {
        Work {
            remaining: 512_000_000,
            interrupt: None,
            cancelled: false,
        }
    }
    #[test]
    fn raw_semantic_wire_and_analytic_support_records_are_proved_before_binding_material() {
        assert_eq!(
            tail_edges(&[(1, true), (2, false), (3, false)]).unwrap(),
            [1, 2, 3]
        );
        assert!(tail_edges(&[(1, false), (2, false), (3, false)]).is_err());
        assert!(one_edge(&[(1, true), (2, true)], true).is_err());
        assert!(one_edge(&[(1, false)], true).is_err());
        let body = body();
        let proof = verifier(&body);
        let face = graph::Face {
            id: 1,
            surface: 1,
            forward: true,
            wires: vec![],
        };
        with_shell("#1=PLANE('');", |s| {
            assert!(proof.plane(s, &face, 0., true).is_err())
        });
        with_shell("#1=CONICAL_SURFACE('');", |s| {
            assert!(proof.cone(s, 1).is_err())
        });
        let axis = "#10=CARTESIAN_POINT('',(1.,0.,0.));#11=DIRECTION('',(0.,0.,1.));#12=DIRECTION('',(1.,0.,0.));#13=AXIS2_PLACEMENT_3D('',#10,#11,#12);";
        let angle = ((body.source_thread().spec().major_radius_mm - body.entry_radius_mm())
            / body.entry_length_mm())
        .atan();
        with_shell(
            &format!(
                "{axis}#1=CONICAL_SURFACE('',#13,{:.16E},{angle:.16E});",
                body.entry_radius_mm()
            ),
            |s| assert!(proof.cone(s, 1).is_err()),
        );
        for data in [
            "#1=CIRCLE('');",
            "#1=CIRCLE('',#13,2.7);",
            "#1=CIRCLE('',#13,2.6);",
        ] {
            with_shell(
                &format!(
                    "{axis}{data}#2=EDGE_CURVE('',#3,#3,#1,.T.);#3=VERTEX_POINT('',#4);#4=CARTESIAN_POINT('',(2.6,0.,0.));"
                ),
                |s| {
                    let edge = graph::Edge::read(s, 2, &mut work()).unwrap();
                    assert!(proof.circle(s, &edge, 2.6, 0.).is_err());
                },
            );
        }
        for (line, vector, direction, end, forward) in [
            (
                "LINE('')",
                "VECTOR('',#8,1.)",
                "(0.,0.,1.)",
                "(0.,0.,1.)",
                "T",
            ),
            (
                "LINE('',#5,#7)",
                "VECTOR('')",
                "(0.,0.,1.)",
                "(0.,0.,1.)",
                "T",
            ),
            (
                "LINE('',#5,#7)",
                "VECTOR('',#8,1.)",
                "(0.,0.,1.)",
                "(1.,0.,0.)",
                "T",
            ),
            (
                "LINE('',#5,#7)",
                "VECTOR('',#8,1.)",
                "(0.,0.,-1.)",
                "(0.,0.,1.)",
                "F",
            ),
        ] {
            with_shell(
                &format!(
                    "#1={line};#2=EDGE_CURVE('',#3,#4,#1,.{forward}.);#3=VERTEX_POINT('',#5);#4=VERTEX_POINT('',#6);#5=CARTESIAN_POINT('',(0.,0.,0.));#6=CARTESIAN_POINT('',{end});#7={vector};#8=DIRECTION('',{direction});"
                ),
                |s| {
                    let edge = graph::Edge::read(s, 2, &mut work()).unwrap();
                    let result = proof.line(s, &edge);
                    assert_eq!(result.is_ok(), forward == "F");
                },
            );
        }
    }
    #[test]
    fn cumulative_material_binding_limits_stop_at_each_real_stage_and_allow_a_fresh_retry() {
        let body = body();
        let [o, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text = crate::step::write_thread_entry_chamfer(&body, o, u, v, 1e-7).unwrap();
        let entities = parse_entities(&text, 1e-7).unwrap();
        let mut shell = Shell::new(&entities, 1e-7, true);
        let solid = shell.represented_solids(true).unwrap()[0];
        let graph = graph::Graph::read(&mut shell, solid, &mut work()).unwrap();
        let plan =
            crate::step::thread_entry_chamfer::read_plan(&body, [o, u, v], 1e-7, None).unwrap();
        let mut proof = verifier(&body);
        let cone_id = graph.faces[plan.source.bands.len() + 2].surface;
        let cone = proof.cone(&shell, cone_id).unwrap();
        let mut measured = work();
        let maximum = measured.remaining;
        let surface =
            surface::Surface::read(&shell, graph.faces[0].surface, &mut measured).unwrap();
        let read_cost = maximum - measured.remaining;
        let uses = helical::orient(&graph.faces[0].wires[0], false);
        let mut groups = plan
            .groups
            .iter()
            .filter(|g| g.wall == 0)
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| {
            a.trims[0].surface_u_poles[0].total_cmp(&b.trims[0].surface_u_poles[0])
        });
        for (&(edge, _), g) in uses.iter().zip(&groups) {
            proof
                .group(
                    &mut shell,
                    edge,
                    g,
                    Bindings {
                        graph: &graph,
                        wall: (graph.faces[0].surface, &surface),
                        cone: (cone_id, &cone),
                    },
                    &mut measured,
                )
                .unwrap();
        }
        let before_top = maximum - measured.remaining;
        let mut complete = work();
        verifier(&body)
            .walls(&mut shell, &graph, &plan, &cone, &mut complete)
            .unwrap();
        let entire = maximum - complete.remaining;
        for capacity in [0, read_cost, before_top, entire - 1] {
            assert!(matches!(
                verifier(&body).walls(
                    &mut shell,
                    &graph,
                    &plan,
                    &cone,
                    &mut Work {
                        remaining: capacity,
                        interrupt: None,
                        cancelled: false
                    }
                ),
                Err(ReadError::Limit)
            ));
        }
        let g = groups[0];
        let edge = uses[0].0;
        let curve = graph.polynomial(&mut shell, edge).unwrap();
        let uv = Curve::<2>::read(&shell, graph.edges[&edge].pcurves[0].1).unwrap();
        let surface = surface::Surface::read(&shell, graph.faces[0].surface, &mut work()).unwrap();
        let mut measured = work();
        surface.verify(&curve, &uv, 5e-8, &mut measured).unwrap();
        let first_support = maximum - measured.remaining;
        let surface = surface::Surface::read(&shell, graph.faces[0].surface, &mut work()).unwrap();
        assert!(matches!(
            verifier(&body).group(
                &mut shell,
                edge,
                g,
                Bindings {
                    graph: &graph,
                    wall: (graph.faces[0].surface, &surface),
                    cone: (cone_id, &cone)
                },
                &mut Work {
                    remaining: first_support,
                    interrupt: None,
                    cancelled: false
                }
            ),
            Err(ReadError::Limit)
        ));
        assert!(
            verifier(&body)
                .walls(&mut shell, &graph, &plan, &cone, &mut work())
                .is_ok()
        );
    }
}

#[cfg(test)]
mod closure_tests {
    use super::*;
    fn body() -> Chamfer {
        let thread = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 300.,
            pitch_mm: 100.,
            length_mm: 1000.,
            depth_mm: 50.,
            crest_width_mm: 12.5,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        Chamfer::new(
            thread,
            40.,
            Some(crate::thread_end_relief::Intent {
                width_mm: 600.,
                radial_depth_mm: 10.,
            }),
        )
        .unwrap()
    }
    fn work() -> Work<'static> {
        Work {
            remaining: 10_000_000,
            interrupt: None,
            cancelled: false,
        }
    }
    fn verifier(body: &Chamfer) -> Verify<'_> {
        Verify {
            body,
            frame: [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
            tolerance: 1e-7,
            maximum: 0.,
            bound_curves: 0,
        }
    }
    #[test]
    fn raw_cap_and_circle_associations_cannot_substitute_another_material_boundary() {
        assert!(cap_layout(1, 1, false).is_ok());
        assert!(cap_layout(0, 1, false).is_err());
        assert!(cap_layout(1, 1, true).is_err());
        let raw = graph::Edge {
            vertices: [1, 2],
            curve: 3,
            forward: false,
            pcurves: vec![(4, 5)],
            seam: false,
        };
        assert_eq!(circle_binding(&raw, false, 4).unwrap(), 5);
        assert!(circle_binding(&raw, true, 4).is_err());
        assert!(circle_binding(&raw, false, 6).is_err());
    }
    #[test]
    fn actual_terminal_closure_rejects_changed_circles_raw_cylinders_extent_and_material_side() {
        let body = body();
        let text = crate::step::write_thread_entry_chamfer(
            &body,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7,
        )
        .unwrap();
        for case in 0..5 {
            let mut entities = parse_entities(&text, 1e-7).unwrap();
            let (solid, lower, upper, wall, cylinder, lower_curve, upper_curve, normal) = {
                let mut shell = Shell::new(&entities, 1e-7, true);
                let solid = shell.represented_solids(true).unwrap()[0];
                let graph = graph::Graph::read(&mut shell, solid, &mut work()).unwrap();
                let lower = one_edge(&graph.faces[5].wires[1], false).unwrap();
                let upper = one_edge(&graph.faces[9].wires[0], true).unwrap();
                let wall = &graph.faces[8];
                let axis = reference(&shell.args(wall.surface, "CYLINDRICAL_SURFACE").unwrap()[1])
                    .unwrap();
                let normal =
                    reference(&shell.args(axis, "AXIS2_PLACEMENT_3D").unwrap()[2]).unwrap();
                (
                    solid,
                    lower,
                    upper,
                    wall.id,
                    wall.surface,
                    graph.edges[&lower].curve,
                    graph.edges[&upper].curve,
                    normal,
                )
            };
            match case {
                0 => entities.get_mut(&lower_curve).unwrap()[0].args[2] = Value::Number(241.),
                1 => entities.get_mut(&upper_curve).unwrap()[0].args[2] = Value::Number(241.),
                2 => {
                    entities.get_mut(&cylinder).unwrap()[0].args.pop();
                }
                3 => {
                    entities.get_mut(&normal).unwrap()[0].args[1] = Value::List(vec![
                        Value::Number(1e-11),
                        Value::Number(0.),
                        Value::Number(1.),
                    ])
                }
                _ => entities.get_mut(&wall).unwrap()[0].args[3] = Value::Enum("F"),
            }
            let mut shell = Shell::new(&entities, 1e-7, true);
            let graph = graph::Graph::read(&mut shell, solid, &mut work()).unwrap();
            let mut proof = verifier(&body);
            if case == 3 {
                assert!(
                    proof
                        .circle(&mut shell, &graph.edges[&lower], 240., 400.)
                        .is_ok(),
                    "the cap-local radius alone must not reject this full-height counterexample"
                );
            }
            assert!(
                proof
                    .neck(
                        &mut shell,
                        &graph,
                        lower,
                        (4, 1),
                        body.terminal_relief().unwrap()
                    )
                    .is_err(),
                "case {case}"
            );
            assert_eq!(one_edge(&graph.faces[9].wires[0], true).unwrap(), upper);
        }
    }
}

#[cfg(test)]
mod tensor_identity_tests {
    use super::*;
    #[test]
    fn an_altered_untrimmed_tensor_pole_cannot_hide_behind_unchanged_closed_wires() {
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
        let body = Chamfer::new(tool, 0.4, None).unwrap();
        let [o, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text = crate::step::write_thread_entry_chamfer(&body, o, u, v, 1e-7).unwrap();
        let mut entities = parse_entities(&text, 1e-7).unwrap();
        let mut work = Work {
            remaining: 512_000_000,
            interrupt: None,
            cancelled: false,
        };
        let (solid, point) = {
            let mut shell = Shell::new(&entities, 1e-7, true);
            let solid = shell.represented_solids(true).unwrap()[0];
            let graph = graph::Graph::read(&mut shell, solid, &mut work).unwrap();
            let point=*entities.iter().find(|(_,parts)|matches!(parts.as_slice(),[entity] if entity.name=="CARTESIAN_POINT")).unwrap().0;
            assert!(!graph.edges.values().flat_map(|e| e.vertices).any(|v| {
                reference(&shell.args(v, "VERTEX_POINT").unwrap()[1]).unwrap() == point
            }));
            (solid, point)
        };
        let position = Shell::new(&entities, 1e-7, true).point(point).unwrap();
        entities.get_mut(&point).unwrap()[0].args[1] = Value::List(vec![
            Value::Number(position[0]),
            Value::Number(position[1]),
            Value::Number(0.01),
        ]);
        let mut shell = Shell::new(&entities, 1e-7, true);
        let graph = graph::Graph::read(&mut shell, solid, &mut work).unwrap();
        let plan =
            crate::step::thread_entry_chamfer::read_plan(&body, [o, u, v], 1e-7, None).unwrap();
        let mut proof = Verify {
            body: &body,
            frame: [o, u, v],
            tolerance: 1e-7,
            maximum: 0.,
            bound_curves: 0,
        };
        let cone = proof.cone(&shell, graph.faces[6].surface).unwrap();
        assert!(matches!(
            proof.walls(&mut shell, &graph, &plan, &cone, &mut work),
            Err(ReadError::Invalid)
        ));
    }
}
