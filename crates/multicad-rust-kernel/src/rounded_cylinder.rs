//! Exact circular stock with one or two convex quarter-toroidal rim fillets.
//! The analytic patches are authoritative; meshes only approximate their boundary.
use crate::normal_sweep::Edge;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

mod mesh;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDimensions,
    Degenerate,
    Limit,
}

/// Stable semantic face roles. Use `face_index` for contiguous mesh labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Face {
    StartCap = 0,
    EndCap = 1,
    Wall = 2,
    StartFillet = 3,
    EndFillet = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    Plane {
        radius_mm: f64,
        z_mm: f64,
        normal_z: f64,
    },
    Cylinder {
        radius_mm: f64,
        span_mm: [f64; 2],
    },
    /// Full azimuth and the stated meridian interval. The major radius may
    /// be smaller than the minor radius: this outer quarter remains regular.
    Toroid {
        major_radius_mm: f64,
        minor_radius_mm: f64,
        center_z_mm: f64,
        meridian_rad: [f64; 2],
    },
}

/// A full circular trim shared by exactly the indicated two faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rim {
    /// Original sharp rims are 0/1. New start trims are 2/3; end trims 4/5.
    pub ordinal: u32,
    pub radius_mm: f64,
    pub z_mm: f64,
    pub faces: [Face; 2],
}

#[derive(Clone, Debug)]
pub struct RoundedCylinder {
    radius_mm: f64,
    height_mm: f64,
    fillets_mm: [Option<f64>; 2],
    volume_mm3: f64,
    centroid_z_mm: f64,
}

impl RoundedCylinder {
    /// Preserve both positive cap disks and a positive intervening wall.
    /// Axis contacts, disappearing faces and overlapping fillets are refused.
    pub fn new(
        radius_mm: f64,
        height_mm: f64,
        fillets_mm: [Option<f64>; 2],
        minimum_mm: f64,
        maximum_mm: f64,
    ) -> Result<Self, Error> {
        if ![radius_mm, height_mm, minimum_mm, maximum_mm]
            .into_iter()
            .all(f64::is_finite)
            || minimum_mm < 1e-6
            || maximum_mm < minimum_mm
            || radius_mm < minimum_mm
            || height_mm < minimum_mm
            || fillets_mm.iter().all(Option::is_none)
            || fillets_mm
                .iter()
                .flatten()
                .any(|f| !f.is_finite() || *f < minimum_mm)
        {
            return Err(Error::InvalidDimensions);
        }
        if maximum_mm > 1e7 || radius_mm > maximum_mm || height_mm > maximum_mm {
            return Err(Error::Limit);
        }
        let [lower, upper] = fillets_mm.map(|f| f.unwrap_or(0.));
        if fillets_mm
            .iter()
            .flatten()
            .any(|&f| radius_mm - f < minimum_mm)
            || height_mm - lower - upper < minimum_mm
        {
            return Err(Error::Degenerate);
        }
        let mut geometry = Self {
            radius_mm,
            height_mm,
            fillets_mm,
            volume_mm3: 0.,
            centroid_z_mm: 0.,
        };
        // For A=R-f and x=r-A, integrate the removed corner with
        // 0<=z<=f-sqrt(f²-x²), weighted by 2πr. Its first axial moment
        // uses the same radial integral weighted by z.
        let removed = |f: f64| {
            let a = radius_mm - f;
            let volume = TAU * (a * f * f * (1. - PI / 4.) + f.powi(3) / 6.);
            let axial_moment = PI * (a * f.powi(3) * (5. / 3. - FRAC_PI_2) + f.powi(4) / 12.);
            (volume, axial_moment)
        };
        let (lower_volume, lower_moment) = removed(lower);
        let (upper_volume, upper_moment) = removed(upper);
        let stock_volume = PI * radius_mm.powi(2) * height_mm;
        geometry.volume_mm3 = stock_volume - lower_volume - upper_volume;
        geometry.centroid_z_mm =
            (stock_volume * height_mm / 2. - lower_moment - height_mm * upper_volume
                + upper_moment)
                / geometry.volume_mm3;
        Ok(geometry)
    }

