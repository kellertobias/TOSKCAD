//! Readback proofs for Rust ZA worm and generated worm-wheel STEP shells.
//!
//! The wheel proof decodes every face, edge and vertex of one closed shell
//! and checks manifold edge use against the authored sector topology.
//! Planes, tip and bore cylinders, lines and circles are compared with the
//! authored geometry directly. Each spline face must be a C⁰ chain of
//! Bézier spans of degree `(nu, V_DEGREE)` on the authored patch domain:
//! the proof evaluates the decoded poles at the Chebyshev interpolation
//! points, compares them with exact patch values, and bounds its whole
//! surface error by the interpolation remainder plus the Lebesgue-amplified
//! node residual (`worm_gear/wheel_fit.rs`). Spline edges must be boundary
//! curves of such a face. The volume is the flux of `(0, 0, z)` through the
//! decoded boundary: cylinders carry none, end planes carry `z` times their
//! signed area from Green's theorem on the decoded edges, and spline faces
//! are integrated exactly by Gauss quadrature of the decoded polynomials.
use super::{
    helical::{face, require, ring},
    *,
};
use crate::{
    bevel_gear::Containment,
    step::helical::Closure,
    worm_gear::{
        Wheel, Worm, rotate_z,
        wheel_bounds::lebesgue,
        wheel_fit::{Chart, V_DEGREE, bezier, chart_residual},
    },
};

/// Verify the serialized worm against its certified thread shell and bore,
/// returning the volume integrated from the decoded boundary in mm³.
pub fn verify_za_worm(
    input: &str,
    worm: &Worm,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let mut volumes = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        verify_worm_member(shell, id, worm, [origin, u, v])
    })?;
    volumes.pop().ok_or(ReadError::Invalid)
}

/// Verify one worm shell of a flat representation.
pub(super) fn verify_worm_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
    worm: &Worm,
    frame: [Point; 3],
) -> Result<f64> {
    let frame = crate::step::worm_thread_frame(worm, frame);
    helical::verify_member(
        shell,
        id,
        worm.thread(),
        Closure::hollow(worm.bore_radius_mm()),
        frame,
        None,
    )
}

/// Verify one serialized generated wheel and return the volume integrated
/// from its decoded boundary in mm³.
pub fn verify_worm_wheel(
    input: &str,
    wheel: &Wheel,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let mut volumes = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        verify_wheel_member(shell, id, wheel, [origin, u, v], tolerance)
    })?;
    volumes.pop().ok_or(ReadError::Invalid)
}

struct Frame {
    origin: Point,
    axes: [Point; 3],
}

impl Frame {
    fn local(&self, world: Point) -> Point {
        let relative = sub(world, self.origin);
        self.axes.map(|axis| dot(axis, relative))
    }

    fn direction(&self, world: Point) -> Point {
        self.axes.map(|axis| dot(axis, world))
    }
}

