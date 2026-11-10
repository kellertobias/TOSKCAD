//! Verify one or two coaxial end extensions and their continuous bore.
use super::*;
use std::f64::consts::PI;

#[derive(Clone, Copy)]
struct CircleTrim<'a> {
    edge: &'a Traversal,
    circle: CircleEdge,
}

impl CircleTrim<'_> {
    fn area(self) -> f64 {
        PI * self.circle.radius_mm.powi(2)
    }
}

struct PeriodicWall<'a> {
    rims: [CircleTrim<'a>; 2],
    outward: bool,
}

impl PeriodicWall<'_> {
    fn delta(&self) -> Point {
        sub(self.rims[1].circle.center_mm, self.rims[0].circle.center_mm)
    }
}

struct End<'a> {
    bore: Option<CircleTrim<'a>>,
    extension: Option<PeriodicWall<'a>>,
    tooth_area: f64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn verify(
    reader: &Shell<'_, '_>,
    edges: &Edges,
    gear: [(&Plane, &FaceWire); 2],
    areas: [f64; 2],
    extensions: &[Option<(Plane, FaceWire)>; 2],
    walls: &[FaceWire],
    axis: Point,
    gear_height: f64,
) -> Result<(f64, BTreeSet<u32>)> {
    let tolerance = reader.tolerance;
    let end = |side: usize| {
        decode_end(
            edges,
            gear[side],
            areas[side],
            extensions[side].as_ref(),
            side == 1,
            axis,
            tolerance,
        )
    };
    let [lower, upper] = [end(0)?, end(1)?];
    let [area, high_area] = [lower.tooth_area, upper.tooth_area];
    if (area - high_area).abs() > tolerance * area.sqrt().max(1.) * 32. {
        return Err(ReadError::Invalid);
    }
    let bore = match [lower.bore, upper.bore] {
        [None, None] => None,
        [Some(low), Some(high)] if equal_radius(low, high, tolerance) => Some(PeriodicWall {
            rims: [low, high],
            outward: false,
        }),
        _ => return Err(ReadError::Invalid),
    };
    let mut volume = area * gear_height;
    let mut periodic = Vec::with_capacity(3);
    for extension in [lower.extension, upper.extension].into_iter().flatten() {
        volume += extension.rims[0].area() * dot(extension.delta(), axis);
        periodic.push(extension);
    }
    if let Some(bore) = bore {
        volume -= bore.rims[0].area() * dot(bore.delta(), axis);
        periodic.push(bore);
    }

    let mut low = outline_uses(gear[0].1);
    let mut high = outline_uses(gear[1].1);
    let mut seams = BTreeSet::new();
    let gear_delta = sub(gear[1].0.origin, gear[0].0.origin);
    for wall in walls {
        if let Some(index) = periodic.iter().position(|span| {
            wall.outer
                .iter()
                .any(|use_| use_.edge == span.rims[0].edge.edge)
        }) {
            let span = periodic.swap_remove(index);
            if wall.forward != span.outward {
                return Err(ReadError::Invalid);
            }
            let [mut lower, mut upper] = span
                .rims
                .map(|rim| BTreeMap::from([(rim.edge.edge, (rim.edge.forward, true))]));
            verify_wall(
                reader,
                edges,
                wall,
                &mut lower,
                &mut upper,
                &mut seams,
                span.delta(),
                axis,
            )?;
        } else {
            verify_wall(
                reader, edges, wall, &mut low, &mut high, &mut seams, gear_delta, axis,
            )?;
        }
    }
    if !periodic.is_empty() || !low.is_empty() || !high.is_empty() {
        return Err(ReadError::Invalid);
    }
    Ok((volume, seams))
}

fn outline_uses(face: &FaceWire) -> BTreeMap<u32, (bool, bool)> {
    face.outer
        .iter()
        .map(|edge| (edge.edge, (edge.forward, false)))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn decode_end<'a>(
    edges: &Edges,
    (shoulder_plane, shoulder): (&Plane, &'a FaceWire),
    shoulder_area: f64,
    extension: Option<&'a (Plane, FaceWire)>,
    upper: bool,
    axis: Point,
    tolerance: f64,
) -> Result<End<'a>> {
    let inner = |plane: &Plane, face: &'a FaceWire| {
        face.hole
            .as_deref()
            .map(|ring| concentric_circle(edges, ring, plane, false, tolerance))
            .transpose()
    };
    let rim = inner(shoulder_plane, shoulder)?;
    let tooth_area = shoulder_area + rim.map_or(0., CircleTrim::area);
    let Some((plane, face)) = extension else {
        return Ok(End {
            bore: rim,
            extension: None,
            tooth_area,
        });
    };
    let shoulder_trim = rim.ok_or(ReadError::Invalid)?;
    let remote = concentric_circle(edges, &face.outer, plane, true, tolerance)?;
    let bore = inner(plane, face)?;
    let travel = sub(plane.origin, shoulder_plane.origin);
    let distance = dot(travel, axis) * if upper { 1. } else { -1. };
    if distance <= tolerance
        || length(cross(travel, axis)) > tolerance
        || !equal_radius(shoulder_trim, remote, tolerance)
        || bore.is_some_and(|bore| bore.circle.radius_mm + tolerance >= remote.circle.radius_mm)
    {
        return Err(ReadError::Invalid);
    }
    let rims = if upper {
        [shoulder_trim, remote]
    } else {
        [remote, shoulder_trim]
    };
    Ok(End {
        bore,
        extension: Some(PeriodicWall {
            rims,
            outward: true,
        }),
        tooth_area,
    })
}

fn equal_radius(a: CircleTrim<'_>, b: CircleTrim<'_>, tolerance: f64) -> bool {
    (a.circle.radius_mm - b.circle.radius_mm).abs() <= tolerance
}

fn concentric_circle<'a>(
    edges: &Edges,
    ring: &'a [Traversal],
    plane: &Plane,
    outer: bool,
    tolerance: f64,
) -> Result<CircleTrim<'a>> {
    let [edge] = ring else {
        return Err(ReadError::Unsupported);
    };
    let Geometry::Analytic(Curve::Circle(circle)) = edges[&edge.edge].geometry else {
        return Err(ReadError::Unsupported);
    };
    let ccw = (dot(circle.normal, plane.normal) > 0.) == edge.forward;
    if ccw != outer
        || !circle.within_plane(plane, tolerance)
        || length(sub(circle.center_mm, plane.origin)) > tolerance
    {
        return Err(ReadError::Invalid);
    }
    Ok(CircleTrim { edge, circle })
}
