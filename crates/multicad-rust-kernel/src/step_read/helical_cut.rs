//! Independently decode a circular helical groove from its complete STEP shell.
use super::*;
use crate::{
    helical_tube_cut::{BoundaryEdge, Cut, CylinderTrim, Spec},
    step::helical_trim::{CavityCapPcurve, CavitySurface, Pcurve, Rail, TopCavity, UvSpline},
};
use helical::{face, require, ring, step_error};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

#[path = "helical_cut/decode.rs"]
pub(in crate::step_read) mod decode;
use decode::{Affine, Edge};

const BOTTOM: usize = 0;
const TOP_LO: usize = 1;
const TOP_HI: usize = 2;
const SEAM: usize = 3;
const POSITIVE: usize = 4;
const NEGATIVE: usize = 5;
const FLOOR_RIM: usize = 6;
const FLOOR_CYLINDER: usize = 7;
const TOP_CAVITY: usize = 8;

type EdgeUse = (u32, bool);

pub(in crate::step_read) fn read_member(
    shell: &Shell<'_, '_>,
    face_ids: &[u32],
) -> Result<Option<(Readback, ImportSolid)>> {
    let &[bottom, top, cylinder, cavity, floor] = face_ids else {
        return Ok(None);
    };
    let bottom = face(shell, bottom)?;
    let top = face(shell, top)?;
    let cylinder = face(shell, cylinder)?;
    let cavity = face(shell, cavity)?;
    let floor = face(shell, floor)?;
    if shell.simple(cylinder.0)?.name != "CYLINDRICAL_SURFACE"
        || shell.simple(cavity.0)?.name != "B_SPLINE_SURFACE_WITH_KNOTS"
    {
        return Ok(None);
    }
    let surfaces = [bottom.0, top.0, cylinder.0, cavity.0, floor.0];
    require(surfaces.into_iter().collect::<BTreeSet<_>>().len() == 5)?;
    require([bottom.1, top.1, cylinder.1, cavity.1, floor.1] == [true, true, true, false, true])?;
    let wire = |bounds: &[u32]| {
        let [bound] = bounds else {
            return Err(ReadError::Invalid);
        };
        ring(shell, *bound, true)
    };
    let wires = [
        wire(&bottom.2)?,
        wire(&top.2)?,
        wire(&cylinder.2)?,
        wire(&cavity.2)?,
        wire(&floor.2)?,
    ];
    let edges = incidence(shell, &wires)?;
    let result = verify(shell, surfaces, edges)?;
    Ok(Some(result))
}

fn incidence(shell: &Shell<'_, '_>, wires: &[Vec<EdgeUse>; 5]) -> Result<[Edge; 9]> {
    let [bottom, top, cylinder, cavity, floor] = wires.each_ref().map(Vec::as_slice);
    let [(bottom, false)] = bottom else {
        return Err(ReadError::Invalid);
    };
    let [(top_lo, true), (top_hi, true), (top_cavity, false)] = top else {
        return Err(ReadError::Invalid);
    };
    let [(floor_rim, true), (floor_cylinder, true)] = floor else {
        return Err(ReadError::Invalid);
    };
    let [
        _,
        (seam, true),
        _,
        (positive, false),
        _,
        (negative, true),
        _,
        _,
    ] = cylinder
    else {
        return Err(ReadError::Invalid);
    };
    require(
        cylinder
            == [
                (*bottom, true),
                (*seam, true),
                (*top_lo, false),
                (*positive, false),
                (*floor_cylinder, false),
                (*negative, true),
                (*top_hi, false),
                (*seam, false),
            ],
    )?;
    require(
        cavity
            == [
                (*positive, true),
                (*top_cavity, true),
                (*negative, false),
                (*floor_rim, false),
            ],
    )?;
    let ids = [
        *bottom,
        *top_lo,
        *top_hi,
        *seam,
        *positive,
        *negative,
        *floor_rim,
        *floor_cylinder,
        *top_cavity,
    ];
    require(ids.into_iter().collect::<BTreeSet<_>>().len() == ids.len())?;
    let edges = ids
        .into_iter()
        .map(|id| decode::edge(shell, id))
        .collect::<Result<Vec<_>>>()?;
    let edges: [Edge; 9] = edges.try_into().map_err(|_| ReadError::Invalid)?;
    let [bottom_vertex, top_seam] = edges[SEAM].vertices;
    let [p0, p1] = edges[POSITIVE].vertices;
    let [n0, n1] = edges[NEGATIVE].vertices;
    require(
        [bottom_vertex, top_seam, p0, p1, n0, n1]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .len()
            == 6,
    )?;
    require(
        edges[BOTTOM].vertices == [bottom_vertex; 2]
            && edges[TOP_LO].vertices == [p1, top_seam]
            && edges[TOP_HI].vertices == [top_seam, n1]
            && edges[FLOOR_RIM].vertices == [p0, n0]
            && edges[FLOOR_CYLINDER].vertices == [n0, p0]
            && edges[TOP_CAVITY].vertices == [p1, n1],
    )?;
    Ok(edges)
}

