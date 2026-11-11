//! ZA worms as certified helical thread shells with an inner bore wall.
use super::*;
use crate::{
    step_read::MAX_STEP_ENTITIES,
    worm_gear::{
        Wheel, Worm, rotate_z,
        wheel_fit::{Fit, V_DEGREE},
    },
};

/// World frame of the worm's thread coordinates for a placed local frame.
pub(crate) fn thread_frame(worm: &Worm, [origin, u, v]: [Point; 3]) -> [Point; 3] {
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let (s, c) = worm.phase_rad().sin_cos();
    let half = worm.length_mm() / 2.;
    [
        std::array::from_fn(|i| origin[i] - half * n[i]),
        std::array::from_fn(|i| c * u[i] + s * v[i]),
        std::array::from_fn(|i| c * v[i] - s * u[i]),
    ]
}

pub fn write_za_worm(
    worm: &Worm,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_za_worm(worm, origin, u, v, tolerance_mm))
}

/// Whether the placed worm fits the certified helical STEP budget.
pub fn supports_za_worm(worm: &Worm, origin: Point, u: Point, v: Point, tolerance_mm: f64) -> bool {
    let [origin, ..] = thread_frame(worm, [origin, u, v]);
    helical::prepare(
        worm.thread(),
        helical::Closure::hollow(worm.bore_radius_mm()),
        origin,
        tolerance_mm,
    )
    .is_ok()
}

/// Whether the placed generated wheel fits the certified chart budget.
pub fn supports_worm_wheel(
    wheel: &Wheel,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> bool {
    Placement::new(origin, u, v).is_ok() && prepare_wheel(wheel, tolerance_mm).is_ok()
}

pub fn write_worm_wheel(
    wheel: &Wheel,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_worm_wheel(wheel, origin, u, v, tolerance_mm))
}

/// Certified charts of a wheel within the entity budget.
pub(crate) fn prepare_wheel(wheel: &Wheel, tolerance_mm: f64) -> Result<Fit, StepError> {
    if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let fit = wheel.fit(tolerance_mm).ok_or(StepError::Limit)?;
    let width = fit.spans * V_DEGREE + 1;
    let poles: usize = fit.charts.iter().map(|c| (c.degree + 1) * width).sum();
    // Poles dominate; each sector adds about 160 topology entities.
    let cost = (poles + 160) * wheel.teeth() as usize + 256;
    if cost > MAX_STEP_ENTITIES {
        return Err(StepError::Limit);
    }
    Ok(fit)
}

