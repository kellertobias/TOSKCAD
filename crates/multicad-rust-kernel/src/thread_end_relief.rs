//! Exact terminal cylindrical relief inside an unchanged straight thread tool.
//!
//! The helical law is retained on the literal prefix `0 <= z <= L-width`;
//! the terminal material is a plain neck of radius `root-radial_depth`.
use crate::helical_thread::{AxialPrefix, Thread, ThreadError};

pub(crate) fn minimum_radius(tool: Thread) -> f64 {
    // Rounded-root parameters, rather than the separately tolerance-checked
    // depth metadata, determine the actual circle's inner support.
    tool.root_circle().map_or(tool.root_radius_mm(), |root| {
        (root.center_radius - root.radius).next_down()
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intent {
    pub width_mm: f64,
    pub radial_depth_mm: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relief {
    prefix: AxialPrefix,
    neck_radius_mm: f64,
    intent: Option<Intent>,
}
impl Relief {
    /// Match the canonical custom square relief contract, preserving the
    /// authored dimensions even when subtraction rounds its material station.
    pub fn new(tool: Thread, width_mm: f64, radial_depth_mm: f64) -> Result<Self, ThreadError> {
        let spec = tool.spec();
        let neck = tool.root_radius_mm() - radial_depth_mm;
        if !width_mm.is_finite()
            || !radial_depth_mm.is_finite()
            || width_mm < 0.001
            || radial_depth_mm < 0.001
            || width_mm + spec.pitch_mm > spec.length_mm
            || neck <= 0.001
            || neck >= minimum_radius(tool)
        {
            return Err(ThreadError::Invalid);
        }
        Ok(Self {
            prefix: tool.positive_axial_prefix(spec.length_mm - width_mm)?,
            neck_radius_mm: neck,
            intent: Some(Intent {
                width_mm,
                radial_depth_mm,
            }),
        })
    }
    /// Reconstruct the proved finite material from exchange. This preserves
    /// the actual shoulder station and neck radius; original width/depth
    /// intent cannot be recovered from a B-rep and is explicitly absent.
    pub fn from_material(
        tool: Thread,
        material_length_mm: f64,
        neck_radius_mm: f64,
    ) -> Result<Self, ThreadError> {
        let spec = tool.spec();
        let width = spec.length_mm - material_length_mm;
        if !neck_radius_mm.is_finite()
            || neck_radius_mm <= 0.
            || neck_radius_mm >= minimum_radius(tool)
            || width <= 0.
        {
            return Err(ThreadError::Invalid);
        }
        Ok(Self {
            prefix: tool.positive_axial_prefix(material_length_mm)?,
            neck_radius_mm,
            intent: None,
        })
    }
    pub fn source_thread(&self) -> &Thread {
        self.prefix.tool()
    }
    pub fn material_thread(&self) -> &Thread {
        self.prefix.material()
    }
    pub fn intent(&self) -> Option<Intent> {
        self.intent
    }
    pub fn neck_radius_mm(&self) -> f64 {
        self.neck_radius_mm
    }
    pub fn material_length_mm(&self) -> f64 {
        self.material_thread().spec().length_mm
    }
    pub fn full_length_mm(&self) -> f64 {
        self.source_thread().spec().length_mm
    }
    pub fn wall_face(&self) -> u32 {
        2 + 4 * self.source_thread().spec().starts
    }
    pub fn far_cap_face(&self) -> u32 {
        self.wall_face() + 1
    }
    pub fn face_count(&self) -> u32 {
        self.far_cap_face() + 1
    }
    pub fn volume_mm3(&self) -> f64 {
        self.material_thread().volume_mm3()
            + std::f64::consts::PI
                * self.neck_radius_mm.powi(2)
                * (self.full_length_mm() - self.material_length_mm())
    }
    pub fn support(&self, d: [f64; 3]) -> f64 {
        let [x, y, z] = d;
        self.material_thread().support(d).max(
            self.neck_radius_mm * x.hypot(y)
                + (z * self.material_length_mm()).max(z * self.full_length_mm()),
        )
    }
    pub fn contains(&self, p: [f64; 3], tolerance: f64) -> bool {
        let [x, y, z] = p;
        [x, y, z, tolerance].into_iter().all(f64::is_finite)
            && tolerance >= 0.
            && (self.material_thread().contains(p, tolerance)
                || (z >= self.material_length_mm() - tolerance
                    && z <= self.full_length_mm() + tolerance
                    && x.hypot(y) <= self.neck_radius_mm + tolerance))
    }
}
#[cfg(test)]
mod tests;