#[derive(Clone, Copy)]
struct Frame {
    origin: Point,
    normal: Point,
    u: Point,
    v: Point,
}

impl From<Plane> for Frame {
    fn from(plane: Plane) -> Self {
        Self {
            origin: plane.origin,
            normal: plane.normal,
            u: plane.u,
            v: plane.v,
        }
    }
}

fn local(frame: Frame, point: Point) -> Point {
    let offset = sub(point, frame.origin);
    [frame.u, frame.v, frame.normal].map(|axis| dot(axis, offset))
}

fn world(frame: Frame, [x, y, z]: Point) -> Point {
    std::array::from_fn(|i| frame.origin[i] + frame.u[i] * x + frame.v[i] * y + frame.normal[i] * z)
}

fn local_direction(frame: Frame, direction: Point) -> Point {
    [frame.u, frame.v, frame.normal].map(|axis| dot(axis, direction))
}

fn near(a: f64, b: f64, tolerance: f64) -> Result<()> {
    require((a - b).abs() <= tolerance)
}

fn near_point(a: Point, b: Point, tolerance: f64) -> Result<()> {
    require(length(sub(a, b)) <= tolerance)
}

fn vertex(shell: &Shell<'_, '_>, id: u32, frame: Frame) -> Result<Point> {
    let [Value::String(_), point] = shell.args(id, "VERTEX_POINT")? else {
        return Err(ReadError::Invalid);
    };
    Ok(local(frame, shell.point(reference(point)?)?))
}

struct RailSource {
    spline: u32,
    cylinder: Affine,
    cavity: Affine,
}

fn read_rail(shell: &Shell<'_, '_>, edge: Edge, surfaces: [u32; 2]) -> Result<RailSource> {
    let (spline, [cylinder, cavity]) =
        decode::wrapper(shell, edge, "SURFACE_CURVE", "PCURVE_S1", surfaces)?;
    let cylinder = Affine::decode(shell, cylinder)?;
    let cavity = Affine::decode(shell, cavity)?;
    require(cylinder.domain == cavity.domain)?;
    Ok(RailSource {
        spline,
        cylinder,
        cavity,
    })
}

