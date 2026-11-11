//! Certified tensor walls for the retained fixed-binormal Custom transport.
use super::*;
use crate::tapered_thread::{
    Error, MANUFACTURING_DEGREE as DEGREE, ManufacturingTensor, ManufacturingTensors, Thread,
    interval::I,
};

pub fn write_tapered_thread(
    thread: &Thread,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_tapered_thread(thread, origin, u, v, tolerance))
}

pub(crate) fn map_error(error: Error) -> StepError {
    match error {
        Error::Limit => StepError::Limit,
        Error::Invalid | Error::Unsupported | Error::Unresolved | Error::Cancelled => {
            StepError::Unsupported
        }
    }
}

/// Complete source nets, after start rotation and the requested placement.
pub(crate) struct Prepared {
    pub tensors: ManufacturingTensors,
    pub points: Vec<Vec<Point>>,
    placement: Placement,
}

fn dot_i(a: [I; 3], b: [I; 3]) -> I {
    a.into_iter()
        .zip(b)
        .fold(I::point(0.), |s, (a, b)| s + a * b)
}
fn cross_i(a: [I; 3], b: [I; 3]) -> [I; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn prepare(thread: &Thread, placement: Placement, tolerance: f64) -> Result<Prepared, StepError> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let tolerance = tolerance.min(thread.tolerance_mm()).min(1e-7);
    let tensors = thread
        .manufacturing_tensors(tolerance, None)
        .map_err(map_error)?;
    let axes = [placement.u, placement.v, placement.normal].map(|v| v.map(I::point));
    let inverse = [
        cross_i(axes[1], axes[2]),
        cross_i(axes[2], axes[0]),
        cross_i(axes[0], axes[1]),
    ];
    let determinant = dot_i(axes[0], inverse[0]);
    if determinant.lo <= 0. || !determinant.valid() {
        return Err(StepError::InvalidPlacement);
    }
    let inverse = inverse.map(|row| row.map(|v| v / determinant));
    let mut gram = I::point(0.);
    for i in 0..3 {
        for j in 0..3 {
            let e = dot_i(axes[i], axes[j]) - I::point(if i == j { 1. } else { 0. });
            gram = gram + I::point(e.magnitude()).square();
        }
    }
    let spec = thread.spec();
    let radius = I::point(spec.major_radius_mm)
        + I::point(thread.radial_taper().abs()) * I::point(spec.length_mm);
    let size = (I::point(2.) * radius).square() + I::point(spec.length_mm).square();
    let conditioning = (gram.sqrt() * size.sqrt()).hi;
    let budget = (tolerance / 8.).min(tensors.feature_margin_mm() / 128.);
    if !conditioning.is_finite() || conditioning > budget {
        return Err(StepError::Unsupported);
    }
    let mut points = Vec::new();
    let mut charts = Vec::new();
    for start in 0..spec.starts {
        let angle = std::f64::consts::TAU * start as f64 / spec.starts as f64
            * if spec.left_hand { 1. } else { -1. };
        let [sin, cos] = [angle.sin(), angle.cos()];
        let ai = I::point(std::f64::consts::TAU) * I::point(start as f64)
            / I::point(spec.starts as f64)
            * I::point(if spec.left_hand { 1. } else { -1. });
        let [si, ci] = if start == 0 {
            [I::point(0.), I::point(1.)]
        } else {
            [ai.sin(), ai.cos()]
        };
        for tensor in tensors.walls() {
            let mut world = Vec::with_capacity(tensor.poles().len());
            let mut chart = Vec::with_capacity(tensor.poles().len());
            for &[x, y, z] in tensor.poles() {
                let local = [x * cos - y * sin, x * sin + y * cos, z];
                let ideal = [
                    I::point(x) * ci - I::point(y) * si,
                    I::point(x) * si + I::point(y) * ci,
                    I::point(z),
                ];
                let expected = std::array::from_fn::<_, 3, _>(|i| {
                    I::point(placement.origin[i])
                        + axes[0][i] * ideal[0]
                        + axes[1][i] * ideal[1]
                        + axes[2][i] * ideal[2]
                });
                let p = placement.point_f64([local[0], local[1]], local[2]);
                let error = (0..3).fold(I::point(0.), |s, i| {
                    s + I::point((expected[i] - I::point(p[i])).magnitude())
                });
                if p.into_iter().any(|v| !v.is_finite() || v.abs() > 1e7)
                    || !error.valid()
                    || (I::point(error.hi) + I::point(conditioning)).hi > budget
                {
                    return Err(StepError::Unsupported);
                }
                let delta = std::array::from_fn(|i| I::point(p[i]) - I::point(placement.origin[i]));
                // The chart is expressed in this start's source coordinates.
                let q = inverse.map(|row| dot_i(row, delta));
                chart.push([q[0] * ci + q[1] * si, -q[0] * si + q[1] * ci, q[2]]);
                world.push(p);
            }
            points.push(world);
            charts.push(chart);
        }
    }
    thread
        .certify_manufacturing_charts(&tensors, &charts, None)
        .map_err(map_error)?;
    Ok(Prepared {
        tensors,
        points,
        placement,
    })
}

