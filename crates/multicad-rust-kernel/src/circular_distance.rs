//! Exact coaxial circle distance with a finite representative of its family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoaxialDistance {
    pub distance_mm: f64,
    pub a_mm: [f64; 3],
    pub b_mm: [f64; 3],
}

/// Circles are in parallel XY planes with a common Z axis. A representative
/// nearest pair uses +X; this never asserts a unique closest direction.
pub fn coaxial_circles(a: f64, b: f64, axial_offset: f64) -> Option<CoaxialDistance> {
    if !a.is_finite() || !b.is_finite() || !axial_offset.is_finite() || a <= 0. || b <= 0. {
        return None;
    }
    let distance_mm = (a - b).hypot(axial_offset);
    distance_mm.is_finite().then_some(CoaxialDistance {
        distance_mm,
        a_mm: [a, 0., 0.],
        b_mm: [b, 0., axial_offset],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn radial_axial_and_coincident_families_keep_actual_distance_witnesses() {
        for (a, b, dz, expected) in [
            (5., 10., 0., 5.),
            (2., 5., 4., 5.),
            (5., 2., -4., 5.),
            (3., 3., 7., 7.),
            (3., 3., 0., 0.),
        ] {
            let result = coaxial_circles(a, b, dz).unwrap();
            assert_eq!(result.distance_mm, expected);
            assert_eq!(result.a_mm, [a, 0., 0.]);
            assert_eq!(result.b_mm, [b, 0., dz]);
            assert_eq!(
                (result.a_mm[0] - result.b_mm[0]).hypot(result.a_mm[2] - result.b_mm[2]),
                expected
            );
        }
    }
    #[test]
    fn invalid_radii_offsets_and_unrepresentable_distance_are_refused() {
        for value in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(coaxial_circles(value, 1., 0.).is_none());
            assert!(coaxial_circles(1., value, 0.).is_none());
        }
        for value in [f64::NAN, f64::INFINITY] {
            assert!(coaxial_circles(1., 1., value).is_none());
        }
        assert!(coaxial_circles(f64::MAX, 1., f64::MAX).is_none());
    }
}