fn verify(
    shell: &Shell<'_, '_>,
    [bottom, top, cylinder, cavity, floor]: [u32; 5],
    edges: [Edge; 9],
) -> Result<(Readback, ImportSolid)> {
    let [Value::String(_), placement, Value::Number(radius)] =
        shell.args(cylinder, "CYLINDRICAL_SURFACE")?
    else {
        return Err(ReadError::Invalid);
    };
    let radius = *radius;
    let frame = Frame::from(shell.placement(reference(placement)?)?);
    let bottom_plane = decode::plane(shell, bottom)?;
    let top_plane = decode::plane(shell, top)?;
    let floor_plane = decode::plane(shell, floor)?;
    let height = local(frame, top_plane.origin)[2];
    let guard = 256. * f64::EPSILON * (length(frame.origin) + radius.abs() + height.abs() + 1.);
    let tolerance = shell.tolerance;
    if guard >= tolerance / 16. {
        return Err(ReadError::Limit);
    }
    let model_tolerance = tolerance * 0.75;
    let near_geometry = tolerance * 0.125;
    let angle_tolerance = near_geometry / (1. + radius.abs().max(height.abs()));
    near_point(local(frame, bottom_plane.origin), [0.; 3], near_geometry)?;
    near_point(
        local(frame, top_plane.origin),
        [0., 0., height],
        near_geometry,
    )?;
    near_point(
        local_direction(frame, bottom_plane.normal),
        [0., 0., -1.],
        angle_tolerance,
    )?;
    near_point(
        local_direction(frame, top_plane.normal),
        [0., 0., 1.],
        angle_tolerance,
    )?;

    let [positive, negative] =
        [POSITIVE, NEGATIVE].map(|edge| read_rail(shell, edges[edge], [cylinder, cavity]));
    let [positive, negative] = [positive?, negative?];
    let phase_lifted = (positive.cylinder.domain[0] + negative.cylinder.domain[0]) * 0.5;
    let [[u0, z0], [u1, z1]] = positive.cylinder.endpoints;
    let rise = (z1 - z0) / (u1 - u0);
    require(rise.is_finite() && rise > 0.)?;
    let (floor_circle, [floor_uv]) = decode::wrapper(
        shell,
        edges[FLOOR_RIM],
        "SURFACE_CURVE",
        "CURVE_3D",
        [cavity],
    )?;
    let (floor_circle, tube) = decode::circle(shell, floor_circle)?;
    let [x, y, start] = local(frame, floor_circle.origin);
    let phase = y.atan2(x);
    let lift = ((phase_lifted - phase) / TAU).round() * TAU;
    near(phase + lift, phase_lifted, angle_tolerance)?;
    let vertical = radius / radius.hypot(rise);
    let minimum_path = height - start + vertical * tube;
    let path = minimum_path + (tolerance * 32.).max(guard * 32.);
    let spec = Spec {
        stock_radius_mm: radius,
        stock_height_mm: height,
        tube_radius_mm: tube,
        pitch_mm: rise * TAU,
        path_height_mm: path,
        start_height_mm: start,
        start_angle_rad: phase,
    };
    let cut = Cut::new(spec, tolerance).map_err(cut_error)?;
    let budget = Verification {
        frame,
        cut,
        lift,
        geometry: near_geometry,
        angle: angle_tolerance,
        model: model_tolerance,
    };
    let [negative_trim, positive_trim] = cut.cylinder_trims();
    let [profile_lo, profile_hi] = cut.edge_domain(BoundaryEdge::TopCavity);
    budget.rail(shell, edges[POSITIVE], positive, positive_trim, profile_lo)?;
    budget.rail(shell, edges[NEGATIVE], negative, negative_trim, profile_hi)?;
    let surface_bound = budget.surface(shell, cavity)?;
    budget.floor(shell, edges[FLOOR_RIM], floor_plane, floor_circle, floor_uv)?;
    let seam = budget.seam(shell, edges[SEAM], cylinder)?;
    let arcs = budget.stock_rims(
        shell,
        [edges[BOTTOM], edges[TOP_LO], edges[TOP_HI]],
        cylinder,
        seam,
    )?;
    budget.floor_cylinder(shell, edges[FLOOR_CYLINDER], cylinder)?;
    budget.top_cavity(
        shell,
        edges[TOP_CAVITY],
        [cavity, top],
        top_plane,
        surface_bound,
    )?;

    let mut bounds_mm = [frame.origin; 2];
    let [min, max] = &mut bounds_mm;
    for (i, (min, max)) in min.iter_mut().zip(max).enumerate() {
        let direction = [frame.u[i], frame.v[i], frame.normal[i]];
        *min -= cut
            .support_upper_mm(direction.map(|x| -x))
            .map_err(cut_error)?;
        *max += cut.support_upper_mm(direction).map_err(cut_error)?;
    }
    let volume_mm3 = cut.volume_mm3();
    require(volume_mm3.is_finite() && volume_mm3 > 0.)?;
    Ok((
        Readback {
            bounds_mm,
            volume_mm3,
            face_count: 5,
            edges_mm: vec![],
            circles: vec![CircleEdge {
                center_mm: frame.origin,
                normal: frame.normal,
                radius_mm: radius,
            }],
            semicircles: vec![],
            arcs,
            ellipses: vec![],
            splines: vec![],
            solid_count: 1,
        },
        ImportSolid {
            origin_mm: frame.origin,
            u: frame.u,
            v: frame.v,
            height_mm: height,
            profile: ImportProfile::HelicalTubeCut {
                stock_radius_mm: radius,
                radius_mm: tube,
                pitch_mm: spec.pitch_mm,
                path_height_mm: path,
                start_height_mm: start,
                start_angle_rad: phase,
            },
        },
    ))
}

