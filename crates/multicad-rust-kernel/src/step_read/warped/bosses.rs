//! Rebuild a stock shell after proving attached cylindrical faces by their incidence.
use super::*;
use crate::warped_union::{Boss, OrientedEdge};

struct Attachment {
    host: usize,
    hole: usize,
    walls: [usize; 4],
    cap: usize,
    mouth: Point,
    top: Point,
    normal: Point,
    radial: Point,
    radius: f64,
}

pub(super) fn reconstruct(reader: &Shell<'_, '_>, source: &Boundary) -> Result<ImportedWarped> {
    let attachments = attachments(reader, source)?;
    let removed: BTreeSet<_> = attachments
        .iter()
        .flat_map(|boss| boss.walls.into_iter().chain([boss.cap]))
        .collect();
    if removed.len() != attachments.len() * 5
        || attachments.iter().any(|boss| removed.contains(&boss.host))
    {
        return Err(ReadError::Invalid);
    }
    let retained: Vec<_> = source
        .faces
        .iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .collect();
    let face_indices: BTreeMap<_, _> = retained
        .iter()
        .enumerate()
        .map(|(new, &(old, _))| (old, new))
        .collect();
    let mut base = Boundary {
        faces: Vec::with_capacity(retained.len()),
        curves: BTreeMap::new(),
    };
    let mut base_reader = Shell::new(reader.entities, reader.tolerance, true);
    for (_, face) in retained {
        for &(edge, _) in &face.loop_ {
            let curve = source.curves[&edge];
            base.curves.insert(edge, curve);
            for id in curve.0 {
                base_reader.points.insert(id, reader.points[&id]);
            }
        }
        base.faces.push(FaceLoop {
            surface: face.surface,
            loop_: face.loop_.clone(),
            holes: Vec::new(),
        });
    }
    let (mut imported, chart) = resolve_chart(&base_reader, &base, true)?;
    let tol = reader.tolerance;
    let tools: Vec<_> = attachments
        .iter()
        .enumerate()
        .map(|(index, boss)| {
            let mouth = chart.local(boss.mouth);
            let normal = chart.vector(boss.normal);
            let depth = dot(sub(boss.top, boss.mouth), boss.normal);
            let embed = (depth * 0.01).min(4. * tol).max(2. * tol);
            let start_mm = std::array::from_fn(|i| mouth[i] - normal[i] * embed);
            let end_mm = std::array::from_fn(|i| mouth[i] + normal[i] * depth);
            (
                Boss {
                    start_mm,
                    end_mm,
                    radius_mm: boss.radius,
                    source: index + 1,
                    wall_source_face: 0,
                    cap_source_face: 1,
                },
                chart.vector(boss.radial),
            )
        })
        .collect();
    imported.shell = imported
        .shell
        .attach_bosses_with_reference(&tools, tol)
        .map_err(|cause| match cause {
            warped_union::Error::Limit => ReadError::Limit,
            _ => ReadError::Invalid,
        })?;
    if imported.shell.faces().len() != source.faces.len()
        || imported.shell.edges().len() != source.curves.len()
        || imported.shell.vertices().len() != reader.points.len()
    {
        return Err(ReadError::Invalid);
    }
    for (index, attachment) in attachments.iter().enumerate() {
        let host = &imported.shell.faces()[face_indices[&attachment.host]];
        let hole = &source.faces[attachment.host].holes[attachment.hole];
        let expected = host.holes.get(attachment.hole).ok_or(ReadError::Invalid)?;
        if !loop_matches(reader, source, chart, hole, &imported.shell, expected) {
            return Err(ReadError::Invalid);
        }
        let first = base.faces.len() + index * 5;
        for (original, expected) in attachment
            .walls
            .into_iter()
            .chain([attachment.cap])
            .zip(&imported.shell.faces()[first..first + 5])
        {
            let face = &source.faces[original];
            if !support_matches(face.surface, expected, chart, tol)
                || !loop_matches(
                    reader,
                    source,
                    chart,
                    &face.loop_,
                    &imported.shell,
                    &expected.boundary,
                )
            {
                return Err(ReadError::Invalid);
            }
        }
    }
    Ok(imported)
}

