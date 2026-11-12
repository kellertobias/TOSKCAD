//! Shared source wires on exact cone/plane and certified helical surfaces.
use super::{curves::*, plan::*, *};
use crate::normal_relief::precision::{add_up, dot_error, mul_up};

struct Shell<'a> {
    file: &'a mut StepFile,
    body: &'a Chamfer,
    placement: Placement,
    tolerance: f64,
    uv: UvContext,
    walls: Vec<Entity>,
    cone: Entity,
    vertices: HashMap<(u32, u64), Vertex>,
    bottom: HashMap<(u32, u64), Vertex>,
    cone_rails: HashMap<(u32, u64), Option<Entity>>,
    uses: HashMap<usize, (u8, i8)>,
    interrupt: Option<&'a dyn Fn() -> bool>,
}

fn certified_heights(
    curve: &helical::Spline,
    ends: [f64; 2],
    slope: f64,
    tolerance: f64,
) -> Result<Vec<f64>, Error> {
    let heights = affine_heights(curve, ends, slope)?;
    if heights.error_mm > tolerance / 256. {
        return Err(Error::Unsupported);
    }
    Ok(heights.values)
}

fn metric(body: &Chamfer, p: &Placement, tolerance: f64) -> Result<(), Error> {
    let axes = [p.u, p.v, p.normal];
    let mut gram = 0.;
    for i in 0..3 {
        for j in 0..3 {
            let e = dot_error(axes[i], axes[j], if i == j { 1. } else { 0. });
            gram = add_up(gram, mul_up(e, e));
        }
    }
    let r = body.source_thread().spec().major_radius_mm;
    let extent = add_up(
        mul_up(2., mul_up(r, r)),
        mul_up(body.full_length_mm(), body.full_length_mm()),
    );
    let error = mul_up(gram.sqrt().next_up(), extent.sqrt().next_up());
    if !error.is_finite() || error > tolerance / 64. {
        return Err(Error::InvalidPlacement);
    }
    Ok(())
}

pub(super) fn encode<'a>(
    file: &'a mut StepFile,
    body: &'a Chamfer,
    placement: Placement,
    plan: &Plan,
    tolerance: f64,
    interrupt: Option<&'a dyn Fn() -> bool>,
) -> Result<Entity, Error> {
    let tolerance = tolerance.min(1e-7);
    metric(body, &placement, tolerance)?;
    checkpoint(interrupt)?;
    let uv = UvContext::new(file);
    let mut walls = Vec::with_capacity(plan.source.bands.len());
    for band in &plan.source.bands {
        checkpoint(interrupt)?;
        walls.push(helical::surface(
            file,
            band,
            &plan.source.axial,
            &plan.source.heights,
            &placement,
        ));
    }
    let center = file.point(placement.origin);
    let axis = file.axis(center, placement.normal, placement.u);
    let radius = body.entry_radius_mm();
    let angle = plan.cone_angle;
    let cone = file.entity(format_args!(
        "CONICAL_SURFACE('',{axis},{radius:.16E},{angle:.16E})"
    ));
    let shell = Shell {
        file,
        body,
        placement,
        tolerance,
        uv,
        walls,
        cone,
        vertices: HashMap::new(),
        bottom: HashMap::new(),
        cone_rails: HashMap::new(),
        uses: HashMap::new(),
        interrupt,
    };
    shell.build(plan)
}

