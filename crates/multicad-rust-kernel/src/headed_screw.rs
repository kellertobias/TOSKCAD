//! Single connected solids for flat socket-cap and external-hex machine screws.
//! The thread starts at z=0. The shank and head extend along negative Z.

use crate::helical_thread::{Thread, ThreadError};
use std::f64::consts::{FRAC_PI_3, FRAC_PI_6, PI};

const MAX_DIMENSION_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrewHead {
    SocketCap {
        radius_mm: f64,
        height_mm: f64,
        socket_across_flats_mm: f64,
        socket_depth_mm: f64,
    },
    ExternalHex {
        across_flats_mm: f64,
        height_mm: f64,
    },
}

impl ScrewHead {
    pub fn height_mm(self) -> f64 {
        match self {
            Self::SocketCap { height_mm, .. } | Self::ExternalHex { height_mm, .. } => height_mm,
        }
    }

    pub fn cross_section_area_mm2(self) -> f64 {
        match self {
            Self::SocketCap { radius_mm, .. } => PI * radius_mm.powi(2),
            Self::ExternalHex {
                across_flats_mm, ..
            } => hex_area(across_flats_mm),
        }
    }

    /// Across-flats width and depth of the flat-bottom socket, if present.
    pub fn socket(self) -> Option<(f64, f64)> {
        match self {
            Self::SocketCap {
                socket_across_flats_mm,
                socket_depth_mm,
                ..
            } => Some((socket_across_flats_mm, socket_depth_mm)),
            Self::ExternalHex { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrewSpec {
    pub head: ScrewHead,
    pub shank_length_mm: f64,
    pub shank_radius_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrewFace {
    ThreadShoulder,
    ThreadTip,
    /// Zero-based wall number after the two thread end faces.
    ThreadWall(u32),
    ShankWall,
    HeadSeat,
    HeadTop,
    HeadWall(u32),
    SocketFloor,
    SocketWall(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screw {
    thread: Thread,
    spec: ScrewSpec,
}

/// Counterclockwise hexagon, with its first corner at 30 degrees from +X.
pub fn hex_vertices(across_flats_mm: f64) -> [[f64; 2]; 6] {
    let x = across_flats_mm * 0.5;
    let y = x / 3_f64.sqrt();
    [
        [x, y],
        [0., 2. * y],
        [-x, y],
        [-x, -y],
        [0., -2. * y],
        [x, -y],
    ]
}

fn hex_area(across_flats_mm: f64) -> f64 {
    3_f64.sqrt() * across_flats_mm.powi(2) / 2.
}

fn hex_radius(across_flats_mm: f64, theta: f64) -> f64 {
    let offset = (theta + FRAC_PI_6).rem_euclid(FRAC_PI_3) - FRAC_PI_6;
    across_flats_mm * 0.5 / offset.cos()
}

impl Screw {
    pub fn new(thread: Thread, spec: ScrewSpec) -> Result<Self, ThreadError> {
        let ScrewSpec {
            head,
            shank_length_mm,
            shank_radius_mm,
        } = spec;
        let head_height = head.height_mm();
        let major = thread.spec().major_radius_mm;
        if !shank_length_mm.is_finite()
            || shank_length_mm < 0.
            || !shank_radius_mm.is_finite()
            || shank_radius_mm <= 0.
            || (shank_length_mm > 0. && shank_radius_mm < major)
            || !head_height.is_finite()
            || head_height <= 0.
        {
            return Err(ThreadError::Invalid);
        }
        let required = shank_radius_mm.max(major);
        let outside_radius = match head {
            ScrewHead::SocketCap {
                radius_mm,
                socket_across_flats_mm,
                socket_depth_mm,
                ..
            } => {
                if !radius_mm.is_finite()
                    || radius_mm <= required
                    || !socket_across_flats_mm.is_finite()
                    || socket_across_flats_mm <= 0.
                    || socket_across_flats_mm / 3_f64.sqrt() >= radius_mm
                    || !socket_depth_mm.is_finite()
                    || socket_depth_mm <= 0.
                    || socket_depth_mm >= head_height
                {
                    return Err(ThreadError::Invalid);
                }
                radius_mm
            }
            ScrewHead::ExternalHex {
                across_flats_mm, ..
            } => {
                if !across_flats_mm.is_finite() || across_flats_mm / 2. <= required {
                    return Err(ThreadError::Invalid);
                }
                across_flats_mm / 3_f64.sqrt()
            }
        };
        if outside_radius > MAX_DIMENSION_MM || shank_length_mm + head_height > MAX_DIMENSION_MM {
            return Err(ThreadError::Limit);
        }
        Ok(Self { thread, spec })
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    pub fn spec(&self) -> ScrewSpec {
        self.spec
    }

    pub fn head_top_z(&self) -> f64 {
        self.head_seat_z() - self.spec.head.height_mm()
    }

    pub fn head_seat_z(&self) -> f64 {
        -self.spec.shank_length_mm
    }

    pub fn head_radius(&self, theta: f64) -> f64 {
        match self.spec.head {
            ScrewHead::SocketCap { radius_mm, .. } => radius_mm,
            ScrewHead::ExternalHex {
                across_flats_mm, ..
            } => hex_radius(across_flats_mm, theta),
        }
    }

    pub fn socket_radius(&self, theta: f64) -> Option<f64> {
        self.spec
            .head
            .socket()
            .map(|(width, _)| hex_radius(width, theta))
    }

    pub fn faces(&self) -> Vec<ScrewFace> {
        let mut faces = vec![ScrewFace::ThreadShoulder, ScrewFace::ThreadTip];
        faces.extend((0..4 * self.thread.spec().starts).map(ScrewFace::ThreadWall));
        if self.spec.shank_length_mm > 0. {
            faces.extend([ScrewFace::ShankWall, ScrewFace::HeadSeat]);
        }
        faces.push(ScrewFace::HeadTop);
        if self.spec.head.socket().is_some() {
            faces.extend([ScrewFace::HeadWall(0), ScrewFace::SocketFloor]);
            faces.extend((0..6).map(ScrewFace::SocketWall));
        } else {
            faces.extend((0..6).map(ScrewFace::HeadWall));
        }
        faces
    }

    pub fn volume_mm3(&self) -> f64 {
        let head = self.spec.head;
        let socket = head
            .socket()
            .map_or(0., |(width, depth)| hex_area(width) * depth);
        let shank = PI * self.spec.shank_radius_mm.powi(2) * self.spec.shank_length_mm;
        self.thread.volume_mm3() + shank + head.cross_section_area_mm2() * head.height_mm() - socket
    }

    /// Conservative support of the occupied material in a local direction.
    /// The socket leaves the outside silhouette of the head unchanged.
    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.iter().any(|v| !v.is_finite()) {
            return f64::NAN;
        }
        let scale = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if scale == 0. {
            return 0.;
        }
        let unit = direction.map(|v| v / scale);
        let [x, y, z] = unit;
        let radial = x.hypot(y);
        let head_xy = match self.spec.head {
            ScrewHead::SocketCap { radius_mm, .. } => radial * radius_mm,
            ScrewHead::ExternalHex {
                across_flats_mm, ..
            } => hex_vertices(across_flats_mm)
                .into_iter()
                .map(|[u, v]| x.mul_add(u, y * v))
                .fold(f64::NEG_INFINITY, f64::max),
        };
        let head = head_xy + (z * self.head_top_z()).max(z * self.head_seat_z());
        let mut support = self.thread.support(unit).max(head);
        if self.spec.shank_length_mm > 0. {
            let shank = radial * self.spec.shank_radius_mm + (z * self.head_seat_z()).max(0.);
            support = support.max(shank);
        }
        scale * support
    }

    /// Test material membership with the same radial and axial tolerance as
    /// the shaft. Boundaries, including the blind socket floor, are material.
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        if !tolerance.is_finite() || tolerance < 0. || ![x, y, z].into_iter().all(f64::is_finite) {
            return false;
        }
        let radius = x.hypot(y);
        if self.thread.contains([x, y, z], tolerance)
            || (self.spec.shank_length_mm > 0.
                && z >= self.head_seat_z() - tolerance
                && z <= tolerance
                && radius <= self.spec.shank_radius_mm + tolerance)
        {
            return true;
        }
        let theta = y.atan2(x);
        let top = self.head_top_z();
        z >= top - tolerance
            && z <= self.head_seat_z() + tolerance
            && radius <= self.head_radius(theta) + tolerance
            && self.spec.head.socket().is_none_or(|(width, depth)| {
                z >= top + depth - tolerance || radius >= hex_radius(width, theta) - tolerance
            })
    }
}

#[cfg(test)]
mod tests;