#[derive(Clone, Copy)]
struct Verification {
    frame: Frame,
    cut: Cut,
    lift: f64,
    geometry: f64,
    angle: f64,
    model: f64,
}

fn cut_error(cause: crate::helical_tube_cut::Error) -> ReadError {
    match cause {
        crate::helical_tube_cut::Error::Limit => ReadError::Limit,
        _ => ReadError::Invalid,
    }
}

fn align_domain(knots: &mut [f64], expected: [f64; 2], tolerance: f64) -> Result<f64> {
    let [first, .., last] = knots else {
        return Err(ReadError::Invalid);
    };
    near(*first, expected[0], tolerance)?;
    near(*last, expected[1], tolerance)?;
    let drift = (*first - expected[0])
        .abs()
        .max((*last - expected[1]).abs());
    *first = expected[0];
    *last = expected[1];
    require(knots.windows(2).all(|pair| pair[0] < pair[1]))?;
    Ok(drift)
}

impl Verification {
    fn sweep_speed(self) -> f64 {
        let spec = self.cut.spec();
        (spec.stock_radius_mm + spec.tube_radius_mm).hypot(spec.pitch_mm / TAU)
    }

    fn top_speed(self) -> f64 {
        let spec = self.cut.spec();
        let rise = spec.pitch_mm / TAU;
        let vertical = spec.stock_radius_mm / spec.stock_radius_mm.hypot(rise);
        spec.tube_radius_mm + self.sweep_speed() * vertical * spec.tube_radius_mm / rise
    }

    fn vertices(self, shell: &Shell<'_, '_>, edge: Edge, expected: [Point; 2]) -> Result<()> {
        for (id, point) in edge.vertices.into_iter().zip(expected) {
            near_point(vertex(shell, id, self.frame)?, point, self.geometry)?;
        }
        Ok(())
    }

    fn boundary(self, shell: &Shell<'_, '_>, edge: Edge, kind: BoundaryEdge) -> Result<()> {
        self.vertices(
            shell,
            edge,
            self.cut
                .edge_domain(kind)
                .map(|t| self.cut.edge_point(kind, t)),
        )
    }

    fn rail(
        self,
        shell: &Shell<'_, '_>,
        edge: Edge,
        source: RailSource,
        trim: CylinderTrim,
        phi: f64,
    ) -> Result<()> {
        let domain = trim.azimuth_rad.map(|angle| angle + self.lift);
        for (i, expected) in domain.into_iter().enumerate() {
            near(source.cylinder.domain[i], expected, self.angle)?;
            near(source.cylinder.endpoints[i][0], expected, self.angle)?;
            near(
                source.cylinder.endpoints[i][1],
                trim.height_mm[i],
                self.geometry,
            )?;
            near(source.cavity.endpoints[i][0], phi, self.angle)?;
            near(
                source.cavity.endpoints[i][1],
                expected - domain[0],
                self.angle,
            )?;
        }
        self.certify_rail(shell, edge, source.spline, source.cylinder)
    }

    fn certify_rail(
        self,
        shell: &Shell<'_, '_>,
        edge: Edge,
        geometry: u32,
        uv: Affine,
    ) -> Result<()> {
        let radius = self.cut.spec().stock_radius_mm;
        let [[a, za], [b, zb]] = uv.endpoints;
        near(a, uv.domain[0], self.angle)?;
        near(b, uv.domain[1], self.angle)?;
        let spline = decode::curve(shell, geometry)?.into_spline(self.frame);
        let rail = Rail::new(
            radius,
            CylinderTrim {
                azimuth_rad: uv.domain,
                height_mm: [za, zb],
            },
            self.model,
        )
        .map_err(step_error)?;
        let [t0, t1] = uv.domain;
        let velocity = [(b - a) / (t1 - t0), (zb - za) / (t1 - t0)];
        let pcurve = Pcurve {
            origin: [
                (-velocity[0]).mul_add(t0, a),
                (-velocity[1]).mul_add(t0, za),
            ],
            velocity,
            domain: uv.domain,
        };
        rail.certify(pcurve, &spline).map_err(step_error)?;
        self.vertices(
            shell,
            edge,
            uv.endpoints.map(|[u, z]| {
                let (sin, cos) = u.sin_cos();
                [radius * cos, radius * sin, z]
            }),
        )
    }