    pub fn radius_mm(&self) -> f64 {
        self.radius_mm
    }
    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }
    pub fn fillet_radii_mm(&self) -> [Option<f64>; 2] {
        self.fillets_mm
    }
    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }
    pub fn centroid_mm(&self) -> [f64; 3] {
        [0., 0., self.centroid_z_mm]
    }

    /// Counterclockwise [radius, axial] profile, including its axis segment.
    /// This interoperability view infers arc radii from floating endpoints.
    /// The declared toroidal patches and analytic metrics remain authoritative
    /// when large coordinates make a small endpoint difference less precise.
    pub fn meridian(&self) -> Vec<Edge> {
        let r = self.radius_mm;
        let h = self.height_mm;
        let [a, b] = self.fillets_mm.map(|f| f.unwrap_or(0.));
        let mut edges = vec![Edge::Line {
            a: [0., 0.],
            b: [r - a, 0.],
        }];
        if a > 0. {
            edges.push(Edge::Arc {
                center: [r - a, a],
                a: [r - a, 0.],
                b: [r, a],
                clockwise: false,
            });
        }
        edges.push(Edge::Line {
            a: [r, a],
            b: [r, h - b],
        });
        if b > 0. {
            edges.push(Edge::Arc {
                center: [r - b, h - b],
                a: [r, h - b],
                b: [r - b, h],
                clockwise: false,
            });
        }
        edges.extend([
            Edge::Line {
                a: [r - b, h],
                b: [0., h],
            },
            Edge::Line {
                a: [0., h],
                b: [0., 0.],
            },
        ]);
        edges
    }

    pub fn surface(&self, face: Face) -> Option<Surface> {
        let r = self.radius_mm;
        let h = self.height_mm;
        let [a, b] = self.fillets_mm.map(|f| f.unwrap_or(0.));
        Some(match face {
            Face::StartCap => Surface::Plane {
                radius_mm: r - a,
                z_mm: 0.,
                normal_z: -1.,
            },
            Face::EndCap => Surface::Plane {
                radius_mm: r - b,
                z_mm: h,
                normal_z: 1.,
            },
            Face::Wall => Surface::Cylinder {
                radius_mm: r,
                span_mm: [a, h - b],
            },
            Face::StartFillet => Surface::Toroid {
                major_radius_mm: r - self.fillets_mm[0]?,
                minor_radius_mm: a,
                center_z_mm: a,
                meridian_rad: [-FRAC_PI_2, 0.],
            },
            Face::EndFillet => Surface::Toroid {
                major_radius_mm: r - self.fillets_mm[1]?,
                minor_radius_mm: b,
                center_z_mm: h - b,
                meridian_rad: [0., FRAC_PI_2],
            },
        })
    }

    pub fn face_area_mm2(&self, face: Face) -> Option<f64> {
        Some(match self.surface(face)? {
            Surface::Plane { radius_mm, .. } => PI * radius_mm.powi(2),
            Surface::Cylinder { radius_mm, span_mm } => TAU * radius_mm * (span_mm[1] - span_mm[0]),
            Surface::Toroid {
                major_radius_mm: a,
                minor_radius_mm: f,
                ..
            } => TAU * f * (a * FRAC_PI_2 + f),
        })
    }

    pub fn surface_area_mm2(&self) -> f64 {
        [
            Face::StartCap,
            Face::EndCap,
            Face::Wall,
            Face::StartFillet,
            Face::EndFillet,
        ]
        .into_iter()
        .filter_map(|f| self.face_area_mm2(f))
        .sum()
    }

    /// Active semantic roles in the dense order used by manufacturing meshes.
    pub fn active_faces(&self) -> Vec<Face> {
        [
            Face::StartCap,
            Face::EndCap,
            Face::Wall,
            Face::StartFillet,
            Face::EndFillet,
        ]
        .into_iter()
        .filter(|&face| self.surface(face).is_some())
        .collect()
    }
    pub fn face_index(&self, role: Face) -> Option<u32> {
        self.active_faces()
            .iter()
            .position(|&face| face == role)
            .map(|i| i as u32)
    }
    pub fn face_role(&self, index: u32) -> Option<Face> {
        self.active_faces().get(index as usize).copied()
    }

    pub fn rims(&self) -> Vec<Rim> {
        let mut rims = Vec::new();
        for (i, fillet) in self.fillets_mm.into_iter().enumerate() {
            let cap = if i == 0 { Face::StartCap } else { Face::EndCap };
            let z = if i == 0 { 0. } else { self.height_mm };
            if let Some(f) = fillet {
                let patch = if i == 0 {
                    Face::StartFillet
                } else {
                    Face::EndFillet
                };
                rims.push(Rim {
                    ordinal: 2 + 2 * i as u32,
                    radius_mm: self.radius_mm - f,
                    z_mm: z,
                    faces: [cap, patch],
                });
                rims.push(Rim {
                    ordinal: 3 + 2 * i as u32,
                    radius_mm: self.radius_mm,
                    z_mm: if i == 0 { f } else { z - f },
                    faces: [Face::Wall, patch],
                });
            } else {
                rims.push(Rim {
                    ordinal: i as u32,
                    radius_mm: self.radius_mm,
                    z_mm: z,
                    faces: [cap, Face::Wall],
                });
            }
        }
        rims
    }

    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.into_iter().any(|x| !x.is_finite()) {
            return f64::NAN;
        }
        let magnitude = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if magnitude == 0. {
            return 0.;
        }
        let [x, y, z] = direction.map(|x| x / magnitude);
        let radial = x.hypot(y);
        let f = self.fillets_mm[usize::from(z >= 0.)].unwrap_or(0.);
        // Maximize the exact quarter-circle meridian in the radial/axial
        // direction. Radius inference from far endpoint subtraction is avoided.
        (radial * self.radius_mm + (z * self.height_mm).max(0.)
            - f * (radial + z.abs() - radial.hypot(z)))
            * magnitude
    }

    /// Closed occupied material, including each cap and toroidal boundary.
    pub fn contains(&self, point: [f64; 3], tolerance_mm: f64) -> bool {
        if !point.into_iter().all(f64::is_finite) || !tolerance_mm.is_finite() || tolerance_mm < 0.
        {
            return false;
        }
        let [x, y, z] = point;
        let rho = x.hypot(y);
        if !rho.is_finite()
            || z < -tolerance_mm
            || z > self.height_mm + tolerance_mm
            || rho > self.radius_mm + tolerance_mm
        {
            return false;
        }
        for (i, f) in self.fillets_mm.into_iter().enumerate() {
            if let Some(f) = f {
                let axial = if i == 0 { z } else { self.height_mm - z };
                let major = self.radius_mm - f;
                if axial < f && rho > major && (rho - major).hypot(axial - f) > f + tolerance_mm {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
