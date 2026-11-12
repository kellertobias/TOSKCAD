//! Exact sphere metrics and bounded tessellation in its local frame.

use crate::display::{DisplayMesh, MeshError, Tessellation};
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI, TAU};

#[derive(Clone, Copy, Debug)]
pub struct Sphere {
    radius_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointProjection {
    pub point_mm: [f64; 3],
    pub distance_mm: f64,
    /// A surface query at the centre has a continuum of nearest points.
    pub unique: bool,
    pub inside: bool,
}

impl Sphere {
    /// Project onto the closed occupied sphere, in its centre-relative chart.
    pub fn closest_point(self, point: [f64; 3]) -> Option<PointProjection> {
        self.project(point, false)
    }

    /// Project onto the complete spherical face rather than occupied material.
    pub fn closest_surface_point(self, point: [f64; 3]) -> Option<PointProjection> {
        self.project(point, true)
    }

    fn project(self, point: [f64; 3], surface: bool) -> Option<PointProjection> {
        if point.into_iter().any(|x| !x.is_finite()) {
            return None;
        }
        let radial = point[0].hypot(point[1]).hypot(point[2]);
        if !radial.is_finite() {
            return None;
        }
        let inside = self.radius_mm - radial > 64. * f64::EPSILON * self.radius_mm.max(radial);
        if !surface && radial <= self.radius_mm {
            return Some(PointProjection {
                point_mm: point,
                distance_mm: 0.,
                unique: true,
                inside,
            });
        }
        if radial == 0. {
            return Some(PointProjection {
                point_mm: [self.radius_mm, 0., 0.],
                distance_mm: self.radius_mm,
                unique: false,
                inside,
            });
        }
        let projected = if radial < 1. {
            // A subnormal norm can have large relative error. Normalize a
            // rescaled vector, then expand without an overflowing R/norm.
            let scale = point.into_iter().map(f64::abs).fold(0., f64::max);
            let scaled = point.map(|x| x / scale);
            let factor = self.radius_mm / scaled[0].hypot(scaled[1]).hypot(scaled[2]);
            scaled.map(|x| x * factor)
        } else {
            point.map(|coordinate| {
                // Choose the multiplication order per component to preserve
                // representable small coordinates beside very distant probes.
                if coordinate.abs() > self.radius_mm {
                    self.radius_mm * (coordinate / radial)
                } else {
                    coordinate * (self.radius_mm / radial)
                }
            })
        };
        Some(PointProjection {
            point_mm: projected,
            distance_mm: (radial - self.radius_mm).abs(),
            unique: true,
            inside,
        })
    }
    pub fn new(radius_mm: f64) -> Option<Self> {
        ((1e-6..=1e7).contains(&radius_mm)).then_some(Self { radius_mm })
    }

    pub fn radius_mm(self) -> f64 {
        self.radius_mm
    }

    pub fn surface_area_mm2(self) -> f64 {
        4. * PI * self.radius_mm.powi(2)
    }

    pub fn volume_mm3(self) -> f64 {
        self.surface_area_mm2() * self.radius_mm / 3.
    }