    fn surface(self, shell: &Shell<'_, '_>, cavity: u32) -> Result<f64> {
        let mut decoded = decode::surface(shell, cavity, self.frame)?;
        let support = CavitySurface::new(self.cut, self.model).map_err(step_error)?;
        let [profile, sweep] = &mut decoded.axes;
        let [profile_domain, sweep_domain] = support.domains();
        let profile_drift = align_domain(&mut profile.knots, profile_domain, self.angle)?;
        let sweep_drift = align_domain(&mut sweep.knots, sweep_domain, self.angle)?;
        let drift =
            profile_drift * self.cut.spec().tube_radius_mm + sweep_drift * self.sweep_speed();
        let bound = support.certify(&decoded).map_err(step_error)? + drift;
        require(bound < self.model)?;
        Ok(bound)
    }

    fn floor(
        self,
        shell: &Shell<'_, '_>,
        edge: Edge,
        plane: Frame,
        circle: Frame,
        uv: u32,
    ) -> Result<()> {
        let (origin, u, v) = self.cut.floor_frame();
        for placement in [plane, circle] {
            near_point(local(self.frame, placement.origin), origin, self.geometry)?;
            near_point(local_direction(self.frame, placement.u), u, self.angle)?;
            near_point(local_direction(self.frame, placement.v), v, self.angle)?;
        }
        let uv = Affine::decode(shell, uv)?;
        let domain = self.cut.edge_domain(BoundaryEdge::FloorRim);
        for (i, expected) in domain.into_iter().enumerate() {
            near(uv.domain[i], expected, self.angle)?;
            near(uv.endpoints[i][0], expected, self.angle)?;
            near(uv.endpoints[i][1], 0., self.angle)?;
        }
        self.boundary(shell, edge, BoundaryEdge::FloorRim)
    }

    fn seam(self, shell: &Shell<'_, '_>, edge: Edge, cylinder: u32) -> Result<[f64; 2]> {
        let (geometry, [right, left]) =
            decode::wrapper(shell, edge, "SEAM_CURVE", "PCURVE_S1", [cylinder; 2])?;
        let right = Affine::decode(shell, right)?;
        let left = Affine::decode(shell, left)?;
        require(right.domain == left.domain)?;
        let spec = self.cut.spec();
        let rise = spec.pitch_mm / TAU;
        let start = spec.start_angle_rad + self.lift + PI - spec.start_height_mm / rise;
        let expected = [start, start + spec.stock_height_mm / rise];
        require((-self.angle..=TAU + self.angle).contains(&right.domain[0]))?;
        for (i, angle) in expected.into_iter().enumerate() {
            near(right.domain[i], angle, self.angle)?;
            near(right.endpoints[i][0], angle, self.angle)?;
            near(left.endpoints[i][0], angle - TAU, self.angle)?;
            near(
                right.endpoints[i][1],
                spec.stock_height_mm * i as f64,
                self.geometry,
            )?;
            near(left.endpoints[i][1], right.endpoints[i][1], self.geometry)?;
        }
        self.certify_rail(shell, edge, geometry, right)?;
        Ok(expected)
    }