impl Shell<'_> {
    fn tick(&self) -> Result<(), Error> {
        checkpoint(self.interrupt)?;
        if self.file.next - 1 > crate::step_read::MAX_STEP_ENTITIES
            || self.file.data.len() > crate::step_read::MAX_PLANAR_STEP_BYTES
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
    fn vertex(&mut self, plan: &Plan, start: u32, q: f64) -> Result<Vertex, Error> {
        let key = key(self.body, start, q);
        if let Some(&vertex) = self.vertices.get(&key) {
            return Ok(vertex);
        }
        self.tick()?;
        let point = plan.points[&key];
        let vertex = self
            .file
            .vertex(self.placement.point_f64([point[0], point[1]], point[2]));
        self.vertices.insert(key, vertex);
        Ok(vertex)
    }
    fn bottom(&mut self, plan: &Plan, start: u32, q: f64) -> Result<Vertex, Error> {
        let key = key(self.body, start, q);
        if let Some(&vertex) = self.bottom.get(&key) {
            return Ok(vertex);
        }
        let local = plan.points[&key];
        let vertex = if local[2] == 0. {
            self.vertex(plan, start, q)?
        } else {
            let theta = -(q + f64::from(start) * self.body.source_thread().spec().pitch_mm)
                / self.body.source_thread().helical_rate();
            let (sin, cos) = theta.sin_cos();
            self.file.vertex(self.placement.point_f64(
                [
                    self.body.entry_radius_mm() * cos,
                    self.body.entry_radius_mm() * sin,
                ],
                0.,
            ))
        };
        self.bottom.insert(key, vertex);
        Ok(vertex)
    }
    fn face(
        &mut self,
        loops: &[Vec<(Entity, bool)>],
        surface: Entity,
        same_sense: bool,
    ) -> Result<Entity, Error> {
        self.tick()?;
        let mut bounds = Vec::with_capacity(loops.len());
        for (i, uses) in loops.iter().enumerate() {
            if uses.is_empty() {
                return Err(Error::InvalidGeometry);
            }
            let mut oriented = Vec::with_capacity(uses.len());
            for &(edge, sense) in uses {
                let entry = self.uses.entry(edge.0).or_default();
                entry.0 += 1;
                entry.1 += if sense { 1 } else { -1 };
                if entry.0 > 2 {
                    return Err(Error::InvalidGeometry);
                }
                oriented.push(self.file.oriented_edge(edge, sense));
            }
            bounds.push(self.file.bound(&oriented, i == 0));
        }
        Ok(self.file.face_on(&bounds, surface, same_sense))
    }
    fn oriented(&self, mut edges: Vec<(Entity, bool)>, reverse: bool) -> Vec<(Entity, bool)> {
        if reverse {
            edges.reverse();
            for (_, sense) in &mut edges {
                *sense = !*sense;
            }
        }
        edges
    }
    fn plane(&mut self, z: f64, up: bool) -> Entity {
        let center = self.file.point(self.placement.point_f64([0.; 2], z));
        let normal = self.placement.normal.map(|v| if up { v } else { -v });
        let axis = self.file.axis(center, normal, self.placement.u);
        self.file.entity(format_args!("PLANE('',{axis})"))
    }
    fn polynomial_edge(
        &mut self,
        curve: &helical::Spline,
        vertices: [Vertex; 2],
        points: impl Iterator<Item = Point>,
        pcurves: &[Entity],
        seam: bool,
    ) -> Result<Entity, Error> {
        self.tick()?;
        let geometry = spline(self.file, curve, vertices, points)?;
        Ok(edge(self.file, vertices, geometry, pcurves, seam, true))
    }
    fn group(&mut self, plan: &Plan, group: &Group) -> Result<(Entity, Entity), Error> {
        self.tick()?;
        let first = group.trims.first().expect("staged group");
        let last = group.trims.last().expect("staged group");
        let vertices = [
            self.vertex(plan, first.start, first.phase[0])?,
            self.vertex(plan, last.start, last.phase[1])?,
        ];
        let h = self.body.source_thread().helical_rate().abs();
        let (knots, wall) = cubic_uv(&group.trims, |trim, i| {
            [trim.surface_u_poles[i], trim.height_poles[i] / h]
        });
        let wall = self
            .uv
            .spline(self.file, self.walls[group.wall], 3, &knots, &wall)?;
        let mut pcurves = vec![wall];
        if group.active {
            let k = (self.body.source_thread().spec().major_radius_mm
                - self.body.entry_radius_mm())
                / self.body.entry_length_mm();
            let cosine = k.atan().cos();
            let (knots, uv) = cubic_uv(&group.trims, |trim, i| {
                [trim.angle_poles[i], trim.height_poles[i] / cosine]
            });
            pcurves.push(self.uv.spline(self.file, self.cone, 3, &knots, &uv)?);
        }
        let geometry = cubic(self.file, &self.placement, &group.trims)?;
        let seam = edge(self.file, vertices, geometry, &pcurves, false, true);
        if !group.active {
            return Ok((seam, seam));
        }
        let bottom = [
            self.bottom(plan, first.start, first.phase[0])?,
            self.bottom(plan, last.start, last.phase[1])?,
        ];
        let center = self.file.point(self.placement.origin);
        let axis = self
            .file
            .axis(center, self.placement.normal, self.placement.u);
        let radius = self.body.entry_radius_mm();
        let geometry = self
            .file
            .entity(format_args!("CIRCLE('',{axis},{radius:.16E})"));
        let pitch = self.body.source_thread().spec().pitch_mm;
        let rate = self.body.source_thread().helical_rate();
        let domain =
            [first.phase[0], last.phase[1]].map(|q| -(q + f64::from(first.start) * pitch) / rate);
        let [a, b] = if domain[0] < domain[1] {
            domain
        } else {
            [domain[1], domain[0]]
        };
        let pcurve = self
            .uv
            .line(self.file, self.cone, [a, b], [[a, 0.], [b, 0.]])?;
        let rim = edge(self.file, bottom, geometry, &[pcurve], false, rate < 0.);
        Ok((seam, rim))
    }

    fn cone_rail(&mut self, plan: &Plan, start: u32, q: f64) -> Result<Option<Entity>, Error> {
        let key = key(self.body, start, q);
        if let Some(&value) = self.cone_rails.get(&key) {
            return Ok(value);
        }
        let d = plan.points[&key];
        if d[2] == 0. {
            self.cone_rails.insert(key, None);
            return Ok(None);
        }
        let vertices = [self.bottom(plan, start, q)?, self.vertex(plan, start, q)?];
        let rate = self.body.source_thread().helical_rate();
        let h = rate.abs();
        let r0 = self.body.entry_radius_mm();
        let k =
            (self.body.source_thread().spec().major_radius_mm - r0) / self.body.entry_length_mm();
        let theta = if key == (0, 0_f64.to_bits()) {
            0.
        } else {
            -(q + f64::from(start) * self.body.source_thread().spec().pitch_mm) / rate
        };
        let rotation = [theta.cos(), theta.sin()];
        let domain = [0., d[2] / h];
        let radii = [r0, r0 + k * d[2]];
        let hand = rate.signum();
        let error = self.tolerance / 128.;
        let curve = helical::Spline::new(domain[0], domain[1], radii, hand, error)?;
        let heights = certified_heights(&curve, [0., d[2]], h, self.tolerance)?;
        let points = curve
            .poles
            .iter()
            .zip(heights)
            .map(|(&r, z)| self.placement.point_f64(helical::product(rotation, r), z))
            .collect::<Vec<_>>();
        let cosine = k.atan().cos();
        let last = [theta + d[2] / rate, d[2] / cosine];
        let mut pcurves =
            vec![
                self.uv
                    .line(self.file, self.cone, [0., d[2] / h], [[theta, 0.], last])?,
            ];
        let seam = key == (0, 0_f64.to_bits());
        if seam {
            let shift = -rate.signum() * std::f64::consts::TAU;
            let ends = [[theta + shift, 0.], [last[0] + shift, last[1]]];
            let pcurve = self.uv.line(self.file, self.cone, domain, ends)?;
            pcurves.push(pcurve);
        }
        let edge = self.polynomial_edge(&curve, vertices, points.into_iter(), &pcurves, seam)?;
        self.cone_rails.insert(key, Some(edge));
        Ok(Some(edge))
    }

    fn build(mut self, plan: &Plan) -> Result<Entity, Error> {
        let count = self.walls.len();
        let n = self.body.source_thread().spec().starts as usize;
        let mut wall_edges = vec![Vec::new(); count];
        let mut cone_edges = vec![Vec::new(); 2 * n];
        let mut cone_bottom = vec![Vec::new(); 2 * n];
        let mut cone_ends = vec![None::<[(u32, f64); 2]>; 2 * n];
        let mut entry = Vec::new();
        for group in &plan.groups {
            let (seam, bottom) = self.group(plan, group)?;
            wall_edges[group.wall].push((group.trims[0].surface_u_poles[0], seam));
            entry.push((bottom, true));
            if group.active {
                cone_edges[group.cone].push((seam, false));
                cone_bottom[group.cone].push((bottom, true));
                let first = group.trims.first().unwrap();
                let last = group.trims.last().unwrap();
                let a = (first.start, first.phase[0]);
                let b = (last.start, last.phase[1]);
                if let Some(ends) = &mut cone_ends[group.cone] {
                    ends[1] = b;
                } else {
                    cone_ends[group.cone] = Some([a, b]);
                }
            }
        }
        let f = self.body.material_length_mm();
        let rate = self.body.source_thread().helical_rate();
        let spin = [(f / rate).cos(), (f / rate).sin()];
        let top = plan
            .source
            .bands
            .iter()
            .map(|band| {
                self.file.vertex(
                    self.placement
                        .point_f64(helical::product(band.poles[0], spin), f),
                )
            })
            .collect::<Vec<_>>();
        let mut rails = Vec::with_capacity(count);
        let breaks = self.body.source_thread().profile_breaks();
        let local = [breaks[1], breaks[2], breaks[3], breaks[4]];
        for i in 0..count {
            self.tick()?;
            let start = (i / 4) as u32;
            let q = local[i % 4];
            let bottom = self.vertex(plan, start, q)?;
            let z = plan.points[&key(self.body, start, q)][2];
            let radius = self.body.source_thread().radius_at_phase(q);
            let rotation = plan.source.bands[i].poles[0].map(|x| x / radius);
            let [a, b] = [z / rate.abs(), f / rate.abs()];
            let error = self.tolerance / 128.;
            let hand = rate.signum();
            let spline = helical::Spline::new(a, b, [radius; 2], hand, error)?;
            let heights = certified_heights(&spline, [z, f], rate.abs(), self.tolerance)?;
            let points = spline
                .poles
                .iter()
                .zip(heights)
                .map(|(&r, dz)| self.placement.point_f64(helical::product(rotation, r), dz))
                .collect::<Vec<_>>();
            let previous = (i + count - 1) % count;
            let old = &plan.source.bands[previous];
            let u = [plan.source.bands[i].knots[0], *old.knots.last().unwrap()];
            let pcurves = [i, previous]
                .into_iter()
                .zip(u)
                .map(|(j, u)| {
                    self.uv.line(
                        self.file,
                        self.walls[j],
                        [z / rate.abs(), f / rate.abs()],
                        [[u, z / rate.abs()], [u, f / rate.abs()]],
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let vertices = [bottom, top[i]];
            let controls = points.into_iter();
            let edge = self.polynomial_edge(&spline, vertices, controls, &pcurves, false)?;
            rails.push(edge);
        }
        let mut top_edges = Vec::with_capacity(count);
        for (i, band) in plan.source.bands.iter().enumerate() {
            let points = band
                .poles
                .iter()
                .map(|&r| self.placement.point_f64(helical::product(r, spin), f))
                .collect::<Vec<_>>();
            let vertices = [top[i], top[(i + 1) % count]];
            let controls = points.into_iter();
            let edge = self.polynomial_edge(band, vertices, controls, &[], false)?;
            top_edges.push(edge);
        }
        let left = self.body.source_thread().spec().left_hand;
        let mut faces = Vec::new();
        for i in 0..count {
            wall_edges[i].sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut uses = wall_edges[i]
                .iter()
                .map(|(_, edge)| (*edge, true))
                .collect::<Vec<_>>();
            uses.extend([
                (rails[(i + 1) % count], true),
                (top_edges[i], false),
                (rails[i], false),
            ]);
            let uses = self.oriented(uses, !left);
            faces.push(self.face(&[uses], self.walls[i], left)?);
        }
        let entry = self.oriented(entry, left);
        let plane = self.plane(0., false);
        faces.push(self.face(&[entry], plane, true)?);
        let mut cap =
            vec![self.oriented(top_edges.iter().map(|&edge| (edge, true)).collect(), !left)];
        let neck = if let Some(relief) = self.body.terminal_relief() {
            let stations = [f, self.body.full_length_mm()];
            let centers = stations.map(|z| self.placement.point_f64([0.; 2], z));
            let normal = self.placement.normal;
            let u = self.placement.u;
            let radius = relief.neck_radius_mm();
            let neck = CylindricalTrim::new(self.file, centers, normal, u, radius)?;
            cap.push(vec![(neck.rims[0], false)]);
            Some(neck)
        } else {
            None
        };
        let plane = self.plane(f, true);
        faces.push(self.face(&cap, plane, true)?);
        for i in 0..2 * n {
            let [a, b] = cone_ends[i].ok_or(Error::InvalidGeometry)?;
            let mut edges = std::mem::take(&mut cone_bottom[i]);
            if let Some(edge) = self.cone_rail(plan, b.0, b.1)? {
                edges.push((edge, true));
            }
            edges.extend(cone_edges[i].iter().rev().map(|&(edge, _)| (edge, false)));
            if let Some(edge) = self.cone_rail(plan, a.0, a.1)? {
                edges.push((edge, false));
            }
            let edges = self.oriented(edges, !left);
            faces.push(self.face(&[edges], self.cone, true)?);
        }
        if let Some(neck) = neck {
            let edges = vec![
                (neck.rims[0], true),
                (neck.seam, true),
                (neck.rims[1], false),
                (neck.seam, false),
            ];
            faces.push(self.face(&[edges], neck.surface, true)?);
            let plane = self.plane(self.body.full_length_mm(), true);
            faces.push(self.face(&[vec![(neck.rims[1], true)]], plane, true)?);
        }
        self.tick()?;
        if self
            .uses
            .values()
            .any(|&(count, sum)| count != 2 || sum != 0)
        {
            return Err(Error::InvalidGeometry);
        }
        Ok(self.file.solid(&faces))
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
    fn raw<'a>(file: &'a mut StepFile, body: &'a Chamfer) -> Shell<'a> {
        let uv = UvContext::new(file);
        Shell {
            file,
            body,
            placement: Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap(),
            tolerance: 1e-7,
            uv,
            walls: vec![],
            cone: Entity(0),
            vertices: HashMap::new(),
            bottom: HashMap::new(),
            cone_rails: HashMap::new(),
            uses: HashMap::new(),
            interrupt: None,
        }
    }
    #[test]
    fn affine_parameter_height_receipts_reject_wrong_domains_and_unrepresentable_precision() {
        let curve = helical::Spline::new(0., 1., [1.; 2], 1., 1e-8).unwrap();
        assert!(certified_heights(&curve, [0., 1.], 1., 1e-7).is_ok());
        assert!(matches!(
            certified_heights(&curve, [0., 1.001], 1., 1e-7),
            Err(Error::Unsupported)
        ));
        assert!(
            certified_heights(
                &helical::Spline {
                    knots: vec![],
                    poles: vec![]
                },
                [0., 1.],
                1.,
                1e-7
            )
            .is_err()
        );
    }
    #[test]
    fn incomplete_or_overused_raw_wires_and_exhausted_file_budget_cannot_publish_a_solid() {
        let body = body();
        let mut file = StepFile::new();
        let a = file.vertex([0.; 3]);
        let b = file.vertex([1., 0., 0.]);
        let edge = line_edge(&mut file, a, b).unwrap();
        let mut shell = raw(&mut file, &body);
        assert!(matches!(
            shell.face(&[vec![]], Entity(0), true),
            Err(Error::InvalidGeometry)
        ));
        assert!(matches!(
            shell.face(
                &[vec![(edge, true), (edge, false), (edge, true)]],
                Entity(0),
                true
            ),
            Err(Error::InvalidGeometry)
        ));
        shell.file.next = crate::step_read::MAX_STEP_ENTITIES + 2;
        assert_eq!(shell.tick(), Err(Error::Limit));
        let mut file = StepFile::new();
        let a = file.vertex([0.; 3]);
        let b = file.vertex([1., 0., 0.]);
        let edge = line_edge(&mut file, a, b).unwrap();
        let mut shell = raw(&mut file, &body);
        shell.face(&[vec![(edge, true)]], Entity(0), true).unwrap();
        let placement = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
        let plan = prepare(&body, &placement, 1e-7, None).unwrap();
        // Add the proper full source surfaces before exercising the final
        // shared-use guard on this deliberately incomplete raw wire.
        shell.walls = plan
            .source
            .bands
            .iter()
            .map(|b| {
                helical::surface(
                    shell.file,
                    b,
                    &plan.source.axial,
                    &plan.source.heights,
                    &shell.placement,
                )
            })
            .collect();
        let point = shell.file.point([0.; 3]);
        let axis = shell.file.axis(point, [0., 0., 1.], [1., 0., 0.]);
        shell.cone = shell.file.entity(format_args!(
            "CONICAL_SURFACE('',{axis},2.6,0.7853981633974483)"
        ));
        assert!(matches!(shell.build(&plan), Err(Error::InvalidGeometry)));
        assert!(!file.data.contains("MANIFOLD_SOLID_BREP"));
    }
    #[test]
    fn cancellation_inside_a_real_conical_rail_reaches_the_transaction_boundary() {
        let body = body();
        let placement = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
        let plan = prepare(&body, &placement, 1e-7, None).unwrap();
        let mut file = StepFile::new();
        let mut shell = raw(&mut file, &body);
        shell.vertex(&plan, 0, 0.).unwrap();
        shell.bottom(&plan, 0, 0.).unwrap();
        shell.interrupt = Some(&|| true);
        assert!(matches!(
            shell.cone_rail(&plan, 0, 0.),
            Err(Error::Cancelled)
        ));
    }
}
