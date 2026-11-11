//! Independently validate the two closed shells of a rectangular cavity BRep.
use super::*;
use crate::{orthogonal::OrientedFace, orthogonal_boundary::box_void};

pub(super) fn read_member(
    shell: &Shell<'_, '_>,
    member: u32,
) -> Result<Option<(Readback, ImportSolid)>> {
    if shell.simple(member)?.name != "BREP_WITH_VOIDS" {
        return Ok(None);
    }
    let [Value::String(_), outer, voids] = shell.args(member, "BREP_WITH_VOIDS")? else {
        return Err(ReadError::Invalid);
    };
    let voids = refs(voids)?;
    let [void] = voids.as_slice() else {
        return Err(ReadError::Unsupported);
    };
    let [Value::String(_), Value::Derived, inner, orientation] =
        shell.args(*void, "ORIENTED_CLOSED_SHELL")?
    else {
        return Err(ReadError::Invalid);
    };
    let inner = reference(inner)?;
    let outer = reference(outer)?;
    if outer == inner {
        return Err(ReadError::Invalid);
    }
    let orientations = [true, sense(orientation)?];
    let mut physical = Vec::new();
    let mut groups = [Vec::new(), Vec::new()];
    let mut owned = BTreeSet::new();
    let mut edges_mm = Vec::new();
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for (group, id) in [outer, inner].into_iter().enumerate() {
        let [Value::String(_), faces] = shell.args(id, "CLOSED_SHELL")? else {
            return Err(ReadError::Invalid);
        };
        let face_ids = refs(faces)?;
        let mut raw = Shell::new(shell.entities, shell.tolerance, false);
        let faces = raw.validate_faces(face_ids.clone())?;
        for id in face_ids
            .into_iter()
            .chain(raw.edges.keys().copied())
            .chain(raw.points.keys().copied())
        {
            if !owned.insert(id) {
                return Err(ReadError::Invalid);
            }
        }
        let mut successors = vec![BTreeMap::new(); faces.len()];
        for edge in raw.edges.values() {
            edges_mm.push(edge.vertices.map(|id| raw.points[&id]));
            for use_ in &edge.uses {
                let [a, b] = edge.vertices;
                let (start, end) = if use_.forward { (a, b) } else { (b, a) };
                successors[use_.face].insert(start, end);
            }
        }
        for mut next in successors {
            let &start = next.keys().next().ok_or(ReadError::Invalid)?;
            let mut at = start;
            let mut ring = Vec::with_capacity(next.len());
            loop {
                let point = raw.points[&at];
                include_point(&mut bounds, point);
                ring.push(point);
                at = next.remove(&at).ok_or(ReadError::Invalid)?;
                if at == start {
                    break;
                }
            }
            if !next.is_empty() {
                return Err(ReadError::Unsupported);
            }
            let anchor = ring[0];
            let normal =
                ring.iter()
                    .zip(ring.iter().cycle().skip(1))
                    .fold([0.; 3], |sum, (&a, &b)| {
                        let n = cross(sub(a, anchor), sub(b, anchor));
                        std::array::from_fn(|i| sum[i] + n[i])
                    });
            // The plane parser already proved this ring has positive area.
            // Any nonrepresentable normalization is rejected by the enclosure proof.
            let norm = length(normal);
            let mut normal = normal.map(|component| component / norm);
            if !orientations[group] {
                ring.reverse();
                normal = normal.map(|n| -n);
            }
            groups[group].push(physical.len());
            physical.push(OrientedFace {
                normal,
                rings: vec![ring],
            });
        }
    }
    let certificate =
        box_void::recognize(&physical, shell.tolerance).ok_or(ReadError::Unsupported)?;
    if certificate.groups != groups {
        return Err(ReadError::Invalid);
    }
    let volume_mm3 = certificate.solid.volume_mm3();
    let height_mm = certificate.solid.bounds().max[2] - certificate.solid.bounds().min[2];
    let imported = ImportSolid {
        origin_mm: certificate.origin,
        u: certificate.axes[0],
        v: certificate.axes[1],
        height_mm,
        profile: ImportProfile::Orthogonal {
            solid: certificate.solid,
        },
    };
    Ok(Some((
        Readback {
            bounds_mm: bounds,
            volume_mm3,
            face_count: physical.len(),
            edges_mm,
            circles: Vec::new(),
            semicircles: Vec::new(),
            arcs: Vec::new(),
            ellipses: Vec::new(),
            splines: Vec::new(),
            solid_count: 1,
        },
        imported,
    )))
}

#[cfg(test)]
mod tests;
