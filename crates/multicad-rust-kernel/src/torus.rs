//! Exact regular ring torus and a periodic display tessellation.
use crate::display::{DisplayMesh, MeshError, Tessellation};
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug)]
pub struct Torus {
    major_mm: f64,
    minor_mm: f64,
}

impl Torus {
    pub fn new(major_mm: f64, minor_mm: f64) -> Option<Self> {
        ((1e-6..=1e7).contains(&minor_mm) && major_mm > minor_mm && major_mm + minor_mm <= 1e7)
            .then_some(Self { major_mm, minor_mm })
    }

    pub fn major_radius_mm(self) -> f64 {
        self.major_mm
    }

    pub fn minor_radius_mm(self) -> f64 {
        self.minor_mm
    }

    pub fn surface_area_mm2(self) -> f64 {
        TAU.powi(2) * self.major_mm * self.minor_mm
    }

    pub fn volume_mm3(self) -> f64 {
        self.surface_area_mm2() * self.minor_mm * 0.5
    }

    /// Signed distance to the periodic surface in local millimeters.
    pub fn signed_distance_mm(self, [x, y, z]: [f64; 3]) -> f64 {
        (x.hypot(y) - self.major_mm).hypot(z) - self.minor_mm
    }

    pub fn mesh(self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let outer = self.major_mm + self.minor_mm;
        // For a cell, the chart Hessian bounds the linear interpolation
        // remainder by outer*du^2 + minor*dv^2. The normal varies by at
        // most |du| + |dv|. Both bounds apply across the periodic seams.
        let count = |scale: f64| {
            let step = (options.deflection / (2. * scale))
                .sqrt()
                .min(options.angle / 2.);
            (TAU / step).ceil() as usize
        };
        let [u, v] = [count(outer), count(self.minor_mm)];
        if u > options.triangle_limit || v > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        let [u, v] = [u.next_multiple_of(4), v.next_multiple_of(4)];
        let triangles = 2_u64 * u as u64 * v as u64;
        if triangles > options.triangle_limit as u64 {
            return Err(MeshError::BudgetExceeded);
        }
        let triangles = triangles as usize;
        let mut mesh = DisplayMesh {
            vertices: Vec::with_capacity(triangles / 2 * 3),
            indices: Vec::with_capacity(triangles * 3),
            face_indices: Vec::with_capacity(triangles),
        };
        let meridian: Vec<_> = (0..v)
            .map(|i| {
                let (sin, cos) = (TAU * i as f64 / v as f64).sin_cos();
                (
                    self.minor_mm.mul_add(cos, self.major_mm),
                    self.minor_mm * sin,
                )
            })
            .collect();
        for i in 0..u {
            let (sin, cos) = (TAU * i as f64 / u as f64).sin_cos();
            for &(rho, z) in &meridian {
                mesh.vertices.extend([rho * cos, rho * sin, z]);
            }
            for j in 0..v {
                let a = i * v + j;
                let b = ((i + 1) % u) * v + j;
                let c = ((i + 1) % u) * v + (j + 1) % v;
                let d = i * v + (j + 1) % v;
                mesh.triangle([a, b, c], 0);
                mesh.triangle([a, c, d], 0);
            }
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manufacturing;
    use std::f64::consts::PI;

    #[test]
    fn exact_metrics_and_periodic_mesh_keep_the_bore_and_chord_bound() {
        let ring = Torus::new(12., 3.).unwrap();
        assert_eq!(ring.major_radius_mm(), 12.);
        assert_eq!(ring.minor_radius_mm(), 3.);
        assert!((ring.volume_mm3() - 216. * PI.powi(2)).abs() < 1e-12);
        assert!((ring.surface_area_mm2() - 144. * PI.powi(2)).abs() < 1e-12);
        assert_eq!(ring.signed_distance_mm([0.; 3]), 9.);
        assert_eq!(ring.signed_distance_mm([12., 0., 0.]), -3.);
        let options = Tessellation {
            deflection: 0.02,
            angle: 0.3,
            triangle_limit: 300_000,
        };
        let mesh = ring.mesh(options).unwrap();
        assert!(mesh.face_indices.iter().all(|&face| face == 0));
        let vertices = mesh.vertices.as_chunks::<3>().0;
        assert!(
            vertices
                .iter()
                .all(|&p| ring.signed_distance_mm(p).abs() < 1e-12)
        );
        for face in mesh.indices.as_chunks::<3>().0 {
            let points = face.map(|index| vertices[index as usize]);
            let center = std::array::from_fn(|i| points.iter().map(|p| p[i]).sum::<f64>() / 3.);
            assert!(ring.signed_distance_mm(center).abs() < options.deflection);
        }
        let volume = manufacturing::certify(&mesh).unwrap().signed_volume_mm3;
        assert!((volume - ring.volume_mm3()).abs() < ring.surface_area_mm2() * options.deflection);
    }

    #[test]
    fn refuses_pinched_radii_and_meshes_over_budget() {
        for (major, minor) in [(0., 1.), (2., 2.), (3., 0.), (f64::NAN, 1.), (1e7, 1.)] {
            assert!(Torus::new(major, minor).is_none());
        }
        let ring = Torus::new(2., 1.).unwrap();
        let mut options = Tessellation {
            deflection: 0.,
            angle: 0.3,
            triangle_limit: 100,
        };
        assert_eq!(ring.mesh(options), Err(MeshError::InvalidOptions));
        options.deflection = 0.1;
        assert_eq!(ring.mesh(options), Err(MeshError::BudgetExceeded));
        options.deflection = f64::MIN_POSITIVE;
        assert_eq!(ring.mesh(options), Err(MeshError::BudgetExceeded));
    }
}
