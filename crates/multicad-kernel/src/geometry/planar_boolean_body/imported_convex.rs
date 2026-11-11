//! On-demand occupied material from a certified, unchanged convex boundary.
use super::*;

impl Solid {
    pub(super) fn imported_convex_parts(
        &self,
        topology: &TopologySnapshot,
        policy: GeometryPolicy,
    ) -> Option<Result<Vec<ConvexPart>, GeometryError>> {
        let (body, frame) = self.planar_boolean_body()?;
        let proof = body.imported_convex_piece(policy.linear_tolerance_mm)?;
        Some(proof.and_then(|piece| {
            let indices: BTreeMap<_, _> = topology
                .elements
                .iter()
                .enumerate()
                .filter(|(_, element)| element.key.kind == TopologyKind::Face)
                .map(|(index, element)| (element.key.ordinal as usize, index))
                .collect();
            let faces = (0..body.geometry.faces().len())
                .map(|ordinal| {
                    indices
                        .get(&ordinal)
                        .copied()
                        .map(Some)
                        .ok_or_else(|| planar_error(Error::Invalid))
                })
                .collect::<Result<_, _>>()?;
            let mut pieces =
                PlanarBooleanBody::placed_pieces(&[piece], frame, policy.linear_tolerance_mm)?;
            Ok(vec![(
                pieces.pop().expect("one certified convex piece"),
                faces,
            )])
        }))
    }
}

impl PlanarBooleanBody {
    pub(super) fn imported_convex_piece(
        &self,
        tolerance: f64,
    ) -> Option<Result<ConvexPiece, GeometryError>> {
        if self.material.is_some()
            || self.geometry.solid_count() != 1
            || self.geometry.shell_count() != 1
        {
            return None;
        }
        let vertices = self.geometry.vertices();
        let mut remaining = 24_000_000usize;
        for face in self.geometry.faces() {
            let cost = vertices
                .len()
                .saturating_add(face.vertices.len().saturating_mul(face.vertices.len()));
            let Some(next) = remaining.checked_sub(cost) else {
                return Some(Err(planar_error(Error::Limit)));
            };
            remaining = next;
            let point = vertices[face.vertices[0]];
            // A valid nonconvex shell is outside this provider, not an invalid
            // attempt to manufacture its convex hull.
            if vertices
                .iter()
                .any(|&other| dot(face.normal, subtract(other, point)) > tolerance)
            {
                return None;
            }
            for (&a, &b) in face
                .vertices
                .iter()
                .zip(face.vertices.iter().cycle().skip(1))
            {
                let tangent = subtract(vertices[b], vertices[a]);
                let length = norm(tangent);
                if length <= tolerance {
                    return Some(Err(planar_error(Error::Invalid)));
                }
                let inward = cross(face.normal, tangent).map(|v| v / length);
                if face
                    .vertices
                    .iter()
                    .any(|&i| dot(inward, subtract(vertices[i], vertices[a])) < -tolerance)
                {
                    return None;
                }
            }
        }
        Some((|| {
            let rings = self
                .geometry
                .faces()
                .iter()
                .map(|face| face.vertices.iter().map(|&i| vertices[i]).collect())
                .collect();
            let piece = ConvexPiece::new(rings, tolerance).map_err(planar_error)?;
            let proof =
                Union::new(std::slice::from_ref(&piece), tolerance).map_err(planar_error)?;
            // Preserve the exact physical vertices and edges, including any
            // authored subdivisions. The proof never replaces this boundary.
            if proof.solid_count() != 1
                || proof.shell_count() != 1
                || (proof.volume_mm3() - self.geometry.volume_mm3()).abs()
                    > self.geometry.volume_mm3() * 1e-8
                || boundary_keys(&proof) != boundary_keys(&self.geometry)
            {
                return Err(planar_error(Error::Invalid));
            }
            Ok(piece)
        })())
    }
}

type PointKey = [u64; 3];
type EdgeKey = [PointKey; 2];
fn boundary_keys(geometry: &Union) -> (BTreeSet<PointKey>, BTreeSet<EdgeKey>) {
    let key = |point: Point| point.map(|v| if v == 0. { 0 } else { v.to_bits() });
    let vertices = geometry.vertices().iter().copied().map(key).collect();
    let edges = geometry
        .faces()
        .iter()
        .flat_map(|face| {
            face.vertices
                .iter()
                .zip(face.vertices.iter().cycle().skip(1))
                .map(|(&a, &b)| {
                    let mut edge = [key(geometry.vertices()[a]), key(geometry.vertices()[b])];
                    edge.sort_unstable();
                    edge
                })
        })
        .collect();
    (vertices, edges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_convex_factory_limits_propagate_without_changing_imported_boundary() {
        let policy = GeometryPolicy::default();
        let base: Vec<_> = (-64..=64).map(|i| [i as f64, (i * i) as f64, 0.]).collect();
        let upper: Vec<_> = base.iter().map(|&[x, y, _]| [x, y, 1.]).collect();
        let mut rings = vec![base.iter().copied().rev().collect(), upper.clone()];
        for i in 0..base.len() {
            let j = (i + 1) % base.len();
            rings.push(vec![base[i], base[j], upper[j], upper[i]]);
        }
        let geometry = Union::from_planar_rings(&rings, policy.linear_tolerance_mm).unwrap();
        let owner = EntityId::new();
        let source = Solid::from_imported_planar_boundary(geometry, owner, policy).unwrap();
        let before = source.topology_snapshot(owner, 0).unwrap();
        let tool = Solid::from_imported_planar_boundary(
            Union::new(
                &[ConvexPiece::from_box([0.; 3], [1.; 3], policy.linear_tolerance_mm).unwrap()],
                policy.linear_tolerance_mm,
            )
            .unwrap(),
            EntityId::new(),
            policy,
        )
        .unwrap();
        assert_eq!(
            source
                .rust_subtract_planar_tools(&[&tool], EntityId::new(), policy)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Limit
        );
        assert_eq!(
            Solid::union_planar_pieces(&[&source, &tool], EntityId::new(), policy)
                .unwrap()
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Limit
        );
        assert_eq!(
            source
                .rust_planar_intersection_metrics(&tool, policy)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Limit
        );
        assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
        assert!(source.planar_boolean_body().unwrap().0.material.is_none());
    }
}
