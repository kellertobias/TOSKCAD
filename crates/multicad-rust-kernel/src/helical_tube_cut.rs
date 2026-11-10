//! Coaxial cylinder minus a circular normal-profile helical tube.
//! Provides analytic material queries and trim extents for a bounded class.
use crate::prism::Location;
use std::f64::consts::{PI, TAU};

const MAX_LENGTH_MM: f64 = 1e7;
const MAX_TURNS: f64 = 256.;

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub stock_radius_mm: f64,
    pub stock_height_mm: f64,
    pub tube_radius_mm: f64,
    pub pitch_mm: f64,
    pub path_height_mm: f64,
    pub start_height_mm: f64,
    pub start_angle_rad: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInput,
    UnsupportedGeometry,
    Limit,
}

/// A lifted azimuth interval on the stock's outer cylindrical wall. Both
/// endpoints lie on the exact intersection of the stock and tube surfaces.
#[derive(Clone, Copy, Debug)]
pub struct CylinderTrim {
    pub azimuth_rad: [f64; 2],
    pub height_mm: [f64; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct Cut {
    spec: Spec,
    tolerance: f64,
    rise: f64,
    tangent: f64,
    vertical: f64,
    angle_limit: f64,
    rim: [f64; 2],
}

impl Cut {
    /// Prove that a right-handed tube centered on the stock radius starts
    /// between the stock caps, exits completely through the upper cap, and
    /// cannot intersect itself or touch another turn. The stock starts at Z=0.
    pub fn new(spec: Spec, tolerance_mm: f64) -> Result<Self, Error> {
        let Spec {
            stock_radius_mm: radius,
            stock_height_mm: height,
            tube_radius_mm: tube,
            pitch_mm: pitch,
            path_height_mm: travel,
            start_height_mm: start,
            start_angle_rad: phase,
        } = spec;
        let dimensions = [radius, height, tube, pitch, travel, start];
        if !(1e-10..=1e-3).contains(&tolerance_mm)
            || dimensions.iter().any(|&x| !x.is_finite() || x <= 0.)
            || !(-TAU..=TAU).contains(&phase)
        {
            return Err(Error::InvalidInput);
        }
        let scale = dimensions.into_iter().fold(0., f64::max);
        if scale > MAX_LENGTH_MM || travel / pitch > MAX_TURNS {
            return Err(Error::Limit);
        }
        let rise = pitch / TAU;
        let angle_limit = travel / rise;
        let roundoff = 64. * f64::EPSILON * (start + travel + scale + radius * angle_limit);
        if tolerance_mm < roundoff || tube <= 16. * tolerance_mm || tube > radius * 0.5 {
            return Err(Error::UnsupportedGeometry);
        }
        if rise > radius {
            return Err(Error::UnsupportedGeometry);
        }
        let length = radius.hypot(rise);
        let tangent = rise / length;
        let vertical = radius / length;
        let half_height = vertical * tube;
        let angle_shift = (tangent * tube / (radius - tube)).asin();
        // At fixed world radius and azimuth, the normal-disk parameter can
        // shift height by at most half_height + rise * angle_shift. A strict
        // gap excludes any second branch from the neighboring turn.
        if 2. * (half_height + rise * angle_shift) + 8. * tolerance_mm >= pitch
            || start - half_height <= tolerance_mm
            || start + half_height >= height - tolerance_mm
            || start + travel - half_height <= height + tolerance_mm
        {
            return Err(Error::UnsupportedGeometry);
        }
        // On the intersection x²+y²=tube² and (radius+x)²+(tangent*y)²=radius².
        // Rationalize the small root to avoid losing the radial displacement.
        let x = -(tangent * tube).powi(2)
            / (radius + (radius * radius - (tangent * vertical * tube).powi(2)).sqrt());
        let y = (tube * tube - x * x).sqrt();
        Ok(Self {
            spec,
            tolerance: tolerance_mm,
            rise,
            tangent,
            vertical,
            angle_limit,
            rim: [x, y],
        })
    }

    pub fn cylinder_trims(self) -> [CylinderTrim; 2] {
        let [x, y] = self.rim;
        [-y, y].map(|y| {
            let shift = (self.tangent * y).atan2(self.spec.stock_radius_mm + x);
            let start_z = self.spec.start_height_mm - self.vertical * y;
            let azimuth = self.spec.start_angle_rad + shift;
            CylinderTrim {
                azimuth_rad: [
                    azimuth,
                    azimuth + (self.spec.stock_height_mm - start_z) / self.rise,
                ],
                height_mm: [start_z, self.spec.stock_height_mm],
            }
        })
    }

    pub fn removed_volume_mm3(self) -> f64 {
        let radius = self.spec.stock_radius_mm;
        let tube = self.spec.tube_radius_mm;
        let y = self.rim[1];
        let sqrt_integral = |radius: f64, factor: f64| {
            let ratio = factor * y / radius;
            0.5 * radius * y * ((1. - ratio * ratio).sqrt() + ratio.asin() / ratio)
        };
        let disk = sqrt_integral(tube, 1.);
        let radial = sqrt_integral(radius, self.tangent) - radius * y + disk;
        let squared = self.vertical.powi(2) * y.powi(3) / 3. + 2. * radius * disk - tube * tube * y;
        // The Jacobian tangent*rise + vertical*(radius+x) is positive.
        // The cap-limited sweep angle is affine in y, so its odd part
        // integrates to zero over the symmetric disk segment inside stock.
        let center_sweep = (self.spec.stock_height_mm - self.spec.start_height_mm) / self.rise;
        2. * center_sweep * (self.tangent * self.rise * radial + 0.5 * self.vertical * squared)
    }

    pub fn volume_mm3(self) -> f64 {
        PI * self.spec.stock_radius_mm.powi(2) * self.spec.stock_height_mm
            - self.removed_volume_mm3()
    }

    /// Classify material in the stock frame. Locations within the numerical
    /// guard of a surviving support are returned as boundary.
    pub fn classify(self, point: [f64; 3]) -> Result<Location, Error> {
        let [x, y, z] = point;
        if point.iter().any(|&v| !v.is_finite()) {
            return Err(Error::InvalidInput);
        }
        let radial = x.hypot(y);
        let stock_gap = (radial - self.spec.stock_radius_mm)
            .max(z - self.spec.stock_height_mm)
            .max(-z);
        if stock_gap > self.tolerance {
            return Ok(Location::Exterior);
        }
        Ok(match self.tool_location(radial, y.atan2(x), z) {
            Location::Interior => Location::Exterior,
            Location::Boundary => Location::Boundary,
            Location::Exterior if stock_gap >= -self.tolerance => Location::Boundary,
            Location::Exterior => Location::Interior,
        })
    }

    fn tool_location(self, radius: f64, azimuth: f64, z: f64) -> Location {
        let tube = self.spec.tube_radius_mm;
        let guard = self.tolerance;
        if (radius - self.spec.stock_radius_mm).abs() > tube + guard {
            return Location::Exterior;
        }
        let relative = azimuth - self.spec.start_angle_rad;
        let turn =
            ((z - self.spec.start_height_mm - self.rise * relative) / self.spec.pitch_mm).round();
        let base_angle = relative + turn * TAU;
        let angle = |y: f64| base_angle - (self.tangent * y / radius).asin();
        let height = |y: f64| self.spec.start_height_mm + self.rise * angle(y) - self.vertical * y;
        let [mut lo, mut hi] = [-tube - guard, tube + guard];
        if z < height(hi) || z > height(lo) {
            return Location::Exterior;
        }
        // At fixed radius and lifted azimuth, height decreases strictly
        // with y. The pitch proof bounds the inverse to one turn.
        for _ in 0..52 {
            let mid = (lo + hi) * 0.5;
            if height(mid) > z {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let y = (lo + hi) * 0.5;
        let t = angle(y);
        let x = (radius * radius - (self.tangent * y).powi(2)).sqrt() - self.spec.stock_radius_mm;
        let disk_gap = x.hypot(y) - tube;
        // The axial chart boundaries are planes normal to the helix.
        // Convert the local angular margin to millimeters at the radius.
        let end_gap = (-t).max(t - self.angle_limit) * self.spec.stock_radius_mm;
        let gap = disk_gap.max(end_gap);
        if gap > guard {
            Location::Exterior
        } else if gap >= -guard {
            Location::Boundary
        } else {
            Location::Interior
        }
    }
}

#[cfg(test)]
mod tests;
