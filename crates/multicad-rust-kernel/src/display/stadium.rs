use super::*;
use crate::stadium::Stadium;

pub(super) struct StadiumRing {
    pub points: Vec<[f64; 2]>,
    pub sides: Vec<u32>,
}

impl StadiumRing {
    pub(super) fn new(profile: &Stadium, options: Tessellation) -> Result<Self, MeshError> {
        let radius = profile.radius_mm();
        let arc_count = options.arc_segments(radius, PI)?;
        let count = 2 * arc_count + 2;
        if count > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        let corners = profile.corners();
        let centers = profile.centers();
        let mut points = Vec::with_capacity(count);
        let mut sides = Vec::with_capacity(count);
        for (i, corner) in corners.into_iter().enumerate() {
            points.push(corner);
            sides.push(i as u32);
            if i % 2 == 1 {
                let [cx, cy] = centers[usize::from(i == 1)];
                let [x, y] = [corner[0] - cx, corner[1] - cy];
                for j in 1..arc_count {
                    let (sin, cos) = (PI * j as f64 / arc_count as f64).sin_cos();
                    points.push([cx + x * cos - y * sin, cy + x * sin + y * cos]);
                    sides.push(i as u32);
                }
            }
        }
        Ok(Self { points, sides })
    }
}

/// Tessellate a rounded slot extruded normal to its profile. The caps are
/// faces 0 and 1, followed by the four original profile sides in native order.
pub fn stadium_prism(
    profile: &Stadium,
    height: f64,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    if !height.is_finite() || height <= 0. {
        return Err(MeshError::UnresolvedBoundary);
    }
    let StadiumRing { points, sides } = StadiumRing::new(profile, options)?;
    let mut footprint = Footprint::default();
    footprint.ring(
        points
            .into_iter()
            .zip(sides.into_iter().map(|side| side + 2)),
        options,
    )?;
    footprint.tessellate(0., height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manufacturing::certify;

    #[test]
    fn stadium_mesh_is_watertight_and_preserves_native_face_order() {
        let profile = Stadium::new([[2., 3.], [8., 3.]], 1.5).unwrap();
        let options = Tessellation {
            deflection: 0.001,
            angle: 0.2,
            triangle_limit: 20_000,
        };
        let mesh = stadium_prism(&profile, 4., options).unwrap();
        let proof = certify(&mesh).unwrap();
        assert!((proof.signed_volume_mm3 - 4. * profile.area_mm2()).abs() < 0.04);
        let counts = std::array::from_fn::<_, 6, _>(|i| {
            mesh.face_indices
                .iter()
                .filter(|&&face| face as usize == i)
                .count()
        });
        assert_eq!(counts[0], counts[1]);
        assert_eq!(counts[2], 2);
        assert_eq!(counts[4], 2);
        assert_eq!(counts[3], counts[5]);
        assert!(counts[3] > 10);

        assert_eq!(
            stadium_prism(&profile, 0., options),
            Err(MeshError::UnresolvedBoundary)
        );
        for limit in [10, 60, mesh.face_indices.len() - 1] {
            assert_eq!(
                stadium_prism(
                    &profile,
                    4.,
                    Tessellation {
                        triangle_limit: limit,
                        ..options
                    }
                ),
                Err(MeshError::BudgetExceeded)
            );
        }
    }
}