    fn stock_rims(
        self,
        shell: &Shell<'_, '_>,
        [bottom, top_lo, top_hi]: [Edge; 3],
        cylinder: u32,
        seam: [f64; 2],
    ) -> Result<Vec<ArcEdge>> {
        let spec = self.cut.spec();
        let [lo, hi] = self
            .cut
            .edge_domain(BoundaryEdge::TopRim)
            .map(|value| value + self.lift);
        let mut arcs = Vec::with_capacity(2);
        for (edge, domain, height, closed) in [
            (bottom, [seam[0] - TAU, seam[0]], 0., true),
            (top_lo, [lo, seam[1]], spec.stock_height_mm, false),
            (
                top_hi,
                [seam[1] - TAU, hi - TAU],
                spec.stock_height_mm,
                false,
            ),
        ] {
            let (circle, [uv]) =
                decode::wrapper(shell, edge, "SURFACE_CURVE", "CURVE_3D", [cylinder])?;
            let (circle, radius) = decode::circle(shell, circle)?;
            let uv = Affine::decode(shell, uv)?;
            let span = domain[1] - domain[0];
            require(span > 0. && span <= TAU + self.angle)?;
            near(uv.domain[0], 0., self.angle)?;
            near(uv.domain[1], span, self.angle)?;
            near(radius, spec.stock_radius_mm, self.geometry)?;
            near_point(
                local(self.frame, circle.origin),
                [0., 0., height],
                self.geometry,
            )?;
            near_point(
                local_direction(self.frame, circle.normal),
                [0., 0., 1.],
                self.angle,
            )?;
            let radial = |angle: f64| [angle.cos(), angle.sin(), 0.];
            near_point(
                local_direction(self.frame, circle.u),
                radial(domain[0]),
                self.angle,
            )?;
            for (i, angle) in domain.into_iter().enumerate() {
                near(uv.endpoints[i][0], angle, self.angle)?;
                near(uv.endpoints[i][1], height, self.geometry)?;
            }
            if !closed {
                arcs.push(ArcEdge {
                    center_mm: circle.origin,
                    normal: circle.normal,
                    radius_mm: radius,
                    radial_start: circle.u,
                    radial_end: world(
                        Frame {
                            origin: [0.; 3],
                            ..self.frame
                        },
                        radial(domain[1]),
                    ),
                    sweep_rad: span,
                });
            }
        }
        Ok(arcs)
    }

    fn floor_cylinder(self, shell: &Shell<'_, '_>, edge: Edge, cylinder: u32) -> Result<()> {
        let (ellipse, [uv]) =
            decode::wrapper(shell, edge, "SURFACE_CURVE", "CURVE_3D", [cylinder])?;
        let [
            Value::String(_),
            placement,
            Value::Number(major),
            Value::Number(minor),
        ] = shell.args(ellipse, "ELLIPSE")?
        else {
            return Err(ReadError::Invalid);
        };
        let frame = Frame::from(shell.placement(reference(placement)?)?);
        let spec = self.cut.spec();
        let (_, u, v) = self.cut.floor_frame();
        near_point(
            local(self.frame, frame.origin),
            [0., 0., spec.start_height_mm],
            self.geometry,
        )?;
        near_point(local_direction(self.frame, frame.u), v, self.angle)?;
        near_point(
            local_direction(self.frame, frame.v),
            u.map(|x| -x),
            self.angle,
        )?;
        near(
            *major,
            self.cut
                .edge_second_derivative_bound(BoundaryEdge::FloorCylinder),
            self.geometry,
        )?;
        near(*minor, spec.stock_radius_mm, self.geometry)?;
        self.boundary(shell, edge, BoundaryEdge::FloorCylinder)?;
        self.floor_cylinder_uv(shell, uv)
    }

    fn floor_cylinder_uv(self, shell: &Shell<'_, '_>, uv: u32) -> Result<()> {
        let decode::Curve { mut axis, points } = decode::curve::<2>(shell, uv)?;
        let domain = self.cut.edge_domain(BoundaryEdge::FloorCylinder);
        let chart_drift = align_domain(
            &mut axis.knots,
            domain.map(|angle| angle - FRAC_PI_2),
            self.angle,
        )?;
        const DEGREE: usize = 11;
        require(axis.degree == DEGREE)?;
        require(
            axis.multiplicities
                .iter()
                .enumerate()
                .all(|(i, &multiplicity)| {
                    multiplicity == DEGREE + usize::from(i == 0 || i + 1 == axis.knots.len())
                }),
        )?;
        require(points.len() == DEGREE * (axis.knots.len() - 1) + 1)?;
        let spec = self.cut.spec();
        let radius = spec.stock_radius_mm;
        let amplitude = radius * radius / (spec.pitch_mm / TAU);
        for (span, controls) in axis
            .knots
            .windows(2)
            .zip(points.windows(DEGREE + 1).step_by(DEGREE))
        {
            let [a, b] = [span[0] + FRAC_PI_2, span[1] + FRAC_PI_2];
            let width = b - a;
            let mut discrepancy: f64 = 0.;
            for (index, &[u, z]) in controls.iter().enumerate() {
                let angle = a + width * index as f64 / DEGREE as f64;
                let expected_u = spec.start_angle_rad + self.lift + angle;
                let sin = if index <= DEGREE / 2 {
                    sine_hermite_pole(a, width, index)
                } else {
                    sine_hermite_pole(b, -width, DEGREE - index)
                };
                let expected_z = (-amplitude).mul_add(sin, spec.start_height_mm);
                discrepancy = discrepancy.max((radius * (u - expected_u)).hypot(z - expected_z));
            }
            let remainder = amplitude * width.powi(12) / (479_001_600. * 4096.);
            require(discrepancy + remainder + radius.hypot(amplitude) * chart_drift < self.model)?;
        }
        Ok(())
    }

