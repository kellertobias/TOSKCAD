//! Conservative identities and datum charts for finite imported planar faces.
use super::*;
use multicad_core::project_file::ContentHash;

fn zero(value: f64) -> f64 {
    if value == 0. { 0. } else { value }
}

pub(super) fn labels(
    geometry: &Union,
    feature: EntityId,
) -> Result<Vec<FaceLabels>, GeometryError> {
    labels_with_prefix(geometry, feature, "imported-plane.v1.")
}

pub(super) fn labels_with_prefix(
    geometry: &Union,
    feature: EntityId,
    prefix: &str,
) -> Result<Vec<FaceLabels>, GeometryError> {
    geometry
        .faces()
        .iter()
        .map(|face| {
            let mut points: Vec<_> = face
                .vertices
                .iter()
                .map(|&i| geometry.vertices()[i].map(zero))
                .collect();
            let start = (0..points.len())
                .min_by(|&a, &b| {
                    (0..3)
                        .map(|i| points[a][i].total_cmp(&points[b][i]))
                        .find(|order| !order.is_eq())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("certified nonempty imported face");
            points.rotate_left(start);
            let origin = points[0];
            let area = points.windows(2).fold([0.; 3], |sum, pair| {
                let area = cross(subtract(pair[0], origin), subtract(pair[1], origin));
                std::array::from_fn(|i| sum[i] + area[i])
            });
            let length = norm(area);
            if !length.is_finite() || length == 0. {
                return Err(planar_error(Error::Invalid));
            }
            let normal = area.map(|value| zero(value / length));
            let support = zero(dot(normal, origin));
            let mut bytes = b"multicad.imported-planar-face.v1\0".to_vec();
            for value in normal.into_iter().chain([support]) {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            bytes.extend_from_slice(&(points.len() as u64).to_le_bytes());
            for point in &points {
                for value in point {
                    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
            let origin_id = Origin {
                source: feature,
                role: format!("{prefix}{}", ContentHash::of(&bytes)),
            };
            // The finite boundary is part of the identity. A changed region,
            // including a coplanar replacement, needs explicit reference repair.
            // No declaration ordinal or support-only alias is published.
            let axis = (0..3)
                .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()).then(a.cmp(&b)))
                .expect("three axes");
            let tangent: Point = std::array::from_fn(|i| {
                (if i == axis { 1. } else { 0. }) - normal[i] * normal[axis]
            });
            let length = norm(tangent);
            let u = tangent.map(|value| zero(value / length));
            let v = cross(normal, u).map(zero);
            let anchor = ReferenceFrame {
                origin_mm: origin,
                u,
                v,
            };
            anchor
                .validate()
                .map_err(|_| planar_error(Error::Invalid))?;
            Ok((
                vec![origin_id.clone()],
                vec![AnchoredFrame {
                    origin: origin_id,
                    frame: anchor,
                }],
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swept(points: &[[f64; 2]], bottom: f64) -> Union {
        let base: Vec<_> = points.iter().map(|&[x, y]| [x, y, bottom]).collect();
        let end: Vec<_> = points.iter().map(|&[x, y]| [x, y, bottom + 2.]).collect();
        let mut faces = vec![base.iter().copied().rev().collect(), end.clone()];
        for i in 0..base.len() {
            let j = (i + 1) % base.len();
            faces.push(vec![base[i], base[j], end[j], end[i]]);
        }
        Union::from_planar_rings(&faces, 1e-7).unwrap()
    }

    #[test]
    fn finite_concave_region_not_its_hull_and_material_side_define_identity() {
        let owner = EntityId::new();
        let points = [
            [0., 0.],
            [6., 0.],
            [6., 5.],
            [4., 5.],
            [4., 2.],
            [2., 2.],
            [2., 5.],
            [0., 5.],
        ];
        let source = swept(&points, 0.);
        let original = labels(&source, owner).unwrap();
        let mut rotated = points;
        rotated.rotate_left(3);
        let reordered = labels(&swept(&rotated, 0.), owner).unwrap();
        assert_eq!(original[0], reordered[0]);
        assert_eq!(original[1], reordered[1]);
        let signed_zero =
            points.map(|point| point.map(|value| if value == 0. { -0. } else { value }));
        assert_eq!(
            original[0],
            labels(&swept(&signed_zero, 0.), owner).unwrap()[0]
        );
        let mut changed = points;
        changed[4][1] = 1.;
        changed[5][1] = 1.;
        let changed = labels(&swept(&changed, 0.), owner).unwrap();
        assert_ne!(original[0].0, changed[0].0);
        let other_side = labels(&swept(&points, -2.), owner).unwrap();
        assert_ne!(original[0].0, other_side[1].0);
        assert!(
            original
                .iter()
                .flat_map(|label| &label.0)
                .all(|origin| origin.role.starts_with("imported-plane.v1."))
        );
        assert_eq!(source.volume_mm3(), 48.);
    }
}