    pub fn mesh(self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let r = self.radius_mm;
        // The spherical chart metric is bounded by its Euclidean parameter
        // metric. This limits every cell corner to half the allowed arc
        // angle about the cell midpoint, including the collapsed pole rows.
        let step = options.maximum_arc_angle(r) * FRAC_1_SQRT_2;
        let columns = (TAU / step).ceil() as usize;
        let rows = (PI / step).ceil() as usize;
        if columns > options.triangle_limit || rows > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        let columns = columns.next_multiple_of(4);
        let rows = rows.next_multiple_of(2);
        let triangles = 2_u64 * columns as u64 * (rows - 1) as u64;
        if triangles > options.triangle_limit as u64 {
            return Err(MeshError::BudgetExceeded);
        }
        let triangles = triangles as usize;
        let north = 1 + (rows - 1) * columns;
        let mut mesh = DisplayMesh {
            vertices: Vec::with_capacity((north + 1) * 3),
            indices: Vec::with_capacity(triangles * 3),
            face_indices: Vec::with_capacity(triangles),
        };
        mesh.vertices.extend([0., 0., -r]);
        let meridians: Vec<_> = (0..columns)
            .map(|column| (TAU * column as f64 / columns as f64).sin_cos())
            .collect();
        for row in 1..rows {
            let (sin, cos) = (PI * row as f64 / rows as f64 - FRAC_PI_2).sin_cos();
            let xy = r * cos;
            let z = r * sin;
            for &(sin, cos) in &meridians {
                mesh.vertices.extend([xy * cos, xy * sin, z]);
            }
        }
        mesh.vertices.extend([0., 0., r]);
        for column in 0..columns {
            let a = 1 + column;
            let b = 1 + (column + 1) % columns;
            mesh.triangle([0, b, a], 0);
            for band in 0..rows - 2 {
                let a = a + band * columns;
                let b = b + band * columns;
                mesh.triangle([a, b, b + columns], 0);
                mesh.triangle([a, b + columns, a + columns], 0);
            }
            mesh.triangle(
                [a + (rows - 2) * columns, b + (rows - 2) * columns, north],
                0,
            );
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manufacturing;

    #[test]
    fn point_projection_distinguishes_material_surface_and_nonunique_centre() {
        let sphere = Sphere::new(2.).unwrap();
        for (point, expected, distance, inside) in [
            ([3., 4., 0.], [1.2, 1.6, 0.], 3., false),
            ([0., 0., 3.], [0., 0., 2.], 1., false),
            ([0., 1., 0.], [0., 2., 0.], 1., true),
            ([2., 0., 0.], [2., 0., 0.], 0., false),
            ([1e-300, 0., 0.], [2., 0., 0.], 2., true),
        ] {
            let surface = sphere.closest_surface_point(point).unwrap();
            assert!(surface.unique);
            assert_eq!(surface.inside, inside);
            assert!((surface.distance_mm - distance).abs() < 1e-14);
            for (a, b) in surface.point_mm.into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-14);
            }
            let material = sphere.closest_point(point).unwrap();
            assert_eq!(material.inside, inside);
            assert_eq!(material.distance_mm, if inside { 0. } else { distance });
        }
        let centre = sphere.closest_surface_point([0.; 3]).unwrap();
        assert!(!centre.unique && centre.inside);
        assert_eq!(centre.distance_mm, 2.);
        assert_eq!(centre.point_mm, [2., 0., 0.]);
        let occupied = sphere.closest_point([0.; 3]).unwrap();
        assert!(occupied.unique && occupied.inside);
        assert_eq!(occupied.distance_mm, 0.);
        let tiny = f64::from_bits(1);
        let subnormal = sphere.closest_surface_point([tiny, tiny, 0.]).unwrap();
        assert!((subnormal.point_mm[0].hypot(subnormal.point_mm[1]) - 2.).abs() < 1e-14);
        let large = Sphere::new(1e7).unwrap();
        let far = large.closest_surface_point([1e-20, 1e308, 0.]).unwrap();
        assert!(far.point_mm[0] > 0. && far.point_mm[1] == 1e7);
        for invalid in [[f64::NAN, 0., 0.], [f64::INFINITY; 3], [f64::MAX; 3]] {
            assert!(sphere.closest_point(invalid).is_none());
            assert!(sphere.closest_surface_point(invalid).is_none());
        }
    }

    #[test]
    fn exact_metrics_and_closed_mesh_within_the_chord_bound() {
        let sphere = Sphere::new(20.).unwrap();
        let options = Tessellation {
            deflection: 0.01,
            angle: 0.2,
            triangle_limit: 200_000,
        };
        let mesh = sphere.mesh(options).unwrap();
        assert_eq!(sphere.radius_mm(), 20.);
        assert_eq!(sphere.surface_area_mm2(), 1600. * PI);
        assert!(mesh.face_indices.iter().all(|&face| face == 0));
        assert_eq!(&mesh.vertices[..3], &[0., 0., -20.]);
        assert_eq!(&mesh.vertices[mesh.vertices.len() - 3..], &[0., 0., 20.]);
        for v in mesh.vertices.as_chunks::<3>().0 {
            assert!((v.iter().map(|x| x * x).sum::<f64>().sqrt() - 20.).abs() < 1e-12);
        }
        // Every triangle's support plane bounds its radial chord error.
        for face in mesh.indices.as_chunks::<3>().0 {
            let vertex = |index: u32| -> [f64; 3] {
                let i = index as usize * 3;
                mesh.vertices[i..i + 3].try_into().unwrap()
            };
            let [a, b, c] = [vertex(face[0]), vertex(face[1]), vertex(face[2])];
            let [u, v]: [[f64; 3]; 2] = [b, c].map(|p| std::array::from_fn(|i| p[i] - a[i]));
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let support = a.into_iter().zip(n).map(|(a, n)| a * n).sum::<f64>()
                / n.into_iter().map(|x| x * x).sum::<f64>().sqrt();
            assert!(support >= 20. - options.deflection);
        }
        let proof = manufacturing::certify(&mesh).unwrap();
        let deficit = sphere.volume_mm3() - proof.signed_volume_mm3;
        assert!(deficit > 0. && deficit < sphere.surface_area_mm2() * options.deflection);
    }

    #[test]
    fn rejects_invalid_radius_options_and_resolution_budgets() {
        for radius in [0., -1., 1e-10, f64::NAN, f64::INFINITY, 1e8] {
            assert!(Sphere::new(radius).is_none());
        }
        let sphere = Sphere::new(1.).unwrap();
        let mut options = Tessellation {
            deflection: 0.,
            angle: 0.2,
            triangle_limit: 1_000,
        };
        assert_eq!(sphere.mesh(options), Err(MeshError::InvalidOptions));
        options.deflection = f64::MIN_POSITIVE;
        assert_eq!(sphere.mesh(options), Err(MeshError::BudgetExceeded));
        options.deflection = 0.1;
        assert_eq!(sphere.mesh(options), Err(MeshError::BudgetExceeded));
    }
}