    fn top_cavity(
        self,
        shell: &Shell<'_, '_>,
        edge: Edge,
        surfaces: [u32; 2],
        plane: Frame,
        surface_bound: f64,
    ) -> Result<()> {
        let (spline, [cavity, cap]) =
            decode::wrapper(shell, edge, "SURFACE_CURVE", "CURVE_3D", surfaces)?;
        let mut spline = decode::curve(shell, spline)?.into_spline(self.frame);
        let domain = self.cut.edge_domain(BoundaryEdge::TopCavity);
        let cap_drift = align_domain(&mut spline.knots, domain, self.angle)?;
        let support = TopCavity::new(self.cut, self.model).map_err(step_error)?;
        let cap_bound =
            support.certify(&spline).map_err(step_error)? + cap_drift * self.top_speed();
        let decode::Curve { axis, points } = decode::curve(shell, cap)?;
        let mut projected = crate::step::helical_trim::Spline {
            degree: axis.degree,
            knots: axis.knots,
            multiplicities: axis.multiplicities,
            poles_mm: points
                .into_iter()
                .map(|[u, v]| local(self.frame, world(plane, [u, v, 0.])))
                .collect(),
        };
        let planar_drift = align_domain(&mut projected.knots, domain, self.angle)?;
        let projected_bound =
            support.certify(&projected).map_err(step_error)? + planar_drift * self.top_speed();
        require(cap_bound + projected_bound < self.model)?;
        self.boundary(shell, edge, BoundaryEdge::TopCavity)?;
        self.top_cavity_uv(shell, cavity, cap_bound + surface_bound)
    }

    fn top_cavity_uv(self, shell: &Shell<'_, '_>, uv: u32, already_used: f64) -> Result<()> {
        let decode::Curve {
            mut axis,
            mut points,
        } = decode::curve(shell, uv)?;
        let domain = self.cut.edge_domain(BoundaryEdge::TopCavity);
        let chart_drift = align_domain(&mut axis.knots, domain, self.angle)?;
        let support = CavitySurface::new(self.cut, self.model).map_err(step_error)?;
        let [first, .., last] = points.as_mut_slice() else {
            return Err(ReadError::Invalid);
        };
        let spec = self.cut.spec();
        let sweep_speed = self.sweep_speed();
        let mut correction: f64 = 0.;
        // The reconstructed chart can differ from the encoded chart by
        // roundoff. Pin its cap endpoints only after verifying the shift,
        // and charge the full convex-hull error of that coefficient change.
        for (point, angle) in [first, last].into_iter().zip(domain) {
            let expected = [
                angle,
                support.top_parameter(angle).ok_or(ReadError::Invalid)?,
            ];
            near(point[0], expected[0], self.angle)?;
            near(point[1], expected[1], self.angle)?;
            let drift = spec.tube_radius_mm * (point[0] - expected[0]).abs()
                + sweep_speed * (point[1] - expected[1]).abs();
            correction = correction.max(drift);
            *point = expected;
        }
        let support = CavityCapPcurve::new(self.cut, self.model).map_err(step_error)?;
        let error = support
            .certify(&UvSpline {
                degree: axis.degree,
                knots: axis.knots,
                multiplicities: axis.multiplicities,
                poles_uv: points,
            })
            .map_err(step_error)?;
        require(already_used + error + correction + chart_drift * self.top_speed() < self.model)
    }
}

fn sine_hermite_pole(angle: f64, width: f64, index: usize) -> f64 {
    let (sin, cos) = angle.sin_cos();
    let derivatives = [sin, cos, -sin, -cos];
    let mut coefficient = 1.;
    let mut pole = sin;
    for k in 0..index {
        coefficient *= width * (index - k) as f64 / ((k + 1) * (11 - k)) as f64;
        pole = coefficient.mul_add(derivatives[(k + 1) % 4], pole);
    }
    pole
}

#[cfg(test)]
#[path = "helical_cut/tests.rs"]
mod tests;
