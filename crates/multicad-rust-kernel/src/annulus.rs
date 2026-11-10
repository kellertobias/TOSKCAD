//! Finite coaxial circular rings with an analytic through bore.

use crate::{
    Error,
    display::{DisplayMesh, MeshError, Tessellation},
};
use std::f64::consts::{PI, TAU};

const MAX_DIMENSION_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Annulus {
    inner_radius_mm: f64,
    outer_radius_mm: f64,
    height_mm: f64,
}

impl Annulus {
    pub fn new(inner_radius_mm: f64, outer_radius_mm: f64, height_mm: f64) -> Result<Self, Error> {
        if !inner_radius_mm.is_finite()
            || !outer_radius_mm.is_finite()
            || !height_mm.is_finite()
            || inner_radius_mm <= 0.
            || outer_radius_mm <= inner_radius_mm
            || height_mm <= 0.
        {
            return Err(Error::InvalidLength);
        }
        let annulus = Self {
            inner_radius_mm,
            outer_radius_mm,
            height_mm,
        };
        if outer_radius_mm > MAX_DIMENSION_MM
            || height_mm > MAX_DIMENSION_MM
            // Both sampled rims must remain distinct at every angular station.
            || outer_radius_mm - inner_radius_mm <= 4. * f64::EPSILON * outer_radius_mm
            || annulus.volume_mm3() == 0.
        {
            return Err(Error::OutOfRange);
        }
        Ok(annulus)
    }

    pub fn inner_radius_mm(&self) -> f64 {
        self.inner_radius_mm
    }

    pub fn outer_radius_mm(&self) -> f64 {
        self.outer_radius_mm
    }

    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }

    pub fn cross_section_area_mm2(&self) -> f64 {
        PI * (self.outer_radius_mm - self.inner_radius_mm)
            * (self.outer_radius_mm + self.inner_radius_mm)
    }

    pub fn volume_mm3(&self) -> f64 {
        self.cross_section_area_mm2() * self.height_mm
    }

    /// The support point lies on the exterior cylinder or one of its rims.
    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.iter().any(|component| !component.is_finite()) {
            return f64::NAN;
        }
        let scale = direction.into_iter().map(f64::abs).fold(1., f64::max);
        let [x, y, z] = direction.map(|component| component / scale);
        (x.hypot(y) * self.outer_radius_mm + z.max(0.) * self.height_mm) * scale
    }

    /// Test the closed annular material with an optional radial and axial tolerance.
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance_mm: f64) -> bool {
        if !tolerance_mm.is_finite()
            || tolerance_mm < 0.
            || ![x, y, z].into_iter().all(f64::is_finite)
        {
            return false;
        }
        let radius = x.hypot(y);
        z >= -tolerance_mm
            && z <= self.height_mm + tolerance_mm
            && radius >= self.inner_radius_mm - tolerance_mm
            && radius <= self.outer_radius_mm + tolerance_mm
    }
}

/// Sample both circles on identical rays. The caps and walls share the rim
/// vertices and retain their analytic face ordinals.
pub fn tessellate(annulus: &Annulus, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    if !options.deflection.is_finite()
        || options.deflection <= 0.
        || !(0.01..=1.).contains(&options.angle)
        || !(1..=5_000_000).contains(&options.triangle_limit)
    {
        return Err(MeshError::InvalidOptions);
    }
    let outer = annulus.outer_radius_mm;
    let inner = annulus.inner_radius_mm;
    let height = annulus.height_mm;
    let sagitta_angle = 4. * (options.deflection / (2. * outer)).sqrt().min(1.).asin();
    let sectors = (TAU / options.angle.min(sagitta_angle)).ceil();
    if sectors > (options.triangle_limit / 8) as f64 {
        return Err(MeshError::BudgetExceeded);
    }
    let sectors = sectors as usize;
    let mut mesh = DisplayMesh {
        vertices: Vec::with_capacity(sectors * 12),
        indices: Vec::with_capacity(sectors * 24),
        face_indices: Vec::with_capacity(sectors * 8),
    };
    for sector in 0..sectors {
        let (sin, cos) = (TAU * sector as f64 / sectors as f64).sin_cos();
        let [ox, oy] = [outer * cos, outer * sin];
        let [ix, iy] = [inner * cos, inner * sin];
        mesh.vertices
            .extend([ox, oy, 0., ox, oy, height, ix, iy, 0., ix, iy, height]);
    }
    let ring_indices = |sector: usize| std::array::from_fn(|i| (sector * 4 + i) as u32);
    for sector in 0..sectors {
        let [outer_bottom, outer_top, inner_bottom, inner_top] = ring_indices(sector);
        let [
            next_outer_bottom,
            next_outer_top,
            next_inner_bottom,
            next_inner_top,
        ] = ring_indices((sector + 1) % sectors);
        let face_triangles = [
            [
                [inner_bottom, next_outer_bottom, outer_bottom],
                [inner_bottom, next_inner_bottom, next_outer_bottom],
            ],
            [
                [inner_top, outer_top, next_outer_top],
                [inner_top, next_outer_top, next_inner_top],
            ],
            [
                [outer_bottom, next_outer_bottom, next_outer_top],
                [outer_bottom, next_outer_top, outer_top],
            ],
            [
                [inner_bottom, next_inner_top, next_inner_bottom],
                [inner_bottom, inner_top, next_inner_top],
            ],
        ];
        for (face, triangles) in face_triangles.into_iter().enumerate() {
            mesh.indices.extend(triangles.into_iter().flatten());
            mesh.face_indices.extend([face as u32; 2]);
        }
    }
    Ok(mesh)
}

/// Bound geometric volume loss for a successful tessellation. Both inscribed
/// rings have the same area scale. The unit polygon contains the disk of radius
/// cos(π/n), so the relative loss is at most 1 - cos²(π/n), bounded by twice
/// the permitted outer sagitta divided by the outer radius.
pub fn volume_error_bound(annulus: &Annulus, options: Tessellation) -> f64 {
    annulus.volume_mm3() * (2. * (options.deflection / annulus.outer_radius_mm)).min(1.)
}

#[cfg(test)]
mod tests;
