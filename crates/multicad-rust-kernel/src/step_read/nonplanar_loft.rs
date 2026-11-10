//! Replay a convex two-section solid from its closed edges and bilinear poles.
use super::*;
use crate::planar_boolean::NonplanarLoft;

#[derive(Debug)]
pub struct ImportedNonplanarLoft {
    pub geometry: NonplanarLoft,
    pub origin_mm: Point,
    pub u: Point,
    pub v: Point,
}

pub fn read_nonplanar_loft_solid(input: &str, tolerance_mm: f64) -> Result<ImportedNonplanarLoft> {
    compound::read_members_by(input, tolerance_mm, true, true, |shell, id, _| {
        let faces = shell.member_faces(id)?;
        read_member(shell, &faces)?
            .map(|(_, geometry)| geometry)
            .ok_or(ReadError::Unsupported)
    })?
    .pop()
    .ok_or(ReadError::Invalid)
}

struct Surface {
    id: u32,
    forward: bool,
    bound: u32,
}

fn surface(shell: &Shell<'_, '_>, face: u32) -> Result<Option<Surface>> {
    let [Value::String(_), bounds, id, forward] = shell.args(face, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    let bounds = refs(bounds)?;
    let [bound] = bounds.as_slice() else {
        return Ok(None);
    };
    Ok(Some(Surface {
        id: reference(id)?,
        forward: sense(forward)?,
        bound: *bound,
    }))
}

struct Cap {
    plane: Plane,
    vertices: Vec<u32>,
}

pub(super) fn read_member(
    shell: &mut Shell<'_, '_>,
    faces: &[u32],
) -> Result<Option<(Readback, ImportedNonplanarLoft)>> {
    if !(5..=66).contains(&faces.len()) {
        return Ok(None);
    }
    let mut caps = Vec::with_capacity(2);
    let mut walls = Vec::with_capacity(faces.len() - 2);
    let mut seen = BTreeSet::new();
    for &face in faces {
        let Some(surface) = surface(shell, face)? else {
            return Ok(None);
        };
        let entity = shell.simple(surface.id)?;
        match (entity.name, entity.args.get(1..3)) {
            ("PLANE", _) => caps.push((face, surface)),
            ("B_SPLINE_SURFACE_WITH_KNOTS", Some([Value::Number(1.), Value::Number(1.)])) => {
                walls.push((face, surface));
            }
            _ => return Ok(None),
        }
        if !seen.insert(face) {
            return Err(ReadError::Invalid);
        }
    }
    let [(first_id, first), (last_id, last)] = caps.as_slice() else {
        return Ok(None);
    };
    let count = walls.len();
    let [first, last] = [
        cap(shell, *first_id, first, count)?,
        cap(shell, *last_id, last, count)?,
    ];
    let mut correspondence = BTreeMap::new();
    let first_ids = first.vertices.iter().copied().collect::<BTreeSet<_>>();
    let last_ids = last.vertices.iter().copied().collect::<BTreeSet<_>>();
    if first_ids.len() != count || last_ids.len() != count || !first_ids.is_disjoint(&last_ids) {
        return Err(ReadError::Invalid);
    }
    for (face, wall) in walls {
        let ring = loop_vertices(shell, face, wall.bound, 4)?;
        let ring: [u32; 4] = ring.try_into().expect("four validated edges");
        let cap_edges = |ids: &BTreeSet<_>| {
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .filter(|&(a, b)| ids.contains(a) && ids.contains(b))
                .count()
        };
        if cap_edges(&first_ids) != 1 || cap_edges(&last_ids) != 1 {
            return Err(ReadError::Invalid);
        }
        verify_surface(shell, &wall, ring)?;
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            if first_ids.contains(&a)
                && last_ids.contains(&b)
                && correspondence.insert(a, b).is_some()
            {
                return Err(ReadError::Invalid);
            }
        }
    }
    if correspondence.len() != count || shell.edges.len() != 3 * count
        || shell.edges.values().any(|edge| {
            !matches!(edge.uses.as_slice(), [a, b] if a.forward != b.forward && a.face != b.face)
        })
    {
        return Err(ReadError::Invalid);
    }
    // Reverse the first outward cap ring to run CCW along the loft.
    let start: Vec<_> = first.vertices.iter().rev().copied().collect();
    let end: Vec<_> = start.iter().map(|id| correspondence[id]).collect();
    verify_cycle(&end, &last.vertices)?;
    let (proof, imported) = reconstruct(shell, first.plane, last.plane, [&start, &end])?;
    Ok(Some((proof, imported)))
}

fn loop_vertices(
    shell: &mut Shell<'_, '_>,
    face: u32,
    bound: u32,
    count: usize,
) -> Result<Vec<u32>> {
    let (_, wire, forward) = shell.bound(bound, true)?;
    let edges = shell.traversals(wire, forward, face as usize, count)?;
    if edges.len() != count {
        return Err(ReadError::Invalid);
    }
    for edge in &edges {
        if !matches!(shell.edges[&edge.edge].curve, Curve::Line) {
            return Err(ReadError::Invalid);
        }
    }
    Ok(edges.into_iter().map(|edge| edge.start).collect())
}