fn attachments(reader: &Shell<'_, '_>, source: &Boundary) -> Result<Vec<Attachment>> {
    let mut incidence = BTreeMap::<u32, Vec<usize>>::new();
    for (i, face) in source.faces.iter().enumerate() {
        for loop_ in std::iter::once(&face.loop_).chain(&face.holes) {
            for &(edge, _) in loop_ {
                incidence.entry(edge).or_default().push(i);
            }
        }
    }
    let adjacent = |edge, current| {
        let [a, b] = incidence[&edge].as_slice() else {
            return Err(ReadError::Invalid);
        };
        if *a != *b {
            return Ok(if *a == current { *b } else { *a });
        }
        Err(ReadError::Invalid)
    };
    let mut attachments = Vec::new();
    for (host, face) in source
        .faces
        .iter()
        .enumerate()
        .filter(|(_, face)| !face.holes.is_empty())
    {
        let Surface::Plane { normal, .. } = face.surface else {
            return Err(ReadError::Unsupported);
        };
        for (hole, ring) in face.holes.iter().enumerate() {
            let [a, b, c, d] = ring.as_slice() else {
                return Err(ReadError::Unsupported);
            };
            let entries = [*d, *c, *b, *a];
            let (ends, first) = source.curves[&d.0];
            let TrimCurve::Arc(first) = first else {
                return Err(ReadError::Unsupported);
            };
            let mouth = first.ellipse().center_mm();
            let radius = first.ellipse().semiaxes_mm()[0];
            let start = reader.points[&ends[usize::from(d.1)]];
            let radial = unit(sub(start, mouth))?;
            let mut walls = [0; 4];
            let mut top = None;
            for (i, (edge, _)) in entries.into_iter().enumerate() {
                let wall = adjacent(edge, host)?;
                walls[i] = wall;
                let side = &source.faces[wall];
                if !matches!(side.surface, Surface::Cylinder { .. }) || !side.holes.is_empty() {
                    return Err(ReadError::Unsupported);
                }
                let mut far = side
                    .loop_
                    .iter()
                    .filter_map(|&(id, _)| match source.curves[&id].1 {
                        TrimCurve::Arc(arc) if id != edge => Some((id, arc)),
                        _ => None,
                    });
                let Some((far_edge, arc)) = far.next() else {
                    return Err(ReadError::Unsupported);
                };
                if far.next().is_some() {
                    return Err(ReadError::Unsupported);
                }
                let cap = adjacent(far_edge, wall)?;
                let center = arc.ellipse().center_mm();
                if let Some((old, top)) = top {
                    if old != cap || length(sub(top, center)) > reader.tolerance {
                        return Err(ReadError::Invalid);
                    }
                } else {
                    top = Some((cap, center));
                }
            }
            let (cap, top) = top.expect("four certified candidate walls");
            let face = &source.faces[cap];
            if !matches!(face.surface, Surface::Plane { .. })
                || !face.holes.is_empty()
                || dot(sub(top, mouth), normal) <= reader.tolerance
                || length(cross(sub(top, mouth), normal)) > reader.tolerance
            {
                return Err(ReadError::Unsupported);
            }
            attachments.push(Attachment {
                host,
                hole,
                walls,
                cap,
                mouth,
                top,
                normal,
                radial,
                radius,
            });
        }
    }
    Ok(attachments)
}

fn loop_matches(
    reader: &Shell<'_, '_>,
    source: &Boundary,
    chart: Chart,
    loop_: &[(u32, bool)],
    shell: &warped_union::Shell,
    expected: &[OrientedEdge],
) -> bool {
    if loop_.len() != expected.len() {
        return false;
    }
    (0..expected.len()).any(|offset| {
        loop_
            .iter()
            .zip(expected.iter().cycle().skip(offset))
            .all(|(&(edge, forward), usage)| {
                let (vertices, curve) = source.curves[&edge];
                let actual_start = reader.points[&vertices[usize::from(!forward)]];
                let rebuilt = shell.edges()[usage.edge];
                let expected_start =
                    shell.vertices()[rebuilt.vertices[usize::from(!usage.forward)]];
                length(sub(chart.local(actual_start), expected_start)) <= reader.tolerance
                    && curve_matches(
                        curve,
                        forward,
                        rebuilt.curve,
                        usage.forward,
                        chart,
                        reader.tolerance,
                    )
            })
    })
}

#[cfg(test)]
mod tests;
