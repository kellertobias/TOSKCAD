//! Verify bounded normal sweeps with cubic spline and analytic rim edges.
use super::*;

mod hub;
mod reconstruction;
pub(super) use reconstruction::{DecodedSplineSweep, import_member};
mod section;

struct FaceWire {
    surface: u32,
    forward: bool,
    outer: Vec<Traversal>,
    hole: Option<Vec<Traversal>>,
}

enum Geometry {
    Analytic(Curve),
    Cubic { id: u32, curve: SplineEdge },
}

struct EdgeData {
    vertices: [u32; 2],
    geometry: Geometry,
    uses: [u8; 2],
}

type Edges = BTreeMap<u32, EdgeData>;

pub(super) fn has_extrusion_support(reader: &Shell<'_, '_>, id: u32) -> bool {
    matches!(
        face_args(reader, id),
        Ok((_, support, _)) if reader.simple(support).is_ok_and(|s| s.name == "SURFACE_OF_LINEAR_EXTRUSION")
    )
}

fn face_args(reader: &Shell<'_, '_>, id: u32) -> Result<(Vec<u32>, u32, bool)> {
    let [Value::String(_), bounds, surface, forward] = reader.args(id, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    Ok((refs(bounds)?, reference(surface)?, sense(forward)?))
}

pub(super) fn read_member(reader: &mut Shell<'_, '_>, ids: Vec<u32>) -> Result<Readback> {
    read_member_with(reader, ids, |_, _, _, _, _, _| Ok(())).map(|(readback, _)| readback)
}

fn read_member_with<T>(
    reader: &mut Shell<'_, '_>,
    ids: Vec<u32>,
    reconstruct: impl FnOnce(&Shell<'_, '_>, &Edges, &Plane, &FaceWire, Point, f64) -> Result<T>,
) -> Result<(Readback, Option<T>)> {
    if ids.len() < 5 || ids.len() > MAX_VERTICES + 7 {
        return Err(ReadError::Limit);
    }
    let first_spline_wall = ids
        .iter()
        .find(|&&id| has_extrusion_support(reader, id))
        .ok_or(ReadError::Unsupported)?;
    let (_, support, _) = face_args(reader, *first_spline_wall)?;
    let (_, normal) = reader.spline_extrusion(support)?;

    let mut edges = Edges::new();
    let mut face_ids = BTreeSet::new();
    let mut caps = [None, None];
    let mut ends = [None, None];
    let mut walls = Vec::new();
    for id in ids {
        if !face_ids.insert(id) {
            return Err(ReadError::Invalid);
        }
        let face = read_face(reader, id, &mut edges)?;
        if let Some(cap) = normal_cap(reader, &face, normal)? {
            let side = usize::from(dot(cap.normal, normal) > 0.);
            let destination = if face.outer.len() == 1 {
                &mut ends[side]
            } else {
                &mut caps[side]
            };
            if destination.replace((cap, face)).is_some() {
                return Err(ReadError::Invalid);
            }
        } else {
            walls.push(face);
        }
    }
    if edges.values().any(|edge| edge.uses != [1, 1]) {
        return Err(ReadError::Invalid);
    }
    let [Some((base, low)), Some((top, high))] = caps else {
        return Err(ReadError::Invalid);
    };
    let delta = sub(top.origin, base.origin);
    let height = dot(delta, normal);
    if height <= reader.tolerance || length(cross(normal, delta)) > reader.tolerance {
        return Err(ReadError::Invalid);
    }
    let base_area = section::verify(reader, &edges, &base, &low)?;
    let top_area = section::verify(reader, &edges, &top, &high)?;
    if ends.iter().any(Option::is_some) {
        let (volume, seams) = hub::verify(
            reader,
            &edges,
            [(&base, &low), (&top, &high)],
            [base_area, top_area],
            &ends,
            &walls,
            normal,
            height,
        )?;
        return metrics(reader, edges, &seams, face_ids.len(), volume).map(|proof| (proof, None));
    }
    if (base_area - top_area).abs() > reader.tolerance * base_area.sqrt().max(1.) * 32. {
        return Err(ReadError::Invalid);
    }
    let mut low_uses = rim_uses(&low);
    let mut high_uses = rim_uses(&high);
    let mut seams = BTreeSet::new();
    for wall in &walls {
        verify_wall(
            reader,
            &edges,
            wall,
            &mut low_uses,
            &mut high_uses,
            &mut seams,
            delta,
            normal,
        )?;
    }
    // Every rim edge has two opposite uses and each wall consumes exactly
    // one edge from each cap, so every rim edge has a unique side.
    let solid = reconstruct(reader, &edges, &base, &low, normal, height)?;
    let readback = metrics(reader, edges, &seams, face_ids.len(), base_area * height)?;
    Ok((readback, Some(solid)))
}

fn read_face(reader: &mut Shell<'_, '_>, id: u32, edges: &mut Edges) -> Result<FaceWire> {
    let (bounds, surface, forward) = face_args(reader, id)?;
    if !(1..=2).contains(&bounds.len()) {
        return Err(ReadError::Unsupported);
    }
    let implicit_outer = bounds.len() == 1;
    let mut outer = None;
    let mut hole = None;
    for id in bounds {
        let (exterior, wire, sense) = reader.bound(id, implicit_outer)?;
        let ring = read_ring(reader, wire, sense, edges)?;
        let slot = if exterior { &mut outer } else { &mut hole };
        if slot.replace(ring).is_some() {
            return Err(ReadError::Invalid);
        }
    }
    Ok(FaceWire {
        surface,
        forward,
        outer: outer.ok_or(ReadError::Invalid)?,
        hole,
    })
}

fn read_ring(
    reader: &mut Shell<'_, '_>,
    id: u32,
    forward: bool,
    edges: &mut Edges,
) -> Result<Vec<Traversal>> {
    let [Value::String(_), oriented] = reader.args(id, "EDGE_LOOP")? else {
        return Err(ReadError::Invalid);
    };
    let mut oriented = refs(oriented)?;
    if oriented.is_empty() || oriented.len() > MAX_VERTICES {
        return Err(ReadError::Limit);
    }
    if !forward {
        oriented.reverse();
    }
    let ring = oriented
        .into_iter()
        .map(|id| {
            let [
                Value::String(_),
                Value::Derived,
                Value::Derived,
                edge,
                direction,
            ] = reader.args(id, "ORIENTED_EDGE")?
            else {
                return Err(ReadError::Invalid);
            };
            let edge = reference(edge)?;
            let forward = sense(direction)? == forward;
            let data = match edges.entry(edge) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(read_edge(reader, edge)?)
                }
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            };
            let count = &mut data.uses[usize::from(forward)];
            if *count != 0 {
                return Err(ReadError::Invalid);
            }
            *count = 1;
            let [a, b] = data.vertices;
            let [start, end] = if forward { [a, b] } else { [b, a] };
            Ok(Traversal {
                edge,
                start,
                end,
                forward,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .any(|(a, b)| a.end != b.start)
    {
        return Err(ReadError::Invalid);
    }
    Ok(ring)
}

fn read_edge(reader: &mut Shell<'_, '_>, id: u32) -> Result<EdgeData> {
    let [Value::String(_), start, end, curve, forward] = reader.args(id, "EDGE_CURVE")? else {
        return Err(ReadError::Invalid);
    };
    let vertices = [reference(start)?, reference(end)?];
    let forward = sense(forward)?;
    let id = reference(curve)?;
    let geometry = match reader.simple(id)?.name {
        "LINE" => Geometry::Analytic(reader.line_curve(id, vertices, forward)?),
        "CIRCLE" => Geometry::Analytic(reader.circle_curve(id, vertices, forward)?),
        "B_SPLINE_CURVE_WITH_KNOTS" if forward => {
            let curve = reader.spline_curve(id)?;
            let [a, b] = [reader.vertex(vertices[0])?, reader.vertex(vertices[1])?];
            if length(sub(a, b)) <= reader.tolerance
                || length(sub(a, curve.poles_mm()[0])) > reader.tolerance
                || length(sub(b, *curve.poles_mm().last().ok_or(ReadError::Invalid)?))
                    > reader.tolerance
            {
                return Err(ReadError::Invalid);
            }
            Geometry::Cubic { id, curve }
        }
        _ => return Err(ReadError::Unsupported),
    };
    Ok(EdgeData {
        vertices,
        geometry,
        uses: [0; 2],
    })
}

fn normal_cap(reader: &Shell<'_, '_>, face: &FaceWire, normal: Point) -> Result<Option<Plane>> {
    if reader.simple(face.surface)?.name != "PLANE" {
        return Ok(None);
    }
    let plane = face_plane(reader, face)?;
    Ok((length(cross(plane.normal, normal)) <= 1e-10).then_some(plane))
}

fn face_plane(reader: &Shell<'_, '_>, face: &FaceWire) -> Result<Plane> {
    let [Value::String(_), placement] = reader.args(face.surface, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    let mut plane = reader.placement(reference(placement)?)?;
    if !face.forward {
        plane.normal = plane.normal.map(|n| -n);
        plane.v = plane.v.map(|n| -n);
    }
    Ok(plane)
}

fn rim_uses(face: &FaceWire) -> BTreeMap<u32, (bool, bool)> {
    std::iter::once((&face.outer, false))
        .chain(face.hole.iter().map(|ring| (ring, true)))
        .flat_map(|(ring, bore)| {
            ring.iter()
                .map(move |edge| (edge.edge, (edge.forward, bore)))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn verify_wall(
    reader: &Shell<'_, '_>,
    edges: &Edges,
    wall: &FaceWire,
    low_uses: &mut BTreeMap<u32, (bool, bool)>,
    high_uses: &mut BTreeMap<u32, (bool, bool)>,
    seams: &mut BTreeSet<u32>,
    delta: Point,
    normal: Point,
) -> Result<()> {
    let [_, _, _, _] = wall.outer.as_slice() else {
        return Err(ReadError::Unsupported);
    };
    if wall.hole.is_some() {
        return Err(ReadError::Unsupported);
    }
    let start = wall
        .outer
        .iter()
        .position(|edge| low_uses.contains_key(&edge.edge))
        .ok_or(ReadError::Invalid)?;
    let [lower, up, upper, down] = std::array::from_fn(|i| &wall.outer[(start + i) % 4]);
    let (low_sense, low_bore) = low_uses.remove(&lower.edge).ok_or(ReadError::Invalid)?;
    let (high_sense, high_bore) = high_uses.remove(&upper.edge).ok_or(ReadError::Invalid)?;
    if low_bore != high_bore
        || low_sense == lower.forward
        || high_sense == upper.forward
        || !matches!(edges[&up.edge].geometry, Geometry::Analytic(Curve::Line))
        || !matches!(edges[&down.edge].geometry, Geometry::Analytic(Curve::Line))
        || length(sub(
            sub(reader.points[&up.end], reader.points[&up.start]),
            delta,
        )) > reader.tolerance
        || length(sub(
            sub(reader.points[&down.start], reader.points[&down.end]),
            delta,
        )) > reader.tolerance
    {
        return Err(ReadError::Invalid);
    }
    if (up.edge == down.edge) != low_bore {
        return Err(ReadError::Invalid);
    }
    if low_bore {
        seams.insert(up.edge);
    }
    let low = &edges[&lower.edge].geometry;
    let high = &edges[&upper.edge].geometry;
    match (low, high) {
        (Geometry::Analytic(Curve::Line), Geometry::Analytic(Curve::Line)) if !low_bore => {
            let plane = face_plane(reader, wall)?;
            let [a, b] = [lower.start, lower.end].map(|id| reader.points[&id]);
            let expected = unit(cross(sub(b, a), normal))?;
            if !same_direction(plane.normal, expected)
                || wall.outer.iter().any(|edge| {
                    dot(sub(reader.points[&edge.start], plane.origin), plane.normal).abs()
                        > reader.tolerance
                })
            {
                return Err(ReadError::Invalid);
            }
        }
        (Geometry::Cubic { id, curve }, Geometry::Cubic { curve: top, .. }) if !low_bore => {
            let (support, axis) = reader.spline_extrusion(wall.surface)?;
            if support != *id
                || wall.forward != lower.forward
                || lower.forward == upper.forward
                || !same_direction(axis, normal)
                || !curve.translated(top, delta, reader.tolerance)
            {
                return Err(ReadError::Invalid);
            }
        }
        (Geometry::Analytic(low), Geometry::Analytic(high)) => {
            let [Some(low), Some(high)] = [round_geometry(*low), round_geometry(*high)] else {
                return Err(ReadError::Invalid);
            };
            let axis_sense = |circle: Round, forward| {
                if forward {
                    circle.normal
                } else {
                    circle.normal.map(|n| -n)
                }
            };
            let sense = axis_sense(low, lower.forward);
            if low.full != low_bore
                || high.full != high_bore
                || !same_direction(sense, axis_sense(high, !upper.forward))
                || (low.sweep - high.sweep).abs() * low.radius > reader.tolerance
                || (low.radius - high.radius).abs() > reader.tolerance
                || length(sub(sub(high.center, low.center), delta)) > reader.tolerance
                || wall.forward != (dot(sense, normal) > 0.)
            {
                return Err(ReadError::Invalid);
            }
            let [Value::String(_), axis, Value::Number(radius)] =
                reader.args(wall.surface, "CYLINDRICAL_SURFACE")?
            else {
                return Err(ReadError::Invalid);
            };
            let axis = reader.placement(reference(axis)?)?;
            if length(cross(axis.normal, normal)) > 1e-10
                || (*radius - low.radius).abs() > reader.tolerance
                || length(cross(sub(axis.origin, low.center), normal)) > reader.tolerance
            {
                return Err(ReadError::Invalid);
            }
        }
        _ => return Err(ReadError::Invalid),
    }
    Ok(())
}

fn same_direction(a: Point, b: Point) -> bool {
    dot(a, b) > 0. && length(cross(a, b)) <= 1e-10
}

#[derive(Clone, Copy)]
struct Round {
    center: Point,
    normal: Point,
    radius: f64,
    sweep: f64,
    full: bool,
}

fn round_geometry(curve: Curve) -> Option<Round> {
    use std::f64::consts::{PI, TAU};
    let (center, normal, radius, sweep, full) = match curve {
        Curve::Line => return None,
        Curve::Circle(edge) => (edge.center_mm, edge.normal, edge.radius_mm, TAU, true),
        Curve::Semicircle(edge) => (edge.center_mm, edge.normal, edge.radius_mm, PI, false),
        Curve::Arc(edge) => (
            edge.center_mm,
            edge.normal,
            edge.radius_mm,
            edge.sweep_rad,
            false,
        ),
    };
    Some(Round {
        center,
        normal,
        radius,
        sweep,
        full,
    })
}

fn metrics(
    reader: &Shell<'_, '_>,
    edges: Edges,
    seams: &BTreeSet<u32>,
    face_count: usize,
    volume_mm3: f64,
) -> Result<Readback> {
    if !volume_mm3.is_finite() || volume_mm3 <= 0. {
        return Err(ReadError::Invalid);
    }
    let anchor = *reader.points.values().next().ok_or(ReadError::Invalid)?;
    let mut bounds_mm = [anchor; 2];
    for &point in reader.points.values() {
        include_point(&mut bounds_mm, point);
    }
    let mut result = Readback {
        bounds_mm,
        volume_mm3,
        face_count,
        edges_mm: Vec::new(),
        circles: Vec::new(),
        semicircles: Vec::new(),
        arcs: Vec::new(),
        ellipses: Vec::new(),
        splines: Vec::new(),
        solid_count: 1,
    };
    for (id, edge) in edges {
        match edge.geometry {
            Geometry::Analytic(Curve::Line) => {
                if !seams.contains(&id) {
                    result
                        .edges_mm
                        .push(edge.vertices.map(|vertex| reader.points[&vertex]));
                }
            }
            Geometry::Analytic(Curve::Circle(circle)) => {
                for p in circle.bounds() {
                    include_point(&mut result.bounds_mm, p);
                }
                result.circles.push(circle);
            }
            Geometry::Analytic(Curve::Semicircle(arc)) => {
                arc.include_bounds(&mut result.bounds_mm);
                result.semicircles.push(arc);
            }
            Geometry::Analytic(Curve::Arc(arc)) => {
                arc.include_bounds(&mut result.bounds_mm);
                result.arcs.push(arc);
            }
            Geometry::Cubic { curve, .. } => {
                for p in curve.bounds() {
                    include_point(&mut result.bounds_mm, p);
                }
                result.splines.push(curve);
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
