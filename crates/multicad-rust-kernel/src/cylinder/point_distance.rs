use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointDistance {
    pub point_mm: [f64; 3],
    pub distance_mm: f64,
    pub strictly_inside: bool,
}

impl AxialCylinder {
    /// Euclidean projection onto the closed occupied cylinder. Coordinates are
    /// in its placement chart: the lower cap center is zero and +Z is axial.
    /// Authored circle-center and bottom offsets are represented by that chart.
    pub fn closest_point_in_frame(&self, point: [f64; 3]) -> Result<PointDistance, Error> {
        if point.into_iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidLength);
        }
        let radius = self.circle().radius().as_f64();
        let height = self.height().as_f64();
        let [x, y, z] = point;
        let radial = x.hypot(y);
        // Select multiplication order per component: an extreme quotient can
        // lose precision, or discard a representable final small coordinate.
        let project = |coordinate: f64| {
            if radial <= radius {
                coordinate
            } else if coordinate.abs() > radius {
                radius * (coordinate / radial)
            } else {
                coordinate * (radius / radial)
            }
        };
        let closest = [project(x), project(y), z.clamp(0., height)];
        let delta: [f64; 3] = std::array::from_fn(|i| point[i] - closest[i]);
        let distance = delta[0].hypot(delta[1]).hypot(delta[2]);
        if !radial.is_finite() || !distance.is_finite() {
            return Err(Error::InvalidLength);
        }
        Ok(PointDistance {
            point_mm: closest,
            distance_mm: distance,
            strictly_inside: radial < radius && z > 0. && z < height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_projection_distinguishes_wall_caps_rims_and_occupied_interior() {
        let n = |n| Rational::new(n, 1).unwrap();
        let geometry =
            AxialCylinder::new(1, Circle::new([n(20), n(-40)], n(2)).unwrap(), n(7), n(5)).unwrap();
        for (point, expected, distance, inside) in [
            ([3., 4., 2.], [1.2, 1.6, 2.], 3., false),
            ([1., 0., -3.], [1., 0., 0.], 3., false),
            ([0., 1., 8.], [0., 1., 5.], 3., false),
            ([3., 4., 9.], [1.2, 1.6, 5.], 5., false),
            ([0., 0., 2.], [0., 0., 2.], 0., true),
            ([1., 0., 2.], [1., 0., 2.], 0., true),
            ([2., 0., 2.], [2., 0., 2.], 0., false),
            ([0., 0., 0.], [0., 0., 0.], 0., false),
        ] {
            let result = geometry.closest_point_in_frame(point).unwrap();
            assert!((result.distance_mm - distance).abs() < 1e-14);
            assert_eq!(result.strictly_inside, inside);
            for (a, b) in result.point_mm.into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-14);
            }
        }
        let tiny = AxialCylinder::new(
            2,
            Circle::new([n(0); 2], Rational::new(1, 1_000_000_000).unwrap()).unwrap(),
            n(0),
            n(5),
        )
        .unwrap();
        let far = tiny.closest_point_in_frame([1e308, 0., 2.]).unwrap();
        assert_eq!(far.point_mm, [1e-9, 0., 2.]);
        let large = AxialCylinder::new(
            3,
            Circle::new([n(0); 2], n(1_000_000_000)).unwrap(),
            n(0),
            n(1),
        )
        .unwrap();
        let far = large.closest_point_in_frame([1e-20, 1e308, 2.]).unwrap();
        assert!(far.point_mm[0] > 0.);
        assert!((far.point_mm[0] / 1e-319 - 1.).abs() < 1e-4);
        assert_eq!(far.point_mm[1], 1e9);
        for point in [[f64::NAN, 0., 0.], [0., f64::INFINITY, 0.], [f64::MAX; 3]] {
            assert_eq!(
                geometry.closest_point_in_frame(point),
                Err(Error::InvalidLength)
            );
        }
    }
}
