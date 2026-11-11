//! Verify a decoded spherical bevel B-rep against its authored solid and
//! integrate the volume from the decoded boundary by the divergence theorem.
//!
//! Every side face is a cone through the pitch apex, so `p · n` vanishes on
//! it once that property is checked on the decoded surface. Only the two
//! back cones, where `p · n` equals the cone distance, and the bore remain.
use super::{
    helical::{face, require, ring},
    *,
};
use crate::bevel_gear::{Bevel, Containment, End, integrate};

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

enum Curve {
    Line,
    Circle {
        center: Point,
        normal: Point,
        radius: f64,
    },
    Spline {
        poles: Vec<Point>,
        knots: Vec<f64>,
    },
}

struct Decoded {
    ends: [Point; 2],
    curve: Curve,
}

impl Decoded {
    fn range(&self) -> [f64; 2] {
        match &self.curve {
            Curve::Spline { knots, .. } => [knots[3], knots[knots.len() - 4]],
            _ => [0., 1.],
        }
    }

    /// Local point at parameter `t` of `range()`.
    fn point(&self, t: f64) -> Point {
        let [a, b] = self.ends;
        match &self.curve {
            Curve::Line => std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t),
            Curve::Circle {
                center,
                normal,
                radius,
            } => {
                let e1 = scale(sub(a, *center), 1. / radius);
                let e2 = cross(*normal, e1);
                let sweep = sweep(self.ends, *center, *normal);
                let (s, c) = (sweep * t).sin_cos();
                std::array::from_fn(|i| center[i] + radius * (c * e1[i] + s * e2[i]))
            }
            Curve::Spline { poles, knots } => de_boor(poles, knots, t),
        }
    }

    /// ∫ (x dy - y dx) / 2 along the edge in the local axial projection.
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
            Curve::Spline { .. } => {
                let [lo, hi] = self.range();
                let h = 1e-7 * (hi - lo);
                integrate(
                    |t| {
                        let p = self.point(t);
                        let q = self.point((t + h).min(hi));
                        let r = self.point((t - h).max(lo));
                        let span = (t + h).min(hi) - (t - h).max(lo);
                        (p[0] * (q[1] - r[1]) - p[1] * (q[0] - r[0])) / (2. * span)
                    },
                    lo,
                    hi,
                    64,
                )
            }
        }
    }
}

/// Positive counterclockwise sweep about `normal` from one end to the other.
fn sweep(ends: [Point; 2], center: Point, normal: Point) -> f64 {
    let [a, b] = ends.map(|p| sub(p, center));
    dot(cross(a, b), normal)
        .atan2(dot(a, b))
        .rem_euclid(std::f64::consts::TAU)
}

fn scale(a: Point, s: f64) -> Point {
    a.map(|v| v * s)
}

fn de_boor(poles: &[Point], knots: &[f64], t: f64) -> Point {
    const P: usize = 3;
    let n = poles.len();
    let span = (P..n).rfind(|&k| knots[k] <= t).unwrap_or(P);
    let mut d: Vec<Point> = (0..=P).map(|j| poles[j + span - P]).collect();
    for r in 1..=P {
        for j in (r..=P).rev() {
            let i = j + span - P;
            let denominator = knots[i + P + 1 - r] - knots[i];
            // A repeated knot has a zero span and a zero numerator.
            let alpha = (t - knots[i]) / denominator.max(f64::MIN_POSITIVE);
            d[j] = std::array::from_fn(|k| (1. - alpha) * d[j - 1][k] + alpha * d[j][k]);
        }
    }
    d[P]
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
            require(*degree == 3. && mult.len() == values.len())?;
            let mut knots = vec![];
            for (m, k) in mult.iter().zip(values) {
                let (Value::Number(m), Value::Number(k)) = (m, k) else {
                    return Err(ReadError::Invalid);
                };
                require(*m >= 1. && *m <= 4. && m.fract() == 0.)?;
                knots.extend(std::iter::repeat_n(*k, *m as usize));
            }
            let poles = refs(controls)?
                .into_iter()
                .map(|p| shell.point(p).map(|p| frame.local(p)))
                .collect::<Result<Vec<_>>>()?;
            require(poles.len() >= 4 && knots.len() == poles.len() + 4)?;
            Curve::Spline { poles, knots }
        }
    };
    Ok(Decoded { ends, curve })
}