fn cap(shell: &mut Shell<'_, '_>, face: u32, surface: &Surface, count: usize) -> Result<Cap> {
    let [Value::String(_), placement] = shell.args(surface.id, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    let mut plane = shell.placement(reference(placement)?)?;
    if !surface.forward {
        plane.normal = plane.normal.map(|x| -x);
    }
    let vertices = loop_vertices(shell, face, surface.bound, count)?;
    let anchor = shell.points[&vertices[0]];
    if vertices
        .iter()
        .any(|id| dot(sub(shell.points[id], plane.origin), plane.normal).abs() > shell.tolerance)
    {
        return Err(ReadError::Invalid);
    }
    let signed_area = vertices
        .windows(2)
        .map(|pair| {
            dot(
                plane.normal,
                cross(
                    sub(shell.points[&pair[0]], anchor),
                    sub(shell.points[&pair[1]], anchor),
                ),
            )
        })
        .sum::<f64>();
    if signed_area <= 0. {
        return Err(ReadError::Invalid);
    }
    Ok(Cap { plane, vertices })
}

fn verify_surface(shell: &Shell<'_, '_>, surface: &Surface, ring: [u32; 4]) -> Result<()> {
    use Value::{Enum, List, Number, String};
    let [
        String(_),
        Number(1.),
        Number(1.),
        List(net),
        Enum("RULED_SURF" | "UNSPECIFIED"),
        Enum("F"),
        Enum("F"),
        Enum("F"),
        List(u_mult),
        List(v_mult),
        List(u_knots),
        List(v_knots),
        Enum(_),
    ] = shell.args(surface.id, "B_SPLINE_SURFACE_WITH_KNOTS")?
    else {
        return Err(ReadError::Invalid);
    };
    if [u_mult, v_mult]
        .into_iter()
        .any(|m| m.as_slice() != [Number(2.), Number(2.)])
        || [u_knots, v_knots]
            .into_iter()
            .any(|k| !matches!(k.as_slice(), [Number(a), Number(b)] if a < b))
    {
        return Err(ReadError::Invalid);
    }
    let [List(u0), List(u1)] = net.as_slice() else {
        return Err(ReadError::Invalid);
    };
    let ([a, d], [b, c]) = (u0.as_slice(), u1.as_slice()) else {
        return Err(ReadError::Invalid);
    };
    let mut points = [[0.; 3]; 4];
    for (point, pole) in points.iter_mut().zip([a, b, c, d]) {
        *point = shell.point(reference(pole)?)?;
    }
    if !surface.forward {
        points.reverse();
    }
    if !(0..4).any(|offset| {
        ring.iter().enumerate().all(|(i, id)| {
            length(sub(shell.points[id], points[(i + offset) % 4])) <= shell.tolerance
        })
    }) {
        return Err(ReadError::Invalid);
    }
    Ok(())
}

fn verify_cycle(a: &[u32], b: &[u32]) -> Result<()> {
    let same = a.len() == b.len()
        && b.iter().position(|p| p == &a[0]).is_some_and(|offset| {
            a.iter()
                .enumerate()
                .all(|(i, p)| *p == b[(i + offset) % b.len()])
        });
    if !same {
        return Err(ReadError::Invalid);
    }
    Ok(())
}

fn reconstruct(
    shell: &Shell<'_, '_>,
    first: Plane,
    last: Plane,
    rings: [&[u32]; 2],
) -> Result<(Readback, ImportedNonplanarLoft)> {
    let origin_mm = shell.points[&rings[0][0]];
    let normal = first.normal.map(|x| -x);
    let u = first.u;
    let v = cross(normal, u);
    let height = dot(sub(shell.points[&rings[1][0]], origin_mm), normal);
    if dot(normal, last.normal) < 0.
        || length(cross(normal, last.normal)) > 1e-10
        || height <= shell.tolerance
    {
        return Err(ReadError::Invalid);
    }
    let project = |ring: &[u32]| -> Vec<_> {
        ring.iter()
            .map(|id| {
                let delta = sub(shell.points[id], origin_mm);
                [dot(delta, u), dot(delta, v)]
            })
            .collect()
    };
    let sections = rings.map(project);
    if rings.into_iter().zip([0., height]).any(|(ring, station)| {
        ring.iter().any(|id| {
            (dot(sub(shell.points[id], origin_mm), normal) - station).abs() > shell.tolerance
        })
    }) {
        return Err(ReadError::Invalid);
    }
    let geometry = NonplanarLoft::new([&sections[0], &sections[1]], height, shell.tolerance)
        .map_err(|_| ReadError::Invalid)?;
    let mut bounds_mm = [origin_mm; 2];
    for &point in shell.points.values() {
        include_point(&mut bounds_mm, point);
    }
    let proof = Readback {
        bounds_mm,
        volume_mm3: geometry.volume_mm3(),
        face_count: rings[0].len() + 2,
        edges_mm: shell
            .edges
            .values()
            .map(|edge| edge.vertices.map(|id| shell.points[&id]))
            .collect(),
        circles: Vec::new(),
        semicircles: Vec::new(),
        arcs: Vec::new(),
        ellipses: Vec::new(),
        splines: Vec::new(),
        solid_count: 1,
    };
    Ok((
        proof,
        ImportedNonplanarLoft {
            geometry,
            origin_mm,
            u,
            v,
        },
    ))
}

#[cfg(test)]
mod tests;