/// Gauss–Legendre nodes and weights on `[0, 1]`.
fn gauss(n: usize) -> Vec<(f64, f64)> {
    (0..n)
        .map(|i| {
            let mut x = (std::f64::consts::PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
            let mut derivative = 1.;
            for _ in 0..64 {
                let (mut p0, mut p1) = (1., x);
                for k in 2..=n {
                    let k = k as f64;
                    (p0, p1) = (p1, ((2. * k - 1.) * x * p1 - (k - 1.) * p0) / k);
                }
                derivative = n as f64 * (x * p1 - p0) / (x * x - 1.);
                x -= p1 / derivative;
            }
            ((1. - x) / 2., 1. / ((1. - x * x) * derivative * derivative))
        })
        .collect()
}

/// Bézier derivative poles with respect to the span parameter.
fn hodograph(poles: &[Point]) -> Vec<Point> {
    let n = (poles.len() - 1) as f64;
    poles
        .windows(2)
        .map(|w| std::array::from_fn(|k| n * (w[1][k] - w[0][k])))
        .collect()
}

enum Curve {
    Line,
    Circle {
        center: Point,
        normal: Point,
        radius: f64,
    },
    /// Equal C⁰ Bézier spans sharing their end poles.
    Spline {
        degree: usize,
        poles: Vec<Point>,
    },
}

struct Decoded {
    ends: [Point; 2],
    curve: Curve,
}

/// Positive sweep about `normal` from one end to the other.
fn sweep(ends: [Point; 2], center: Point, normal: Point) -> f64 {
    let [a, b] = ends.map(|p| sub(p, center));
    dot(cross(a, b), normal)
        .atan2(dot(a, b))
        .rem_euclid(std::f64::consts::TAU)
}

impl Decoded {
    /// Five points along an analytic edge; spline edges have none here
    /// because their faces prove them.
    fn samples(&self) -> Vec<Point> {
        let [a, b] = self.ends;
        let at = |t: f64| match &self.curve {
            Curve::Line => Some(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)),
            Curve::Circle {
                center,
                normal,
                radius,
            } => {
                let e1 = sub(a, *center).map(|x| x / radius);
                let e2 = cross(*normal, e1);
                let (s, c) = (sweep(self.ends, *center, *normal) * t).sin_cos();
                Some(std::array::from_fn(|i| {
                    center[i] + radius * (c * e1[i] + s * e2[i])
                }))
            }
            Curve::Spline { .. } => None,
        };
        [0., 0.25, 0.5, 0.75, 1.]
            .into_iter()
            .filter_map(at)
            .collect()
    }

    /// `∫ (x dy - y dx) / 2` along the edge in the local axial projection.
    fn green(&self) -> f64 {
        let [a, b] = self.ends;
        match &self.curve {
            Curve::Line => (a[0] * b[1] - a[1] * b[0]) / 2.,
            Curve::Circle {
                center,
                normal,
                radius,
            } => {
                let sweep = sweep(self.ends, *center, *normal);
                (radius * radius * sweep * normal[2].signum() + center[0] * (b[1] - a[1])
                    - center[1] * (b[0] - a[0]))
                    / 2.
            }
            Curve::Spline { degree, poles } => {
                // The integrand has degree 2·degree - 1 per span.
                let nodes = gauss(*degree + 1);
                poles
                    .windows(degree + 1)
                    .step_by(*degree)
                    .map(|span| {
                        let velocity = hodograph(span);
                        nodes
                            .iter()
                            .map(|&(t, w)| {
                                let p = bezier(span, t);
                                let d = bezier(&velocity, t);
                                w * (p[0] * d[1] - p[1] * d[0]) / 2.
                            })
                            .sum::<f64>()
                    })
                    .sum()
            }
        }
    }
}

/// Check STEP knot lists of equal C⁰ Bézier spans of one degree on
/// `[0, 1]` and return the span count.
fn bezier_spans(degree: usize, mult: &[Value<'_>], values: &[Value<'_>]) -> Result<usize> {
    let spans = mult.len().saturating_sub(1);
    require(spans >= 1 && values.len() == mult.len())?;
    for (s, (m, k)) in mult.iter().zip(values).enumerate() {
        let expected = if s == 0 || s == spans {
            degree + 1
        } else {
            degree
        };
        let fits = matches!((m, k), (Value::Number(m), Value::Number(k))
            if *m == expected as f64 && *k == s as f64 / spans as f64);
        require(fits)?;
    }
    Ok(spans)
}

fn decode(shell: &mut Shell<'_, '_>, frame: &Frame, id: u32) -> Result<Decoded> {
    let [Value::String(_), a, b, curve, Value::Enum("T")] = shell.args(id, "EDGE_CURVE")? else {
        return Err(ReadError::Invalid);
    };
    let (a, b, curve) = (reference(a)?, reference(b)?, reference(curve)?);
    let ends = [shell.vertex(a)?, shell.vertex(b)?].map(|p| frame.local(p));
    let curve = match shell.simple(curve)?.name {
        "LINE" => Curve::Line,
        "CIRCLE" => {
            let [Value::String(_), axis, Value::Number(radius)] = shell.args(curve, "CIRCLE")?
            else {
                return Err(ReadError::Invalid);
            };
            let placement = shell.placement(reference(axis)?)?;
            Curve::Circle {
                center: frame.local(placement.origin),
                normal: frame.direction(placement.normal),
                radius: *radius,
            }
        }
        _ => {
            let [
                Value::String(_),
                Value::Number(degree),
                controls,
                Value::Enum("UNSPECIFIED"),
                Value::Enum("F"),
                Value::Enum("F"),
                Value::List(mult),
                Value::List(values),
                Value::Enum("UNSPECIFIED"),
            ] = shell.args(curve, "B_SPLINE_CURVE_WITH_KNOTS")?
            else {
                return Err(ReadError::Invalid);
            };
            require((1. ..=25.).contains(degree) && degree.fract() == 0.)?;
            let degree = *degree as usize;
            let spans = bezier_spans(degree, mult, values)?;
            let poles = refs(controls)?
                .into_iter()
                .map(|p| shell.point(p).map(|p| frame.local(p)))
                .collect::<Result<Vec<_>>>()?;
            let ends_match = poles.len() == spans * degree + 1
                && poles[0] == ends[0]
                && poles[poles.len() - 1] == ends[1];
            require(ends_match)?;
            Curve::Spline { degree, poles }
        }
    };
    Ok(Decoded { ends, curve })
}

