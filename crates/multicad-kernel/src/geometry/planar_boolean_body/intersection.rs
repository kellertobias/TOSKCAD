//! Exact finite-boundary exclusion for a rectangular probe filling a closed void.
use super::*;

const MAX_MATCH_WORK: usize = 256_000;
type PointKey = [u64; 3];
type FaceKey = (PointKey, Vec<PointKey>);

fn charge(budget: &mut usize, amount: usize) -> Result<(), GeometryError> {
    *budget = budget
        .checked_sub(amount)
        .ok_or_else(|| planar_error(Error::Limit))?;
    Ok(())
}
fn point_key(point: Point) -> PointKey {
    point.map(|value| if value == 0. { 0 } else { value.to_bits() })
}
fn face_key(
    geometry: &Union,
    face: &PureFace,
    reverse: bool,
    budget: &mut usize,
) -> Result<FaceKey, GeometryError> {
    charge(budget, face.vertices.len() + 1)?;
    let mut points: Vec<_> = face
        .vertices
        .iter()
        .map(|&i| point_key(geometry.vertices()[i]))
        .collect();
    if reverse {
        points.reverse();
    }
    let first = points
        .iter()
        .enumerate()
        .min_by_key(|(_, point)| *point)
        .expect("certified nonempty face")
        .0;
    points.rotate_left(first);
    Ok((
        point_key(
            face.normal
                .map(|value| if reverse { -value } else { value }),
        ),
        points,
    ))
}

fn one_direction(
    solid: &Solid,
    probe: &Solid,
    policy: GeometryPolicy,
    budget: &mut usize,
) -> Result<bool, GeometryError> {
    let Some((body, frame)) = solid.planar_boolean_body() else {
        return Ok(false);
    };
    // A positive island nested in the void is another solid and must remain
    // occupied. Only the already-certified one-solid boundary qualifies.
    if frame != PlaneFrame::default()
        || body.geometry.solid_count() != 1
        || body.geometry.shell_count() < 2
    {
        return Ok(false);
    }
    let Some(prism) = probe.rust_prism() else {
        return Ok(false);
    };
    if prism.rounded_end().is_some() || prism.inner_loops().next().is_some() {
        return Ok(false);
    }
    let Some(snapshot) = probe.rust_topology() else {
        return Ok(false);
    };
    let snapshot = snapshot?;
    let Some(parts) = probe.planar_convex_parts(&snapshot, policy) else {
        return Ok(false);
    };
    let parts = parts?;
    if parts.len() != 1 {
        return Ok(false);
    }
    let query = Union::new(
        &[parts.into_iter().next().expect("one query cell").0],
        policy.linear_tolerance_mm,
    )
    .map_err(planar_error)?;
    if query.solid_count() != 1 || query.shell_count() != 1 || query.faces().len() != 6 {
        return Ok(false);
    }
    let expected = query
        .faces()
        .iter()
        .map(|face| face_key(&query, face, false, budget))
        .collect::<Result<BTreeSet<_>, _>>()?;
    for shell in body.geometry.shells() {
        charge(budget, 1)?;
        if shell.signed_volume_mm3 >= 0. || shell.faces.len() != expected.len() {
            continue;
        }
        let actual = shell
            .faces
            .iter()
            .map(|&i| face_key(&body.geometry, &body.geometry.faces()[i], true, budget))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if actual == expected {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Literal finite rings and opposite support normals prove the query equals
/// one inward shell. No metric tolerance, hull or source-label guess is used.
pub(super) fn excluded_by_closed_void(
    left: &Solid,
    right: &Solid,
    policy: GeometryPolicy,
) -> Result<bool, GeometryError> {
    let mut budget = MAX_MATCH_WORK;
    Ok(one_direction(left, right, policy, &mut budget)?
        || one_direction(right, left, policy, &mut budget)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make(size: [i64; 3], origin: [f64; 3], pose: RigidTransform) -> Solid {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|v| Rational::new(v, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            GeometryPolicy::default(),
        )
        .unwrap()
        .transformed(pose)
        .unwrap()
    }
    fn pose() -> RigidTransform {
        RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm: [80., 60., 20.],
        }
    }
    #[test]
    fn authoritative_inward_shell_equals_the_exact_posed_tool_boundary() {
        let policy = GeometryPolicy::default();
        let stock = make([15, 10, 8], [0.; 3], pose());
        let probe = make([5, 4, 3], [2., 3., 1.], pose());
        let cavity = stock
            .rust_subtract_planar_tools(&[&probe], EntityId::new(), policy)
            .unwrap()
            .unwrap();
        assert!(excluded_by_closed_void(&cavity, &probe, policy).unwrap());
        assert!(excluded_by_closed_void(&probe, &cavity, policy).unwrap());
        let (body, _) = cavity.planar_boolean_body().unwrap();
        let mut budget = 0;
        assert_eq!(
            one_direction(&cavity, &probe, policy, &mut budget)
                .unwrap_err()
                .kind,
            GeometryErrorKind::Limit
        );
        assert!(excluded_by_closed_void(&cavity, &probe, policy).unwrap());
        assert_eq!(body.geometry.shell_count(), 2);
    }
}
