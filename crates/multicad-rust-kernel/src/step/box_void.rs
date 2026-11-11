//! One logical planar solid with strictly enclosed rectangular cavity shells.
use super::*;
use crate::{orthogonal::OrientedFace, orthogonal_boundary::box_void};

pub(super) fn encode(
    file: &mut StepFile,
    faces: &[OrientedFace],
    transform: &dyn Fn(Point) -> Point,
) -> Result<Entity, StepError> {
    let mut placed = Vec::with_capacity(faces.len());
    for face in faces {
        let ring = face.rings[0]
            .iter()
            .copied()
            .map(transform)
            .collect::<Vec<_>>();
        let anchor = ring[0];
        let normal = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .fold([0.; 3], |sum, (&a, &b)| {
                let n = cross(
                    std::array::from_fn(|i| a[i] - anchor[i]),
                    std::array::from_fn(|i| b[i] - anchor[i]),
                );
                std::array::from_fn(|i| sum[i] + n[i])
            });
        let length = dot(normal, normal).sqrt();
        if !length.is_finite() || length == 0. {
            return Err(StepError::InvalidGeometry);
        }
        placed.push(OrientedFace {
            normal: normal.map(|n| n / length),
            rings: vec![ring],
        });
    }
    let certificate = box_void::certify(&placed, 0.).map_err(|error| match error {
        crate::orthogonal::BooleanError::Limit => StepError::Limit,
        _ => StepError::InvalidGeometry,
    })?;
    let mut shells = Vec::new();
    for (group, indices) in certificate.groups.iter().enumerate() {
        let mut points = Vec::new();
        let mut keys = BTreeMap::new();
        for &i in indices {
            for &point in &placed[i].rings[0] {
                keys.entry(point_key(point)).or_insert_with(|| {
                    let i = points.len();
                    points.push(point);
                    i
                });
            }
        }
        let mut brep = BRep::new(file, points);
        let mut emitted = Vec::new();
        for &i in indices {
            let face = &placed[i];
            let mut ring = face.rings[0]
                .iter()
                .map(|&p| keys[&point_key(p)])
                .collect::<Vec<_>>();
            let normal = if group == 0 {
                face.normal
            } else {
                ring.reverse();
                face.normal.map(|n| -n)
            };
            let boundary = brep.bound(&ring, true)?;
            let point = brep.vertices[ring[0]].cartesian;
            let (&a, &b) = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .max_by(|&(&a, &b), &(&c, &d)| {
                    let length = |a: usize, b: usize| {
                        let delta = std::array::from_fn(|i| {
                            brep.vertices[b].point[i] - brep.vertices[a].point[i]
                        });
                        dot(delta, delta)
                    };
                    length(a, b).total_cmp(&length(c, d))
                })
                .expect("certified nonempty face");
            let tangent = direction(brep.vertices[a].point, brep.vertices[b].point)?;
            emitted.push(brep.file.face(&[boundary], point, normal, tangent));
        }
        shells.push(
            brep.file
                .entity(format_args!("CLOSED_SHELL('',{})", Entities(&emitted))),
        );
    }
    let voids = shells[1..]
        .iter()
        .map(|shell| file.entity(format_args!("ORIENTED_CLOSED_SHELL('',*,{shell},.F.)")))
        .collect::<Vec<_>>();
    Ok(file.entity(format_args!(
        "BREP_WITH_VOIDS('',{},{})",
        shells[0],
        Entities(&voids)
    )))
}

pub(super) fn union_faces(union: &crate::planar_boolean::Union) -> Vec<OrientedFace> {
    union
        .faces()
        .iter()
        .map(|face| OrientedFace {
            normal: face.normal,
            rings: vec![face.vertices.iter().map(|&i| union.vertices()[i]).collect()],
        })
        .collect()
}

#[cfg(test)]
mod tests;