/// One decoded spline face: poles `[i][j]` in local mm.
struct Patch {
    degree: usize,
    spans: usize,
    poles: Vec<Vec<Point>>,
}

impl Patch {
    /// Boundary pole lists: start and end heights, then start and end
    /// profile columns.
    fn boundaries(&self) -> [Vec<Point>; 4] {
        let last = self.spans * V_DEGREE;
        [
            self.poles.iter().map(|row| row[0]).collect(),
            self.poles.iter().map(|row| row[last]).collect(),
            self.poles[0].clone(),
            self.poles[self.degree].clone(),
        ]
    }

    /// Exact flux of `(0, 0, z)` through the decoded surface. The integrand
    /// has degree at most `3n - 1` in each span parameter.
    fn flux(&self) -> f64 {
        let along = gauss((3 * self.degree).div_ceil(2) + 1);
        let across = gauss((3 * V_DEGREE).div_ceil(2) + 1);
        let mut total = 0.;
        for s in 0..self.spans {
            let first = s * V_DEGREE;
            for &(u, wu) in &along {
                let (values, rates): (Vec<Point>, Vec<Point>) = (first..=first + V_DEGREE)
                    .map(|j| {
                        let column: Vec<Point> = self.poles.iter().map(|row| row[j]).collect();
                        (bezier(&column, u), bezier(&hodograph(&column), u))
                    })
                    .unzip();
                let lean = hodograph(&values);
                for &(t, wv) in &across {
                    let p = bezier(&values, t);
                    let du = bezier(&rates, t);
                    let dv = bezier(&lean, t);
                    total += wu * wv * p[2] * (du[0] * dv[1] - du[1] * dv[0]);
                }
            }
        }
        total
    }
}

