//! Recover a line-and-arc miter sweep from the oriented wall and rim uses.
use super::{axial_cuts::verify, *};
use crate::normal_sweep::Edge;

type WireUse = (u32, bool);

pub(super) fn reconstruct(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
) -> Result<Option<ImportedWarped>> {
    if boundary.faces.iter().any(|face| !face.holes.is_empty()) {
        return Ok(None);
    }
    let caps: Vec<_> = boundary
        .faces
        .iter()
        .enumerate()
        .filter_map(|(index, face)| {
            let Surface::Plane { normal, .. } = face.surface else {
                return None;
            };
            let mut has_arc = false;
            for &(id, _) in &face.loop_ {
                if let TrimCurve::Arc(arc) = boundary.curves[&id].1 {
                    if !circle(arc) {
                        return None;
                    }
                    has_arc = true;
                }
            }
            has_arc.then_some((index, normal))
        })
        .collect();
    let &[(start, outward), (end, _)] = caps.as_slice() else {
        return Ok(None);
    };
    let profile: Vec<_> = boundary.faces[start]
        .loop_
        .iter()
        .rev()
        .map(|&(edge, forward)| (edge, !forward))
        .collect();
    let Some(vertices) = generator(boundary, &profile, end) else {
        return Ok(None);
    };

    // Any corresponding rail generator is a valid reference path. Anchor
    // the chart on its first vertex and retain the profile's signed offset
    // from that generator when replaying every miter plane.
    let origin_mm = reader.points[&vertices[0]];
    let up = outward.map(|v| -v);
    let chord = sub(reader.points[&ends(boundary, profile[0])[1]], origin_mm);
    let axial = dot(chord, up);
    let u = unit(std::array::from_fn(|i| chord[i] - axial * up[i]))?;
    let v = cross(up, u);
    let chart = Chart {
        origin: origin_mm,
        u,
        v,
        up,
        // This verifier uses only the rigid placement.
        band: Band {
            center_z_mm: 0.,
            radius_mm: 1.,
        },
    };
    let mut path: Vec<_> = vertices
        .iter()
        .map(|id| chart.local(reader.points[id]))
        .collect();
    let tolerance = reader.tolerance;
    if path[1][0].hypot(path[1][1]) > tolerance || path[1][2] <= tolerance {
        return Err(ReadError::Invalid);
    }
    path[1][..2].copy_from_slice(&[0.; 2]);
    let profile = profile
        .into_iter()
        .map(|usage| profile_edge(reader, boundary, usage, chart))
        .collect::<Result<Vec<_>>>()?;
    let shell = warped_union::mixed_miter_from_rim(&profile, &path, tolerance)
        .map_err(axial_cuts::replay_error)?;
    verify::replayed(reader, boundary, chart, &shell)?;
    Ok(Some(ImportedWarped {
        shell,
        origin_mm,
        u,
        v,
    }))
}

fn circle(arc: EllipseArc) -> bool {
    let [major, minor] = arc.ellipse().semiaxes_mm();
    major == minor
}

fn ends(boundary: &Boundary, (edge, forward): WireUse) -> [u32; 2] {
    let [a, b] = boundary.curves[&edge].0;
    if forward { [a, b] } else { [b, a] }
}

fn generator(boundary: &Boundary, initial: &[WireUse], end: usize) -> Option<Vec<u32>> {
    let width = initial.len();
    let walls = boundary.faces.len() - 2;
    let legs = walls / width;
    if !(2..=64).contains(&legs)
        || walls != legs * width
        || boundary.faces[end].loop_.len() != width
        || boundary.curves.len() != width * (2 * legs + 1)
    {
        return None;
    }
    // Boundary parsing proved that both uses of every edge occur once.
    let by_use: BTreeMap<_, _> = boundary
        .faces
        .iter()
        .enumerate()
        .flat_map(|(index, face)| {
            face.loop_
                .iter()
                .enumerate()
                .map(move |(slot, &usage)| (usage, (index, slot)))
        })
        .collect();
    let mut used = BTreeSet::new();
    let mut wire = initial.to_vec();
    let mut path = vec![ends(boundary, wire[0])[0]];
    for _ in 0..legs {
        let mut next = Vec::with_capacity(width);
        for usage in wire {
            let (wall, slot) = by_use[&usage];
            let loop_ = &boundary.faces[wall].loop_;
            if wall == end || loop_.len() != 4 || !used.insert(wall) {
                return None;
            }
            if [1, 3].into_iter().any(|offset| {
                !matches!(
                    boundary.curves[&loop_[(slot + offset) % 4].0].1,
                    TrimCurve::Line
                )
            }) {
                return None;
            }
            let (edge, forward) = loop_[(slot + 2) % 4];
            next.push((edge, !forward));
        }
        if next
            .iter()
            .zip(next.iter().cycle().skip(1))
            .any(|(&a, &b)| ends(boundary, a)[1] != ends(boundary, b)[0])
        {
            return None;
        }
        path.push(ends(boundary, next[0])[0]);
        wire = next;
    }
    wire.iter()
        .all(|usage| by_use[usage].0 == end)
        .then_some(path)
}

fn profile_edge(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
    usage: WireUse,
    chart: Chart,
) -> Result<Edge> {
    let point = |world| {
        let [x, y, z] = chart.local(world);
        if z.abs() > reader.tolerance {
            return Err(ReadError::Invalid);
        }
        Ok([x, y])
    };
    let [a, b] = ends(boundary, usage).map(|id| point(reader.points[&id]));
    let (a, b) = (a?, b?);
    match boundary.curves[&usage.0].1 {
        TrimCurve::Line => Ok(Edge::Line { a, b }),
        TrimCurve::Arc(arc) => {
            let ellipse = arc.ellipse();
            let facing = dot(chart.up, ellipse.normal());
            if facing.abs() < 1. - 1e-10 {
                return Err(ReadError::Invalid);
            }
            Ok(Edge::Arc {
                center: point(ellipse.center_mm())?,
                a,
                b,
                clockwise: (facing * arc.sweep_rad() < 0.) == usage.1,
            })
        }
    }
}

#[cfg(test)]
mod tests;