/// Knot values and multiplicities of `spans` equal Bézier spans of `degree`.
fn bezier_knots(degree: usize, spans: usize) -> String {
    let mults = (0..=spans)
        .map(|s| {
            if s == 0 || s == spans {
                degree + 1
            } else {
                degree
            }
        })
        .map(|m| m.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let values = (0..=spans)
        .map(|s| format!("{:.16E}", s as f64 / spans as f64))
        .collect::<Vec<_>>()
        .join(",");
    format!("({mults}),({values})")
}

pub(super) fn encode_wheel(
    file: &mut StepFile,
    wheel: &Wheel,
    placement: Placement,
    tolerance_mm: f64,
) -> Result<Entity, StepError> {
    let fit = prepare_wheel(wheel, tolerance_mm)?;
    let topology = wheel.topology();
    let teeth = wheel.teeth();
    let world = |p: Point| placement.point_f64([p[0], p[1]], p[2]);
    let vertices: Vec<Vertex> = topology
        .vertices_mm
        .iter()
        .map(|p| file.vertex(world(*p)))
        .collect();
    let bore = wheel.bore_radius_mm();
    let wrap = |k: u32| (k % teeth) as usize;
    let joint = |k: u32, e: usize, j: usize| vertices[wrap(k) * 14 + e * 7 + j];
    let base = 14 * teeth as usize;
    let inner =
        |k: u32, e: usize| vertices[base + if bore.is_some() { 2 * wrap(k) + e } else { e }];
    let m = wheel.module_mm();
    let span = std::f64::consts::PI / f64::from(teeth);
    let blank = (f64::from(teeth) / 2. + 1.) * m;
    let width_knots = bezier_knots(V_DEGREE, fit.spans);
    let last = fit.spans * V_DEGREE;
    // A Bézier boundary curve through rotated chart poles.
    let spline =
        |file: &mut StepFile, poles: Vec<Point>, k: u32, [a, b]: [Vertex; 2], knots: &str| {
            let degree = if poles.len() == last + 1 {
                V_DEGREE
            } else {
                poles.len() - 1
            };
            let count = poles.len();
            let points: Vec<Entity> = poles
                .into_iter()
                .enumerate()
                .map(|(i, p)| match i {
                    0 => a.cartesian,
                    i if i + 1 == count => b.cartesian,
                    _ => file.point(world(rotate_z(p, wheel.sector_angle(k)))),
                })
                .collect();
            let curve = file.entity(format_args!(
            "B_SPLINE_CURVE_WITH_KNOTS('',{degree},{},.UNSPECIFIED.,.F.,.F.,{knots},.UNSPECIFIED.)",
            Entities(&points),
        ));
            file.entity(format_args!(
                "EDGE_CURVE('',{},{},{curve},.T.)",
                a.vertex, b.vertex
            ))
        };
    let arc =
        |file: &mut StepFile, [a, b]: [Vertex; 2], z: f64, radius: f64, start: f64, sweep: f64| {
            let reference = placement.vector([start.cos(), start.sin(), 0.]);
            circular_arc_edge(
                file,
                [a, b],
                world([0., 0., z]),
                placement.normal,
                reference,
                radius,
                sweep,
            )
            .map(|(edge, _)| edge)
        };
    struct SectorEdges {
        radial: [Entity; 2],
        chain: [[Entity; 7]; 2],
        bore: Option<[Entity; 2]>,
        junction: [Entity; 7],
    }
    let mut sectors = Vec::with_capacity(teeth as usize);
    for k in 0..teeth {
        let angle = wheel.sector_angle(k);
        let end = |file: &mut StepFile, e: usize| -> Result<_, StepError> {
            let z = wheel.end_v(e) * m;
            let [low, high] = wheel.section(wheel.end_v(e)).exit;
            let radial = line_edge(file, inner(k, e), joint(k, e, 0))?;
            let first = arc(
                file,
                [joint(k, e, 0), joint(k, e, 1)],
                z,
                blank,
                angle - span,
                low + span,
            )?;
            let profiles: [Entity; 5] = std::array::from_fn(|piece| {
                let chart = &fit.charts[piece];
                let row = chart
                    .poles
                    .iter()
                    .map(|r| r[if e == 0 { 0 } else { last }])
                    .collect();
                let ends = [joint(k, e, piece + 1), joint(k, e, piece + 2)];
                spline(file, row, k, ends, &bezier_knots(chart.degree, 1))
            });
            let closing = arc(
                file,
                [joint(k, e, 6), joint(k + 1, e, 0)],
                z,
                blank,
                angle + high,
                span - high,
            )?;
            let [a, b, c, d, f] = profiles;
            let rim = match bore {
                Some(radius) => Some(arc(
                    file,
                    [inner(k, e), inner(k + 1, e)],
                    z,
                    radius,
                    angle - span,
                    2. * span,
                )?),
                None => None,
            };
            Ok((radial, [first, a, b, c, d, f, closing], rim))
        };
        let (start, end) = (end(file, 0)?, end(file, 1)?);
        let line = line_edge(file, joint(k, 0, 0), joint(k, 1, 0))?;
        let columns: [Entity; 6] = std::array::from_fn(|i| {
            let chart = &fit.charts[i.min(4)];
            let column = chart.poles[if i == 5 { chart.degree } else { 0 }].clone();
            let ends = [joint(k, 0, i + 1), joint(k, 1, i + 1)];
            spline(file, column, k, ends, &width_knots)
        });
        let [a, b, c, d, e, f] = columns;
        sectors.push(SectorEdges {
            radial: [start.0, end.0],
            chain: [start.1, end.1],
            bore: start.2.zip(end.2).map(|(x, y)| [x, y]),
            junction: [line, a, b, c, d, e, f],
        });
    }
    let origin = file.point(placement.origin);
    let axis_z = placement.normal;
    let tip = {
        let axis = file.axis(origin, axis_z, placement.u);
        file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{blank:.16E})"))
    };
    let mut faces = vec![];
    for k in 0..teeth {
        let edges = &sectors[wrap(k)];
        let next = &sectors[wrap(k + 1)];
        for e in 0..2 {
            let mut ring = vec![(edges.radial[e], true)];
            ring.extend(edges.chain[e].iter().map(|&edge| (edge, true)));
            ring.push((next.radial[e], false));
            if let Some(rims) = edges.bore {
                ring.push((rims[e], false));
            }
            if e == 0 {
                ring.reverse();
                for item in &mut ring {
                    item.1 = !item.1;
                }
            }
            let oriented: Vec<_> = ring
                .into_iter()
                .map(|(edge, f)| file.oriented_edge(edge, f))
                .collect();
            let bound = file.bound(&oriented, true);
            let point = file.point(world([0., 0., wheel.end_v(e) * m]));
            let normal = axis_z.map(|x| if e == 0 { -x } else { x });
            faces.push(file.face(&[bound], point, normal, placement.u));
        }
        for (piece, chart) in fit.charts.iter().enumerate() {
            let rows = chart
                .poles
                .iter()
                .map(|row| {
                    let row: Vec<Entity> = row
                        .iter()
                        .map(|&p| file.point(world(rotate_z(p, wheel.sector_angle(k)))))
                        .collect();
                    Entities(&row).to_string()
                })
                .collect::<Vec<_>>()
                .join(",");
            let [(um, uk), (vm, vk)] =
                [bezier_knots(chart.degree, 1), width_knots.clone()].map(|knots| {
                    let (mults, values) = knots.split_once("),(").expect("knot lists");
                    (format!("{mults})"), format!("({values}"))
                });
            let surface = file.entity(format_args!(
                "B_SPLINE_SURFACE_WITH_KNOTS('',{},{V_DEGREE},({rows}),.UNSPECIFIED.,.F.,.F.,.F.,{um},{vm},{uk},{vk},.UNSPECIFIED.)",
                chart.degree,
            ));
            let ring = [
                file.oriented_edge(edges.chain[0][piece + 1], true),
                file.oriented_edge(edges.junction[piece + 2], true),
                file.oriented_edge(edges.chain[1][piece + 1], false),
                file.oriented_edge(edges.junction[piece + 1], false),
            ];
            let bound = file.bound(&ring, true);
            faces.push(file.face_on(&[bound], surface, true));
        }
        for (start, side, top, end) in [
            (
                edges.chain[0][0],
                edges.junction[1],
                edges.chain[1][0],
                edges.junction[0],
            ),
            (
                edges.chain[0][6],
                next.junction[0],
                edges.chain[1][6],
                edges.junction[6],
            ),
        ] {
            let ring = [
                file.oriented_edge(start, true),
                file.oriented_edge(side, true),
                file.oriented_edge(top, false),
                file.oriented_edge(end, false),
            ];
            let bound = file.bound(&ring, true);
            faces.push(file.face_on(&[bound], tip, true));
        }
    }
    if let Some(radius) = bore {
        let axis = file.axis(origin, axis_z, placement.u);
        let surface = file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"));
        let bottom: Vec<_> = (0..teeth)
            .rev()
            .map(|k| file.oriented_edge(sectors[wrap(k)].bore.expect("bore")[0], false))
            .collect();
        let top: Vec<_> = (0..teeth)
            .map(|k| file.oriented_edge(sectors[wrap(k)].bore.expect("bore")[1], true))
            .collect();
        let bounds = [file.bound(&bottom, true), file.bound(&top, false)];
        faces.push(file.face_on(&bounds, surface, false));
    }
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        step_read::verify_za_worm,
        worm_gear::{WormSpec, worm::tests::spec},
    };

    #[test]
    fn worms_with_and_without_bores_round_trip_their_volume() {
        let d = 0.5_f64.sqrt();
        let frame = [[3., -2., 7.], [d, d, 0.], [0., 0., 1.]];
        for spec in [spec(1., 20., 0., 0.02), spec(2., 16., 3., 0.1)] {
            let worm = Worm::new(spec, 1e-3).unwrap();
            let [o, u, v] = frame;
            assert!(supports_za_worm(&worm, o, u, v, 1e-7));
            let text = write_za_worm(&worm, o, u, v, 1e-7).unwrap();
            let volume = verify_za_worm(&text, &worm, o, u, v, 1e-7).unwrap();
            let expected = worm.volume_mm3();
            assert!(
                (volume - expected).abs() / expected < 1e-8,
                "{volume} {expected}"
            );
            // A different bore is rejected by the readback proof.
            let other = Worm::new(
                WormSpec {
                    bore_radius_mm: spec.bore_radius_mm + 0.5,
                    ..spec
                },
                1e-3,
            )
            .unwrap();
            assert!(verify_za_worm(&text, &other, o, u, v, 1e-7).is_err());
        }
        let worm = Worm::new(spec(1., 20., 1., 0.), 1e-3).unwrap();
        let [o, u, v] = frame;
        assert!(!supports_za_worm(&worm, o, u, v, 0.));
        assert!(write_za_worm(&worm, o, u, v, f64::NAN).is_err());
    }
}
