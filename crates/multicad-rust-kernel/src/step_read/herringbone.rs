//! Certify the decoded tensor-product B-rep of a known analytic chevron.
use super::{
    helical::{check_axis, face, orient, poles, require, ring, step_error},
    *,
};
use crate::{
    normal_sweep::Edge as PlanarEdge,
    spur_gear::herringbone::Herringbone,
    step::{
        helical::product,
        herringbone::{Chart, prepare},
    },
};
use std::f64::consts::PI;

struct Frame {
    origin: Point,
    u: Point,
    v: Point,
    normal: Point,
}

impl Frame {
    fn world(&self, [x, y]: Point2, z: f64) -> Point {
        std::array::from_fn(|i| self.origin[i] + self.u[i] * x + self.v[i] * y + self.normal[i] * z)
    }

    fn local(&self, world: Point) -> Point {
        let relative = sub(world, self.origin);
        [self.u, self.v, self.normal].map(|axis| dot(axis, relative))
    }
}

/// Verify the closed authored shell and every transmitted spline control.
/// The volume is integrated from the decoded radial section and axial chart.
pub fn verify_herringbone(
    input: &str,
    gear: &Herringbone,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    require(
        origin.into_iter().chain(u).chain(v).all(f64::is_finite)
            && (dot(u, u) - 1.).abs() <= 1e-10
            && (dot(v, v) - 1.).abs() <= 1e-10
            && dot(u, v).abs() <= 1e-10,
    )?;
    let frame = Frame {
        origin,
        u,
        v,
        normal: cross(u, v),
    };
    let plan = prepare(gear, origin, tolerance).map_err(step_error)?;
    let bore = gear.profile().bore().and_then(PlanarEdge::radius_mm);
    let n = plan.transverse.len();
    let result = compound::read_members_by(input, tolerance, true, true, |shell, solid, _| {
        let faces = shell.member_faces(solid)?;
        require(
            faces.len() == 2 * n + 2 + usize::from(bore.is_some())
                && faces.iter().collect::<BTreeSet<_>>().len() == faces.len(),
        )?;
        let mut rims = [(); 3].map(|()| vec![None; n]);
        let mut rails = [(); 2].map(|()| vec![None; n]);
        let mut integral = 0.;
        for band in 0..2 {
            let axial = &plan.spins[band];
            let heights = &plan.heights[band];
            for (index, radial) in plan.transverse.iter().enumerate() {
                let (support, forward, bounds) = face(shell, faces[band * n + index])?;
                let ([bound], true) = (bounds.as_slice(), forward) else {
                    return Err(ReadError::Invalid);
                };
                let decoded = surface(shell, support, radial, axial, heights, &frame)?;
                if index == 0 {
                    integral += rotation_integral(&decoded, radial.poles[0], axial, &frame)?;
                }
                let uses = ring(shell, *bound, true)?;
                let [(bottom, true), (next, true), (top, false), (prior, false)] = uses[..] else {
                    return Err(ReadError::Invalid);
                };
                let next_index = (index + 1) % n;
                register(&mut rims[band][index], bottom)?;
                register(&mut rims[band + 1][index], top)?;
                register(&mut rails[band][index], prior)?;
                register(&mut rails[band][next_index], next)?;
            }
        }
        let rims = resolved(rims)?;
        let rails = resolved(rails)?;
        let mut edge_ids: BTreeSet<_> = rims.iter().chain(&rails).flatten().copied().collect();
        require(edge_ids.len() == 5 * n)?;
        let mut vertices = [(); 3].map(|()| vec![None; n]);
        let mut area = 0.;
        for station in 0..3 {
            let spin = plan.phases[station];
            let z = plan.levels[station];
            for (index, radial) in plan.transverse.iter().enumerate() {
                let id = rims[station][index];
                let points = radial
                    .poles
                    .iter()
                    .map(|&p| frame.world(product(p, spin), z));
                let (ends, decoded, curve) = trim(shell, id, radial, points)?;
                register(&mut vertices[station][index], ends[0])?;
                register(&mut vertices[station][(index + 1) % n], ends[1])?;
                if station == 0 {
                    area += section_area(shell, curve, radial, &decoded, &frame)?;
                }
            }
        }
        let vertices = resolved(vertices)?;
        let mut vertex_ids: BTreeSet<_> = vertices.iter().flatten().copied().collect();
        require(vertex_ids.len() == 3 * n)?;
        for band in 0..2 {
            let axial = &plan.spins[band];
            for (index, radial) in plan.transverse.iter().enumerate() {
                let points = axial
                    .poles
                    .iter()
                    .zip(&plan.heights[band])
                    .map(|(&spin, &z)| frame.world(product(radial.poles[0], spin), z));
                let (ends, _, _) = trim(shell, rails[band][index], axial, points)?;
                require(ends == [vertices[band][index], vertices[band + 1][index]])?;
            }
        }
        let mut bore_rims = [0; 2];
        for (end, station) in [0, 2].into_iter().enumerate() {
            let top = end == 1;
            let (plane, outward, bounds) = face(shell, faces[2 * n + end])?;
            require(outward && bounds.len() == 1 + usize::from(bore.is_some()))?;
            let [Value::String(_), placement] = shell.args(plane, "PLANE")? else {
                return Err(ReadError::Invalid);
            };
            let z = plan.levels[station];
            let center = frame.world([0.; 2], z);
            check_axis(
                shell,
                placement,
                center,
                frame.normal.map(|x| if top { x } else { -x }),
                frame.u,
            )?;
            let outer = ring(shell, bounds[0], true)?;
            let expected = rims[station]
                .iter()
                .copied()
                .map(|id| (id, true))
                .collect::<Vec<_>>();
            require(orient(&outer, top) == expected)?;
            if let Some(radius) = bore {
                let uses = ring(shell, bounds[1], false)?;
                let [(id, sense)] = uses[..] else {
                    return Err(ReadError::Invalid);
                };
                require(sense != top && edge_ids.insert(id))?;
                let [a, b] = shell.edge(id)?;
                let Curve::Circle(circle) = shell.edges[&id].curve else {
                    return Err(ReadError::Invalid);
                };
                // A circle's seam is defined relative to its serialized axis
                // center, which is already placed in the end plane.
                let rim_vertex = std::array::from_fn(|i| center[i] + radius * frame.u[i]);
                require(
                    a == b
                        && vertex_ids.insert(a)
                        && circle.center_mm == center
                        && circle.radius_mm == radius
                        && length(sub(circle.normal, frame.normal)) < 1e-10
                        && shell.vertex(a)? == rim_vertex,
                )?;
                bore_rims[end] = id;
            }
        }
        let void = if let Some(radius) = bore {
            let height = bore_volume(
                shell,
                faces[2 * n + 2],
                bore_rims,
                radius,
                &frame,
                &mut edge_ids,
            )?;
            PI * radius * radius * height
        } else {
            0.
        };
        let volume = area * integral - void;
        require(volume.is_finite() && volume > 0.)?;
        Ok(volume)
    })?;
    result.into_iter().next().ok_or(ReadError::Invalid)
}