fn surface_patch(shell: &Shell<'_, '_>, frame: &Frame, id: u32) -> Result<Patch> {
    let [
        Value::String(_),
        Value::Number(du),
        Value::Number(dv),
        Value::List(rows),
        Value::Enum("UNSPECIFIED"),
        Value::Enum("F"),
        Value::Enum("F"),
        Value::Enum("F"),
        Value::List(um),
        Value::List(vm),
        Value::List(uk),
        Value::List(vk),
        Value::Enum("UNSPECIFIED"),
    ] = shell.args(id, "B_SPLINE_SURFACE_WITH_KNOTS")?
    else {
        return Err(ReadError::Invalid);
    };
    require((1. ..=25.).contains(du) && du.fract() == 0. && *dv == V_DEGREE as f64)?;
    let degree = *du as usize;
    require(bezier_spans(degree, um, uk)? == 1)?;
    let spans = bezier_spans(V_DEGREE, vm, vk)?;
    let poles = rows
        .iter()
        .map(|row| {
            refs(row)?
                .into_iter()
                .map(|p| shell.point(p).map(|p| frame.local(p)))
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let width = spans * V_DEGREE + 1;
    require(poles.len() == degree + 1 && poles.iter().all(|row| row.len() == width))?;
    Ok(Patch {
        degree,
        spans,
        poles,
    })
}

pub(super) fn verify_wheel_member(
    shell: &mut Shell<'_, '_>,
    solid: u32,
    wheel: &Wheel,
    [origin, u, v]: [Point; 3],
    tolerance: f64,
) -> Result<f64> {
    require(
        origin.into_iter().chain(u).chain(v).all(f64::is_finite)
            && (dot(u, u) - 1.).abs() <= 1e-10
            && (dot(v, v) - 1.).abs() <= 1e-10
            && dot(u, v).abs() <= 1e-10
            && tolerance > 0.,
    )?;
    let frame = Frame {
        origin,
        axes: [u, v, cross(u, v)],
    };
    let topology = wheel.topology();
    let teeth = wheel.teeth();
    let m = wheel.module_mm();
    let h = wheel.half_width() * m;
    let blank = (f64::from(teeth) / 2. + 1.) * m;
    let pitch = std::f64::consts::TAU / f64::from(teeth);
    // Rounding of the placement transform at this model size.
    let placement = 64. * f64::EPSILON * (length(origin) + blank + h);
    let faces = shell.member_faces(solid)?;
    require(faces.len() == topology.faces.len())?;
    let mut uses: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut vertices = BTreeSet::new();
    let mut decoded: BTreeMap<u32, Decoded> = BTreeMap::new();
    let mut patches = BTreeSet::new();
    let mut flux = 0.;
    for face_id in faces {
        let (surface, forward, bounds) = face(shell, face_id)?;
        let mut loops = vec![];
        for (i, &bound) in bounds.iter().enumerate() {
            loops.push(ring(shell, bound, i == 0)?);
        }
        let mut green = 0.;
        for &(edge, same) in loops.iter().flatten() {
            let entry = uses.entry(edge).or_default();
            if same {
                entry.0 += 1;
            } else {
                entry.1 += 1;
            }
            if let std::collections::btree_map::Entry::Vacant(slot) = decoded.entry(edge) {
                let curve = decode(shell, &frame, edge)?;
                for p in curve.samples() {
                    require(wheel.classify(p, tolerance) == Containment::Boundary)?;
                }
                for end in curve.ends {
                    vertices.insert(end.map(f64::to_bits));
                }
                slot.insert(curve);
            }
            green += if same { 1. } else { -1. } * decoded[&edge].green();
        }
        let edges: Vec<&Decoded> = loops.iter().flatten().map(|(e, _)| &decoded[e]).collect();
        let splines = edges.iter().filter_map(|edge| match &edge.curve {
            Curve::Spline { poles, .. } => Some(poles),
            _ => None,
        });
        match shell.simple(surface)?.name {
            "PLANE" => {
                let [Value::String(_), axis] = shell.args(surface, "PLANE")? else {
                    return Err(ReadError::Invalid);
                };
                let axis = shell.placement(reference(axis)?)?;
                let normal = frame.direction(axis.normal)[2];
                let z = frame.local(axis.origin)[2];
                // The loop runs counterclockwise about the outward normal,
                // and every spline edge lies in the plane.
                let end_plane = forward
                    && loops.len() == 1
                    && normal.abs() > 1. - 1e-12
                    && (z.abs() - h).abs() <= tolerance
                    && z * normal > 0.
                    && green * normal > 0.;
                require(end_plane)?;
                for poles in splines {
                    require(poles.iter().all(|p| (p[2] - z).abs() <= tolerance))?;
                }
                flux += z * green;
            }
            "CYLINDRICAL_SURFACE" => {
                let [Value::String(_), axis, Value::Number(radius)] =
                    shell.args(surface, "CYLINDRICAL_SURFACE")?
                else {
                    return Err(ReadError::Invalid);
                };
                let axis = shell.placement(reference(axis)?)?;
                let center = frame.local(axis.origin);
                let (expected, count) = if forward {
                    (blank, 1)
                } else {
                    (wheel.bore_radius_mm().ok_or(ReadError::Invalid)?, 2)
                };
                let coaxial = center[0].abs() <= tolerance
                    && center[1].abs() <= tolerance
                    && frame.direction(axis.normal)[2] > 1. - 1e-12
                    && (radius - expected).abs() <= tolerance
                    && loops.len() == count;
                require(coaxial)?;
            }
            _ => {
                let patch = surface_patch(shell, &frame, surface)?;
                require(forward && loops.len() == 1)?;
                // Identify the sector and piece from the corner poles.
                let corner = patch.poles[0][0];
                let sector = (corner[1].atan2(corner[0]) / pitch).round();
                let sector = (sector as i64).rem_euclid(i64::from(teeth)) as u32;
                let joint = |j: usize| topology.vertices_mm[sector as usize * 14 + j];
                let near = |a: Point, b: Point| length(sub(a, b)) <= tolerance;
                let end = patch.poles[patch.degree][0];
                let piece = (0..5)
                    .find(|&p| near(corner, joint(p + 1)) && near(end, joint(p + 2)))
                    .ok_or(ReadError::Invalid)?;
                require(patches.insert((sector, piece)))?;
                let angle = wheel.sector_angle(sector);
                let chart = Chart {
                    degree: patch.degree,
                    poles: patch
                        .poles
                        .iter()
                        .map(|row| row.iter().map(|&p| rotate_z(p, -angle)).collect())
                        .collect(),
                };
                let heights = wheel.chart_heights(patch.spans);
                let nodes = wheel.chart_nodes(piece, patch.degree, &heights);
                let residual = chart_residual(&chart, patch.spans, &nodes) + placement;
                let remainder = wheel
                    .chart_remainder_mm(piece, patch.degree, patch.spans)
                    .ok_or(ReadError::Limit)?;
                let bound = remainder + lebesgue(patch.degree) * lebesgue(V_DEGREE) * residual;
                require(bound <= tolerance)?;
                // Every spline edge is one boundary curve of this face.
                let boundaries = patch.boundaries();
                for poles in splines {
                    let reversed: Vec<Point> = poles.iter().rev().copied().collect();
                    require(boundaries.iter().any(|b| b == poles || *b == reversed))?;
                }
                flux += patch.flux();
            }
        }
    }
    let manifold = uses.len() == topology.edges.len()
        && uses.values().all(|&(a, b)| a == 1 && b == 1)
        && vertices.len() == topology.vertices_mm.len()
        && patches.len() == 5 * teeth as usize;
    require(manifold && flux.is_finite() && flux > 0.)?;
    Ok(flux)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        step::{supports_worm_wheel, write_worm_wheel},
        worm_gear::{WheelSpec, wheel::tests::spec},
    };

    #[test]
    fn decoded_wheels_reproduce_the_exact_volume_at_tilted_placements() {
        let frame = ([10., -20., 30.], [0.8, 0., -0.6], [0., 1., 0.]);
        for (teeth, bore, tolerance) in [(32, 0., 1e-6), (40, 4., 1e-7)] {
            let wheel = Wheel::new(spec(1., teeth, 1.5, bore, 0.05), 1e-3, 1e7).unwrap();
            let started = std::time::Instant::now();
            let text = write_worm_wheel(&wheel, frame.0, frame.1, frame.2, tolerance).unwrap();
            let written = started.elapsed();
            let volume =
                verify_worm_wheel(&text, &wheel, frame.0, frame.1, frame.2, tolerance).unwrap();
            eprintln!(
                "{} bytes write {written:?} verify {:?}",
                text.len(),
                started.elapsed()
            );
            let expected = wheel.volume_mm3();
            assert!(
                (volume - expected).abs() / expected < 1e-8,
                "{volume} {expected}"
            );
            let other = Wheel::new(
                WheelSpec {
                    pitch_thinning_mm: 0.04,
                    ..wheel.spec()
                },
                1e-3,
                1e7,
            )
            .unwrap();
            assert!(
                verify_worm_wheel(&text, &other, frame.0, frame.1, frame.2, tolerance).is_err()
            );
            assert_eq!(
                verify_worm_wheel(&text, &wheel, frame.0, frame.1, frame.1, tolerance),
                Err(ReadError::Invalid)
            );
        }
    }

    /// Rewrite the first line declaring `kind` with `edit`.
    fn mutate(text: &str, kind: &str, edit: impl Fn(&str) -> String) -> String {
        let marker = format!("={kind}(");
        let mut done = false;
        text.lines()
            .map(|line| {
                if !done && line.contains(&marker) {
                    done = true;
                    edit(line)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn widened(line: &str) -> String {
        line.replacen(");", ",0.);", 1)
    }

    #[test]
    fn malformed_wheel_entities_are_refused() {
        let frame = ([1., 2., 3.], [1., 0., 0.], [0., 1., 0.]);
        let wheel = Wheel::new(spec(1., 32, 1., 3., 0.05), 1e-3, 1e7).unwrap();
        let text = write_worm_wheel(&wheel, frame.0, frame.1, frame.2, 1e-5).unwrap();
        assert!(verify_worm_wheel(&text, &wheel, frame.0, frame.1, frame.2, 1e-5).is_ok());
        type Edit = fn(&str) -> String;
        let cases: [(&str, Edit); 8] = [
            ("EDGE_CURVE", |l| l.replacen(".T.);", ".F.);", 1)),
            ("CIRCLE", widened),
            ("B_SPLINE_CURVE_WITH_KNOTS", widened),
            ("B_SPLINE_CURVE_WITH_KNOTS", |l| {
                l.replacen("KNOTS('',", "KNOTS('',1", 1)
            }),
            ("B_SPLINE_SURFACE_WITH_KNOTS", widened),
            ("PLANE", widened),
            ("CYLINDRICAL_SURFACE", widened),
            ("CYLINDRICAL_SURFACE", |l| l.replacen("E1)", "E2)", 1)),
        ];
        for (kind, edit) in cases {
            let broken = mutate(&text, kind, edit);
            assert_ne!(broken, text, "{kind}");
            assert!(
                verify_worm_wheel(&broken, &wheel, frame.0, frame.1, frame.2, 1e-5).is_err(),
                "{kind}"
            );
        }
        // Writer admission follows the placement and the tolerance.
        assert!(supports_worm_wheel(&wheel, frame.0, frame.1, frame.2, 1e-5));
        assert!(!supports_worm_wheel(
            &wheel, frame.0, frame.1, frame.1, 1e-5
        ));
        assert!(!supports_worm_wheel(
            &wheel,
            frame.0,
            frame.1,
            frame.2,
            f64::NAN
        ));
        assert!(!supports_worm_wheel(
            &wheel, frame.0, frame.1, frame.2, 1e-300
        ));
        assert_eq!(wheel.clone(), wheel);
    }
}