pub(crate) fn source_nets(
    thread: &Thread,
    frame: [Point; 3],
    tolerance: f64,
) -> Result<Prepared, StepError> {
    prepare(
        thread,
        Placement::new(frame[0], frame[1], frame[2])?,
        tolerance,
    )
}

fn knot_data(spans: usize) -> (String, String) {
    let mult = (0..=spans)
        .map(|i| {
            if i == 0 || i == spans {
                DEGREE + 1
            } else {
                DEGREE
            }
            .to_string()
        })
        .collect::<Vec<_>>()
        .join(",");
    let knots = (0..=spans)
        .map(|i| format!("{:.16E}", i as f64 / spans as f64))
        .collect::<Vec<_>>()
        .join(",");
    (mult, knots)
}
fn spline(file: &mut StepFile, points: &[Entity], spans: usize) -> Entity {
    let (mult, knots) = knot_data(spans);
    file.entity(format_args!("B_SPLINE_CURVE_WITH_KNOTS('',{DEGREE},{},.UNSPECIFIED.,.F.,.F.,({mult}),({knots}),.UNSPECIFIED.)", Entities(points)))
}
fn spline_edge(file: &mut StepFile, points: &[Entity], ends: [Vertex; 2], spans: usize) -> Entity {
    let curve = spline(file, points, spans);
    let [a, b] = ends.map(|v| v.vertex);
    file.entity(format_args!("EDGE_CURVE('',{a},{b},{curve},.T.)"))
}
fn surface(file: &mut StepFile, tensor: &ManufacturingTensor, points: &[Entity]) -> Entity {
    let height = tensor.dimensions()[1];
    let rows = points
        .chunks_exact(height)
        .map(|r| Entities(r).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let [nu, nz] = tensor.spans();
    let (um, uk) = knot_data(nu);
    let (vm, vk) = knot_data(nz);
    file.entity(format_args!("B_SPLINE_SURFACE_WITH_KNOTS('',{DEGREE},{DEGREE},({rows}),.UNSPECIFIED.,.F.,.F.,.F.,({um}),({vm}),({uk}),({vk}),.UNSPECIFIED.)"))
}
fn wire(file: &mut StepFile, edges: &[(Entity, bool)], reverse: bool) -> Entity {
    let edges = if reverse {
        edges
            .iter()
            .rev()
            .map(|&(id, sense)| file.oriented_edge(id, !sense))
            .collect::<Vec<_>>()
    } else {
        edges
            .iter()
            .map(|&(id, sense)| file.oriented_edge(id, sense))
            .collect::<Vec<_>>()
    };
    file.bound(&edges, true)
}

pub(super) fn encode(
    file: &mut StepFile,
    thread: &Thread,
    placement: Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    let prepared = prepare(thread, placement, tolerance)?;
    let mut candidate = StepFile::new();
    let solid = encode_prepared(&mut candidate, thread, &prepared)?;
    candidate.finish(vec![solid]);
    certify_candidate(&candidate.data, thread, &prepared, tolerance)?;
    encode_prepared(file, thread, &prepared)
}
fn certify_candidate(
    input: &str,
    thread: &Thread,
    prepared: &Prepared,
    tolerance: f64,
) -> Result<(), StepError> {
    crate::step_read::tapered_thread::verify_prepared(
        input,
        thread,
        [
            prepared.placement.origin,
            prepared.placement.u,
            prepared.placement.v,
        ],
        tolerance,
        prepared,
    )
    .map_err(|error| {
        if error == crate::step_read::ReadError::Limit {
            StepError::Limit
        } else {
            StepError::Unsupported
        }
    })
}

fn encode_prepared(
    file: &mut StepFile,
    thread: &Thread,
    prepared: &Prepared,
) -> Result<Entity, StepError> {
    let placement = &prepared.placement;
    let spec = thread.spec();
    let mut surfaces = Vec::new();
    let mut rims = Vec::<[Entity; 2]>::new();
    let mut rails = Vec::<Entity>::new();
    let mut corners = Vec::<[Vertex; 2]>::new();
    for start in 0..spec.starts as usize {
        for band in 0..3 {
            let tensor = &prepared.tensors.walls()[band];
            let [width, height] = tensor.dimensions();
            let points = &prepared.points[start * 3 + band];
            let ids = points.iter().map(|&p| file.point(p)).collect::<Vec<_>>();
            surfaces.push(surface(file, tensor, &ids));
            let left = if band == 0 {
                [file.vertex(points[0]), file.vertex(points[height - 1])]
            } else {
                *corners.last().expect("preceding crease")
            };
            let right = [
                file.vertex(points[(width - 1) * height]),
                file.vertex(points[width * height - 1]),
            ];
            if band == 0 {
                corners.push(left);
                rails.push(spline_edge(file, &ids[..height], left, tensor.spans()[1]));
            }
            corners.push(right);
            rails.push(spline_edge(
                file,
                &ids[(width - 1) * height..],
                right,
                tensor.spans()[1],
            ));
            let cap = std::array::from_fn(|end| {
                let ids = (0..width)
                    .map(|i| ids[i * height + end * (height - 1)])
                    .collect::<Vec<_>>();
                spline_edge(file, &ids, [left[end], right[end]], tensor.spans()[0])
            });
            rims.push(cap);
        }
    }
    let mut gaps = Vec::<[Vec<Entity>; 2]>::new();
    for start in 0..spec.starts as usize {
        let next = (start + 1) % spec.starts as usize;
        let ends = [corners[start * 4 + 3], corners[next * 4]];
        let mut make_arcs = |end: usize| -> Result<Vec<Entity>, StepError> {
            let z = if end == 0 { 0. } else { spec.length_mm };
            let center = placement.point_f64([0., 0.], z);
            let radius = thread.root_radius_at(z).map_err(map_error)?;
            let radius = (radius[0] + radius[1]) * 0.5;
            let to_local = |p: Point| {
                let d = std::array::from_fn(|i| p[i] - center[i]);
                [dot(d, placement.u), dot(d, placement.v)]
            };
            let a = to_local(ends[0][end].point);
            let b = to_local(ends[1][end].point);
            let from = a[1].atan2(a[0]);
            let to = b[1].atan2(b[0]);
            let sweep = if spec.left_hand {
                (to - from).rem_euclid(std::f64::consts::TAU)
            } else {
                -(from - to).rem_euclid(std::f64::consts::TAU)
            };
            let mid = from + sweep * 0.5;
            let middle =
                file.vertex(placement.point_f64([radius * mid.cos(), radius * mid.sin()], z));
            let c = file.point(center);
            let axis = file.axis(c, placement.normal, placement.u);
            let circle = file.entity(format_args!("CIRCLE('',{axis},{radius:.16E})"));
            Ok([ends[0][end], middle, ends[1][end]]
                .windows(2)
                .map(|pair| {
                    let [a, b] = [pair[0].vertex, pair[1].vertex];
                    let sense = if spec.left_hand { ".T." } else { ".F." };
                    file.entity(format_args!("EDGE_CURVE('',{a},{b},{circle},{sense})"))
                })
                .collect::<Vec<_>>())
        };
        let arcs = [make_arcs(0)?, make_arcs(1)?];
        gaps.push(arcs);
    }
    let mut faces = Vec::new();
    for end in 0..2 {
        let mut edges = Vec::new();
        for start in 0..spec.starts as usize {
            edges.extend((0..3).map(|band| (rims[start * 3 + band][end], true)));
            edges.extend(gaps[start][end].iter().map(|&id| (id, true)));
        }
        let bound = wire(file, &edges, spec.left_hand != (end == 1));
        let p =
            file.point(placement.point_f64([0., 0.], if end == 0 { 0. } else { spec.length_mm }));
        faces.push(file.face(
            &[bound],
            p,
            placement.normal.map(|x| if end == 0 { -x } else { x }),
            placement.u,
        ));
    }
    let k = thread.radial_taper();
    let (z, axis) = if k < 0. {
        (spec.length_mm, placement.normal.map(|x| -x))
    } else {
        (0., placement.normal)
    };
    let p = file.point(placement.point_f64([0., 0.], z));
    let axis = file.axis(p, axis, placement.u);
    let r = thread.root_radius_at(z).map_err(map_error)?;
    let r = (r[0] + r[1]) * 0.5;
    let cone = if k == 0. {
        file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{r:.16E})"))
    } else {
        let angle = k.abs().atan();
        file.entity(format_args!(
            "CONICAL_SURFACE('',{axis},{r:.16E},{angle:.16E})"
        ))
    };
    for start in 0..spec.starts as usize {
        for band in 0..3 {
            let i = start * 3 + band;
            let edges = [
                (rims[i][0], true),
                (rails[start * 4 + band + 1], true),
                (rims[i][1], false),
                (rails[start * 4 + band], false),
            ];
            let bound = wire(file, &edges, !spec.left_hand);
            faces.push(file.face_on(&[bound], surfaces[i], spec.left_hand));
        }
        let next = (start + 1) % spec.starts as usize;
        let mut edges = gaps[start][0]
            .iter()
            .map(|&id| (id, true))
            .collect::<Vec<_>>();
        edges.push((rails[next * 4], true));
        edges.extend(gaps[start][1].iter().rev().map(|&id| (id, false)));
        edges.push((rails[start * 4 + 3], false));
        let bound = wire(file, &edges, !spec.left_hand);
        faces.push(file.face_on(&[bound], cone, true));
    }
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(starts: u32, left_hand: bool, k: f64) -> Thread {
        Thread::new(
            crate::helical_thread::Spec {
                major_radius_mm: 4.,
                pitch_mm: 1.25,
                length_mm: 8.,
                depth_mm: 0.5,
                crest_width_mm: 0.25,
                lower_flank_deg: 12.,
                upper_flank_deg: 18.,
                starts,
                left_hand,
                root_radius_mm: None,
            },
            k,
            1e-7,
            1e-6,
        )
        .unwrap()
    }
    #[test]
    fn actual_transport_walls_share_all_shell_edges() {
        let t = Thread::new(
            crate::helical_thread::Spec {
                major_radius_mm: 4.,
                pitch_mm: 1.25,
                length_mm: 8.,
                depth_mm: 0.5,
                crest_width_mm: 0.25,
                lower_flank_deg: 15.,
                upper_flank_deg: 15.,
                starts: 1,
                left_hand: false,
                root_radius_mm: None,
            },
            0.1,
            1e-7,
            1e-6,
        )
        .unwrap();
        let now = std::time::Instant::now();
        let text = write_tapered_thread(&t, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7).unwrap();
        eprintln!("taper STEP {:?} bytes{}", now.elapsed(), text.len());
        assert_eq!(text.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 3);
        assert_eq!(text.matches("ADVANCED_FACE").count(), 6);
        crate::step_read::verify_tapered_thread(
            &text,
            &t,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            1e-7,
        )
        .unwrap();
    }
    #[test]
    fn placed_signed_multistart_nets_and_shared_trims_survive_readback() {
        for (starts, left, k) in [
            (1, true, -0.1),
            (3, false, -0.03125),
            (3, true, 0.03125),
            (1, false, 0.),
        ] {
            let t = fixture(starts, left, k);
            let origin = [1000., -700., 300.];
            let u = [0.8, 0.6, 0.];
            let v = [-0.36, 0.48, 0.8];
            let text = write_tapered_thread(&t, origin, u, v, 1e-7).unwrap();
            crate::step_read::verify_tapered_thread(&text, &t, origin, u, v, 1e-7).unwrap();
            assert_eq!(
                text.matches("ADVANCED_FACE").count(),
                2 + 4 * starts as usize
            );
        }
    }
    #[test]
    fn manufacturing_and_readback_fail_atomically_then_recover() {
        let t = fixture(1, false, 0.1);
        let origin = [0.; 3];
        let u = [1., 0., 0.];
        let v = [0., 1., 0.];
        for tol in [0., -1., f64::NAN] {
            assert_eq!(
                write_tapered_thread(&t, origin, u, v, tol),
                Err(StepError::InvalidGeometry)
            );
        }
        assert_eq!(
            write_tapered_thread(&t, origin, [0.; 3], v, 1e-7),
            Err(StepError::InvalidPlacement)
        );
        assert_eq!(
            write_tapered_thread(&t, [1e7; 3], u, v, 1e-7),
            Err(StepError::Unsupported)
        );
        assert!(matches!(
            write_tapered_thread(&t, origin, u, v, 1e-12),
            Err(StepError::Unsupported)
        ));
        assert_eq!(
            write_tapered_thread(&t, origin, u, v, 1e-90),
            Err(StepError::Limit)
        );
        let almost_unit = 1. + 4e-11;
        assert_eq!(
            write_tapered_thread(
                &t,
                origin,
                [almost_unit, 0., 0.],
                [0., almost_unit, 0.],
                1e-8
            ),
            Err(StepError::Unsupported)
        );
        let singular = Placement {
            origin,
            u: [0.; 3],
            v,
            normal: [0.; 3],
        };
        assert!(matches!(
            prepare(&t, singular, 1e-7),
            Err(StepError::InvalidPlacement)
        ));
        let text = write_tapered_thread(&t, origin, u, v, 1e-7).unwrap();
        let prepared = source_nets(&t, [origin, u, v], 1e-7).unwrap();
        assert_eq!(
            certify_candidate("not STEP", &t, &prepared, 1e-7),
            Err(StepError::Unsupported)
        );
        let oversized = " ".repeat(crate::step_read::MAX_PLANAR_STEP_BYTES + 1);
        assert_eq!(
            certify_candidate(&oversized, &t, &prepared, 1e-7),
            Err(StepError::Limit)
        );
        drop(oversized);
        certify_candidate(&text, &t, &prepared, 1e-7).unwrap();
        assert!(
            crate::step_read::verify_tapered_thread(
                &text,
                &fixture(1, true, 0.1),
                origin,
                u,
                v,
                1e-7
            )
            .is_err()
        );
        for (from, to) in [
            (
                "B_SPLINE_SURFACE_WITH_KNOTS('',9,9",
                "B_SPLINE_SURFACE_WITH_KNOTS('',8,9",
            ),
            ("CONICAL_SURFACE", "CYLINDRICAL_SURFACE"),
            ("CIRCLE('',", "ELLIPSE('',"),
        ] {
            let changed = text.replacen(from, to, 1);
            assert_ne!(changed, text);
            assert!(
                crate::step_read::verify_tapered_thread(&changed, &t, origin, u, v, 1e-7).is_err()
            );
        }
        let mut writer = Writer::new();
        writer
            .append_tapered_thread(&t, origin, u, v, 1e-7)
            .unwrap();
        assert!(
            writer
                .append_tapered_thread(&t, [1e7; 3], u, v, 1e-7)
                .is_err()
        );
        writer
            .append_tapered_thread(&t, [20., 0., 0.], u, v, 1e-7)
            .unwrap();
        assert_eq!(
            writer
                .finish()
                .unwrap()
                .matches("MANIFOLD_SOLID_BREP")
                .count(),
            2
        );
    }
    #[test]
    fn source_identical_walls_cannot_be_sewn_into_a_multiwrap_root_cap() {
        let t = fixture(3, false, 0.1);
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let mut prepared = source_nets(&t, frame, 1e-7).unwrap();
        // Every finite wall remains a genuine source wall. Sewing physical
        // starts in order 0,2,1 gives two positive angular wraps, however.
        for band in 0..3 {
            prepared.points.swap(3 + band, 6 + band);
        }
        let turn = |a: Point, b: Point| {
            (a[1].atan2(a[0]) - b[1].atan2(b[0])).rem_euclid(std::f64::consts::TAU)
        };
        let mut winding = 0.;
        for start in 0..3 {
            for band in 0..3 {
                let [width, height] = prepared.tensors.walls()[band].dimensions();
                let points = &prepared.points[start * 3 + band];
                winding += turn(points[0], points[(width - 1) * height]);
                if band == 2 {
                    winding += turn(
                        points[(width - 1) * height],
                        prepared.points[((start + 1) % 3) * 3][0],
                    );
                }
            }
        }
        assert!((winding - 2. * std::f64::consts::TAU).abs() < 1e-10);
        let mut file = StepFile::new();
        let solid = encode_prepared(&mut file, &t, &prepared).unwrap();
        file.finish(vec![solid]);
        assert_eq!(
            crate::step_read::verify_tapered_thread(
                &file.data, &t, frame[0], frame[1], frame[2], 1e-7
            ),
            Err(crate::step_read::ReadError::Invalid)
        );
    }
}