fn register(slot: &mut Option<u32>, id: u32) -> Result<()> {
    require(slot.is_none_or(|old| old == id))?;
    *slot = Some(id);
    Ok(())
}

fn resolved<const N: usize>(values: [Vec<Option<u32>>; N]) -> Result<[Vec<u32>; N]> {
    let mut resolved = [(); N].map(|()| Vec::new());
    for (dest, src) in resolved.iter_mut().zip(values) {
        *dest = src
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(ReadError::Invalid)?;
    }
    Ok(resolved)
}

fn list<'a, 'data>(value: &'a Value<'data>) -> Result<&'a [Value<'data>]> {
    match value {
        Value::List(items) => Ok(items),
        _ => Err(ReadError::Invalid),
    }
}

fn complex<'a, 'data>(
    shell: &'a Shell<'_, 'data>,
    id: u32,
    names: [&str; 7],
) -> Result<[&'a [Value<'data>]; 7]> {
    let components: &[Entity<'data>; 7] = shell
        .components(id)?
        .try_into()
        .map_err(|_| ReadError::Invalid)?;
    let parts = components.each_ref();
    require(parts.map(|part| part.name) == names)?;
    Ok(parts.map(|part| part.args.as_slice()))
}

fn knots(mult: &Value<'_>, knots: &Value<'_>, expected: &Chart) -> Result<()> {
    let [mult, knots] = [list(mult)?, list(knots)?];
    require(mult.len() == expected.knots.len() && knots.len() == mult.len())?;
    for (i, ((degree, knot), &parameter)) in mult.iter().zip(knots).zip(&expected.knots).enumerate()
    {
        let multiplicity = if i == 0 || i + 1 == mult.len() {
            expected.degree + 1
        } else {
            expected.interior_mult
        };
        require(
            *degree == Value::Number(multiplicity as f64) && *knot == Value::Number(parameter),
        )?;
    }
    Ok(())
}

fn weights(value: &Value<'_>, expected: impl IntoIterator<Item = f64>) -> Result<()> {
    let values = list(value)?;
    let mut count = 0;
    for (i, w) in expected.into_iter().enumerate() {
        require(values.get(i) == Some(&Value::Number(w)))?;
        count += 1;
    }
    require(count == values.len())
}

fn surface(
    shell: &Shell<'_, '_>,
    id: u32,
    radial: &Chart,
    axial: &Chart,
    heights: &[f64],
    frame: &Frame,
) -> Result<Vec<Point>> {
    let grid = if let Some(expected) = &radial.weights {
        let parts = complex(
            shell,
            id,
            [
                "BOUNDED_SURFACE",
                "B_SPLINE_SURFACE",
                "B_SPLINE_SURFACE_WITH_KNOTS",
                "GEOMETRIC_REPRESENTATION_ITEM",
                "RATIONAL_B_SPLINE_SURFACE",
                "REPRESENTATION_ITEM",
                "SURFACE",
            ],
        )?;
        let [
            [],
            [
                Value::Number(du),
                Value::Number(dv),
                grid,
                Value::Enum("UNSPECIFIED"),
                Value::Enum("F"),
                Value::Enum("F"),
                Value::Enum("F"),
            ],
            [um, vm, uk, vk, Value::Enum("UNSPECIFIED")],
            [],
            [weight_rows],
            [Value::String(_)],
            [],
        ] = parts
        else {
            return Err(ReadError::Invalid);
        };
        let rows = list(weight_rows)?;
        require(rows.len() == expected.len())?;
        for (row, &w) in rows.iter().zip(expected) {
            weights(row, std::iter::repeat_n(w, axial.poles.len()))?;
        }
        surface_parameters([*du, *dv], [um, uk, vm, vk], radial, axial)?;
        grid
    } else {
        let [
            Value::String(_),
            Value::Number(du),
            Value::Number(dv),
            grid,
            Value::Enum("UNSPECIFIED"),
            Value::Enum("F"),
            Value::Enum("F"),
            Value::Enum("F"),
            um,
            vm,
            uk,
            vk,
            Value::Enum("UNSPECIFIED"),
        ] = shell.args(id, "B_SPLINE_SURFACE_WITH_KNOTS")?
        else {
            return Err(ReadError::Invalid);
        };
        surface_parameters([*du, *dv], [um, uk, vm, vk], radial, axial)?;
        grid
    };
    let rows = list(grid)?;
    require(rows.len() == radial.poles.len())?;
    let mut first_row = Vec::new();
    for (i, (row, &point)) in rows.iter().zip(&radial.poles).enumerate() {
        let decoded = poles(
            shell,
            row,
            axial
                .poles
                .iter()
                .zip(heights)
                .map(|(&spin, &z)| frame.world(product(point, spin), z)),
        )?;
        if i == 0 {
            first_row = decoded;
        }
    }
    Ok(first_row)
}

fn surface_parameters(
    [du, dv]: [f64; 2],
    [um, uk, vm, vk]: [&Value<'_>; 4],
    radial: &Chart,
    axial: &Chart,
) -> Result<()> {
    require(du == radial.degree as f64 && dv == axial.degree as f64)?;
    knots(um, uk, radial)?;
    knots(vm, vk, axial)
}

fn trim(
    shell: &mut Shell<'_, '_>,
    id: u32,
    chart: &Chart,
    mut expected: impl Iterator<Item = Point>,
) -> Result<([u32; 2], Vec<Point>, u32)> {
    let [Value::String(_), a, b, curve, Value::Enum("T")] = shell.args(id, "EDGE_CURVE")? else {
        return Err(ReadError::Invalid);
    };
    let ends = [reference(a)?, reference(b)?];
    let curve = reference(curve)?;
    require(ends[0] != ends[1])?;
    let actual = [shell.vertex(ends[0])?, shell.vertex(ends[1])?];
    require(actual[0] != actual[1])?;
    if chart.degree == 1 {
        shell.edge(id)?;
        require(matches!(shell.edges[&id].curve, Curve::Line) && expected.eq(actual))?;
        return Ok((ends, actual.into(), curve));
    }
    let (degree, controls, mult, parameters) = if let Some(expected_weights) = &chart.weights {
        let parts = complex(
            shell,
            curve,
            [
                "BOUNDED_CURVE",
                "B_SPLINE_CURVE",
                "B_SPLINE_CURVE_WITH_KNOTS",
                "CURVE",
                "GEOMETRIC_REPRESENTATION_ITEM",
                "RATIONAL_B_SPLINE_CURVE",
                "REPRESENTATION_ITEM",
            ],
        )?;
        let [
            [],
            [
                Value::Number(degree),
                control,
                Value::Enum("UNSPECIFIED"),
                Value::Enum("F"),
                Value::Enum("F"),
            ],
            [mult, parameters, Value::Enum("UNSPECIFIED")],
            [],
            [],
            [data],
            [Value::String(_)],
        ] = parts
        else {
            return Err(ReadError::Invalid);
        };
        weights(data, expected_weights.iter().copied())?;
        (*degree, control, mult, parameters)
    } else {
        let [
            Value::String(_),
            Value::Number(degree),
            controls,
            Value::Enum("UNSPECIFIED"),
            Value::Enum("F"),
            Value::Enum("F"),
            mult,
            parameters,
            Value::Enum("UNSPECIFIED"),
        ] = shell.args(curve, "B_SPLINE_CURVE_WITH_KNOTS")?
        else {
            return Err(ReadError::Invalid);
        };
        (*degree, controls, mult, parameters)
    };
    require(degree == chart.degree as f64)?;
    knots(mult, parameters, chart)?;
    let points = poles(shell, controls, &mut expected)?;
    require(points[0] == actual[0] && points[points.len() - 1] == actual[1])?;
    Ok((ends, points, curve))
}

fn section_area(
    shell: &Shell<'_, '_>,
    curve: u32,
    chart: &Chart,
    points: &[Point],
    frame: &Frame,
) -> Result<f64> {
    if chart.degree == 3 {
        return Ok(shell
            .spline_curve(curve)?
            .signed_area(frame.origin, frame.normal));
    }
    let cross2 = |a: Point2, b: Point2| a[0] * b[1] - a[1] * b[0];
    let project = |p| {
        let [x, y, _] = frame.local(p);
        [x, y]
    };
    if let Some(weights) = &chart.weights {
        let mut area = 0.;
        for (points, weights) in points
            .windows(3)
            .step_by(2)
            .zip(weights.windows(3).step_by(2))
        {
            let [a, b, c] = [points[0], points[1], points[2]].map(project);
            let w = weights[1];
            require((0. ..1.).contains(&w))?;
            let ab = std::array::from_fn(|i| b[i] - a[i]);
            let ac = std::array::from_fn(|i| c[i] - a[i]);
            let signed = cross2(ab, ac);
            require(signed != 0.)?;
            let radius = ac[0].hypot(ac[1]) / (2. * (1. - w * w).sqrt());
            let angle = 2. * w.acos();
            let segment = if angle < 0.01 {
                let a2 = angle * angle;
                angle * a2 / 6. * (1. - a2 / 20. + a2 * a2 / 840.)
            } else {
                angle - angle.sin()
            };
            area += (cross2(a, c) + signed.signum() * radius * radius * segment) / 2.;
        }
        return Ok(area);
    }
    Ok(cross2(project(points[0]), project(points[1])) / 2.)
}

fn binomial(n: usize, k: usize) -> f64 {
    (1..=k.min(n - k)).fold(1., |value, i| value * (n + 1 - i) as f64 / i as f64)
}

fn rotation_integral(
    points: &[Point],
    reference: Point2,
    axial: &Chart,
    frame: &Frame,
) -> Result<f64> {
    let [x, y] = reference;
    let norm_squared = x * x + y * y;
    require(norm_squared > 0.)?;
    let inverse = [x / norm_squared, -y / norm_squared];
    let degree = axial.degree;
    let mut result = 0.;
    for segment in points.windows(degree + 1).step_by(degree) {
        let local = segment
            .iter()
            .map(|&point| frame.local(point))
            .collect::<Vec<_>>();
        let duration = local[degree][2] - local[0][2];
        require(duration > 0.)?;
        let poles = local
            .into_iter()
            .map(|[x, y, _]| product([x, y], inverse))
            .collect::<Vec<_>>();
        // Integral of two degree-d Bernstein basis polynomials on [0,1].
        for (i, a) in poles.iter().enumerate() {
            for (j, b) in poles.iter().enumerate() {
                let weight = binomial(degree, i) * binomial(degree, j)
                    / ((2 * degree + 1) as f64 * binomial(2 * degree, i + j));
                result += duration * weight * (a[0] * b[0] + a[1] * b[1]);
            }
        }
    }
    Ok(result)
}

fn bore_volume(
    shell: &mut Shell<'_, '_>,
    face_id: u32,
    rims: [u32; 2],
    radius: f64,
    frame: &Frame,
    edges: &mut BTreeSet<u32>,
) -> Result<f64> {
    let (surface, forward, bounds) = face(shell, face_id)?;
    let ([bound], false) = (bounds.as_slice(), forward) else {
        return Err(ReadError::Invalid);
    };
    let [Value::String(_), axis, Value::Number(actual)] =
        shell.args(surface, "CYLINDRICAL_SURFACE")?
    else {
        return Err(ReadError::Invalid);
    };
    require(*actual == radius)?;
    check_axis(shell, axis, frame.origin, frame.normal, frame.u)?;
    let uses = ring(shell, *bound, true)?;
    let [
        (bottom, false),
        (seam, true),
        (top, true),
        (repeated, false),
    ] = uses[..]
    else {
        return Err(ReadError::Invalid);
    };
    require([bottom, top] == rims && seam == repeated && edges.insert(seam))?;
    let ends = shell.edge(seam)?;
    require(
        matches!(shell.edges[&seam].curve, Curve::Line)
            && ends == rims.map(|id| shell.edges[&id].vertices[0]),
    )?;
    let [a, b] = [shell.vertex(ends[0])?, shell.vertex(ends[1])?];
    Ok(dot(sub(b, a), frame.normal))
}

#[cfg(test)]
mod tests;