/// Verify one closed authored bevel and return its decoded volume in mm³.
pub fn verify_bevel(
    input: &str,
    gear: &Bevel,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let mut volumes =
        compound::read_members_by(input, tolerance, true, true, |shell, solid, _| {
            verify_member(shell, solid, gear, [origin, u, v], tolerance)
        })?;
    volumes.pop().ok_or(ReadError::Invalid)
}

/// Verify one decoded shell against the authored bevel at its placement.
pub(super) fn verify_member(
    shell: &mut Shell<'_, '_>,
    solid: u32,
    gear: &Bevel,
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
    let topology = gear.topology();
    let band = tolerance * 16.;
    let faces = shell.member_faces(solid)?;
    require(faces.len() == topology.faces.len())?;
    let mut uses: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut vertices = BTreeSet::new();
    let mut decoded = BTreeMap::new();
    let mut flux = 0.;
    for face_id in faces {
        let (surface, forward, bounds) = face(shell, face_id)?;
        let [bound] = bounds[..] else {
            return Err(ReadError::Invalid);
        };
        let edges = ring(shell, bound, true)?;
        let mut green = 0.;
        let mut heights = [f64::INFINITY, f64::NEG_INFINITY];
        for &(edge, same) in &edges {
            let entry = uses.entry(edge).or_default();
            if same {
                entry.0 += 1;
            } else {
                entry.1 += 1;
            }
            if let std::collections::btree_map::Entry::Vacant(slot) = decoded.entry(edge) {
                let curve = decode(shell, &frame, edge)?;
                let [lo, hi] = curve.range();
                for t in [0., 0.25, 0.5, 0.75, 1.] {
                    let p = curve.point(lo + (hi - lo) * t);
                    require(gear.classify(p, band) == Containment::Boundary)?;
                }
                for end in curve.ends {
                    vertices.insert(end.map(f64::to_bits));
                }
                slot.insert(curve);
            }
            let curve = &decoded[&edge];
            let sign = if same { 1. } else { -1. };
            green += sign * curve.green();
            for end in curve.ends {
                heights = [heights[0].min(end[2]), heights[1].max(end[2])];
            }
        }
        let entity = shell.simple(surface)?;
        match entity.name {
            "CONICAL_SURFACE" => {
                let [
                    Value::String(_),
                    axis,
                    Value::Number(radius),
                    Value::Number(semi),
                ] = shell.args(surface, "CONICAL_SURFACE")?
                else {
                    return Err(ReadError::Invalid);
                };
                let placement = shell.placement(reference(axis)?)?;
                let apex = frame.local(placement.origin);
                let axis = frame.direction(placement.normal);
                require(*radius == 0. && apex[0].abs() <= band && apex[1].abs() <= band)?;
                if apex[2].abs() > band {
                    // Back cone: p · n is the cone distance a cos δ.
                    require(axis[2] < -1. + 1e-12)?;
                    let delta = std::f64::consts::FRAC_PI_2 - semi;
                    flux += apex[2] * delta.cos() * green / delta.cos();
                } else {
                    require(axis[2] > 1. - 1e-12 && forward)?;
                }
            }
            "PLANE" => {
                let [Value::String(_), axis] = shell.args(surface, "PLANE")? else {
                    return Err(ReadError::Invalid);
                };
                let placement = shell.placement(reference(axis)?)?;
                let through = dot(
                    frame.direction(placement.normal),
                    frame.local(placement.origin),
                );
                require(through.abs() <= band && forward)?;
            }
            "CYLINDRICAL_SURFACE" => {
                let [Value::String(_), axis, Value::Number(radius)] =
                    shell.args(surface, "CYLINDRICAL_SURFACE")?
                else {
                    return Err(ReadError::Invalid);
                };
                let placement = shell.placement(reference(axis)?)?;
                let center = frame.local(placement.origin);
                let coaxial = !forward
                    && center[0].abs() <= band
                    && center[1].abs() <= band
                    && frame.direction(placement.normal)[2] > 1. - 1e-12;
                require(coaxial)?;
                let height = heights[1] - heights[0];
                flux -= radius * std::f64::consts::TAU * radius * height;
            }
            _ => {
                let [
                    Value::String(_),
                    Value::Number(du),
                    Value::Number(dv),
                    Value::List(rows),
                    ..,
                ] = shell.args(surface, "B_SPLINE_SURFACE_WITH_KNOTS")?
                else {
                    return Err(ReadError::Invalid);
                };
                require(*du == 3. && *dv == 1. && forward)?;
                let ratio = gear.end_distance_mm(End::Toe) / gear.end_distance_mm(End::Heel);
                for row in rows {
                    let [toe, heel] = refs(row)?[..] else {
                        return Err(ReadError::Invalid);
                    };
                    let [toe, heel] =
                        [shell.point(toe)?, shell.point(heel)?].map(|p| frame.local(p));
                    // Collinear with the apex: a cone through the apex.
                    require(length(sub(toe, scale(heel, ratio))) <= band)?;
                }
            }
        }
    }
    let manifold = uses.len() == topology.edges.len()
        && uses.values().all(|&(a, b)| a == 1 && b == 1)
        && vertices.len() == topology.vertices_mm.len();
    require(manifold)?;
    let volume = flux / 3.;
    require(volume.is_finite() && volume > 0.)?;
    Ok(volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bevel_gear::tests::spec, step::write_bevel};

    #[test]
    fn decoded_bevels_reproduce_the_exact_volume_at_tilted_placements() {
        let frame = ([10., -20., 30.], [0.8, 0., -0.6], [0., 1., 0.]);
        for (bore, teeth, mate) in [(4., 24, 32), (0., 32, 24), (12.5, 24, 24)] {
            let gear = Bevel::new(spec(2., teeth, mate, 6., bore, 0.2), 1e-3, 1e7).unwrap();
            let text = write_bevel(&gear, frame.0, frame.1, frame.2, 1e-6).unwrap();
            let volume = verify_bevel(&text, &gear, frame.0, frame.1, frame.2, 1e-6).unwrap();
            assert!(
                (volume - gear.volume_mm3()).abs() / volume < 1e-8,
                "{volume}"
            );
            // A different authored gear does not certify the same file.
            let other = Bevel::new(spec(2., teeth, mate, 5., bore, 0.2), 1e-3, 1e7).unwrap();
            assert!(verify_bevel(&text, &other, frame.0, frame.1, frame.2, 1e-6).is_err());
            assert_eq!(
                verify_bevel(&text, &gear, frame.0, frame.1, frame.1, 1e-6),
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

    /// Duplicate the first `(#a` reference list item of a line as `(#a,#a`.
    fn doubled(line: &str, prefix: &str) -> String {
        let start = line.find(prefix).unwrap() + prefix.len();
        let end = start + line[start..].find([',', ')']).unwrap();
        let id = &line[start..end];
        format!("{}{id},{}", &line[..start], &line[start..])
    }

    #[test]
    fn malformed_bevel_entities_are_refused() {
        let frame = ([1., 2., 3.], [1., 0., 0.], [0., 1., 0.]);
        let bored = Bevel::new(spec(2., 24, 32, 6., 4., 0.2), 1e-3, 1e7).unwrap();
        let meridian = Bevel::new(spec(1., 24, 64, 2., 0., 0.2), 1e-3, 1e7).unwrap();
        assert!(meridian.has_meridian());
        type Edit = fn(&str) -> String;
        let cases: [(&Bevel, &str, Edit); 10] = [
            (&bored, "EDGE_CURVE", |l| l.replacen(".T.);", ".F.);", 1)),
            (&bored, "CIRCLE", widened),
            (&bored, "B_SPLINE_CURVE_WITH_KNOTS", widened),
            (&bored, "B_SPLINE_CURVE_WITH_KNOTS", |l| {
                l.replacen(",(4,", ",($,", 1)
            }),
            (&bored, "ADVANCED_FACE", |l| doubled(l, "('',(")),
            (&bored, "CONICAL_SURFACE", widened),
            (&meridian, "PLANE", widened),
            (&bored, "CYLINDRICAL_SURFACE", widened),
            (&bored, "B_SPLINE_SURFACE_WITH_KNOTS", |l| {
                l.replacen("WITH_KNOTS('',3,", "WITH_KNOTS('',$,", 1)
            }),
            (&bored, "B_SPLINE_SURFACE_WITH_KNOTS", |l| doubled(l, ",((")),
        ];
        for (gear, kind, edit) in cases {
            let text = write_bevel(gear, frame.0, frame.1, frame.2, 1e-6).unwrap();
            assert!(verify_bevel(&text, gear, frame.0, frame.1, frame.2, 1e-6).is_ok());
            let broken = mutate(&text, kind, edit);
            assert_ne!(broken, text, "{kind}");
            assert!(
                verify_bevel(&broken, gear, frame.0, frame.1, frame.2, 1e-6).is_err(),
                "{kind}"
            );
        }
    }
}
