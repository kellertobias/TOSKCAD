//! Exact conical entry clipping of an unchanged straight thread tool.
//!
//! The retained cone connects the literal source-mask points `(R-c, 0)`
//! and `(R, c)`. Its material is intersected with the original helical law;
//! an optional terminal neck preserves the same full source and its phase.
use crate::{
    helical_thread::Thread,
    thread_end_relief::{Intent as TerminalIntent, Relief},
};

mod interval;
mod mesh;
pub use mesh::{tessellate, tessellate_with_interrupt};
mod phase;
mod profile;
mod properties;
mod support;
mod topology;
pub(crate) mod trim;
pub use topology::{
    BoundaryEdge, BoundaryFace, BoundaryTopology, EdgeUse, PhaseConstraint, PhaseEndpoint,
    PointSample, SourceCurve,
};
mod volume;
pub use properties::{FaceProperties, MassProperties, PropertiesOptions};

pub type Point = [f64; 3];
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enclosure {
    pub lower: f64,
    pub upper: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueryOptions {
    pub tolerance: f64,
    pub maximum_cells: usize,
}
impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            tolerance: 1e-7,
            maximum_cells: 1_000_000,
        }
    }
}
struct Work<'a> {
    remaining: usize,
    interrupt: Option<&'a dyn Fn() -> bool>,
}
impl<'a> Work<'a> {
    fn new(options: QueryOptions, interrupt: Option<&'a dyn Fn() -> bool>) -> Result<Self, Error> {
        if !options.tolerance.is_finite() || options.tolerance <= 0. {
            return Err(Error::Invalid);
        }
        if options.maximum_cells == 0 || options.maximum_cells > 1_000_000 {
            return Err(Error::Limit);
        }
        Ok(Self {
            remaining: options.maximum_cells,
            interrupt,
        })
    }
    fn step(&mut self) -> Result<(), Error> {
        if self.interrupt.is_some_and(|check| check()) {
            return Err(Error::Cancelled);
        }
        if self.remaining == 0 {
            return Err(Error::Limit);
        }
        self.remaining -= 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Limit,
    Unresolved,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intent {
    pub radial_depth_mm: f64,
    pub terminal: Option<TerminalIntent>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConeHalf {
    Rising,
    Falling,
}
/// Dense semantic roles shared by the retained supports, queries and mesh.
/// Source phase mappings are explicit; unrelated Thread/Shaft ordinals stay
/// unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Material at source station zero, after cone clipping.
    EntryCap,
    /// Prefix station F: a full cap without a neck, its annular shoulder
    /// when a terminal neck is retained.
    EndCap,
    /// Band 0 is the falling flank, 1 the root, 2 the rising flank and 3
    /// the crest. These are the retained STEP support roles, independent of
    /// handedness; source phase increases around each start's profile.
    ThreadBand {
        start: u32,
        band: u32,
    },
    ConeHalf {
        start: u32,
        half: ConeHalf,
    },
    NeckWall,
    /// The neck's actual far end at the original full source height H.
    FarCap,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chamfer {
    tool: Thread,
    terminal: Option<Relief>,
    entry_radius_mm: f64,
    entry_length_mm: f64,
    intent: Option<Intent>,
}
impl Chamfer {
    /// Preserve canonical end dimensions exactly, including their original
    /// validation order before any subtraction of physical stations.
    pub fn new(
        tool: Thread,
        radial_depth_mm: f64,
        terminal: Option<TerminalIntent>,
    ) -> Result<Self, Error> {
        let spec = tool.spec();
        let width = terminal.map_or(0., |intent| intent.width_mm);
        if !radial_depth_mm.is_finite()
            || radial_depth_mm < 0.001
            || spec.major_radius_mm - radial_depth_mm <= 0.001
            || radial_depth_mm + width + spec.pitch_mm > spec.length_mm
        {
            return Err(Error::Invalid);
        }
        let tail = terminal
            .map(|intent| Relief::new(tool, intent.width_mm, intent.radial_depth_mm))
            .transpose()
            .map_err(thread_error)?;
        // The original c+w+pitch check and the source turn bound prove
        // a positive remaining interval; Relief::new retains this same tool.
        // Do not reconstruct authored dimensions through from_material.
        let value = Self {
            tool,
            terminal: tail,
            entry_radius_mm: spec.major_radius_mm - radial_depth_mm,
            entry_length_mm: radial_depth_mm,
            intent: Some(Intent {
                radial_depth_mm,
                terminal,
            }),
        };
        value.certify_source_radius()?;
        Ok(value)
    }
    /// Reconstruct finite proved material without fabricating original intent.
    /// The cone's two radii and axial station remain independent stored data.
    pub fn from_material(
        tool: Thread,
        entry_radius_mm: f64,
        entry_length_mm: f64,
        terminal: Option<Relief>,
    ) -> Result<Self, Error> {
        let upper = terminal.map_or(tool.spec().length_mm, |tail| tail.material_length_mm());
        if !entry_radius_mm.is_finite()
            || entry_radius_mm <= 0.
            || entry_radius_mm >= tool.spec().major_radius_mm
            || !entry_length_mm.is_finite()
            || entry_length_mm <= 0.
            || entry_length_mm >= upper
            || terminal.is_some_and(|tail| *tail.source_thread() != tool)
        {
            return Err(Error::Invalid);
        }
        let value = Self {
            tool,
            terminal,
            entry_radius_mm,
            entry_length_mm,
            intent: None,
        };
        value.certify_source_radius()?;
        Ok(value)
    }
    pub fn source_thread(&self) -> &Thread {
        &self.tool
    }
    pub fn terminal_relief(&self) -> Option<&Relief> {
        self.terminal.as_ref()
    }
    pub fn intent(&self) -> Option<Intent> {
        self.intent
    }
    pub fn entry_radius_mm(&self) -> f64 {
        self.entry_radius_mm
    }
    pub fn entry_length_mm(&self) -> f64 {
        self.entry_length_mm
    }
    pub fn material_length_mm(&self) -> f64 {
        self.terminal
            .map_or(self.tool.spec().length_mm, |tail| tail.material_length_mm())
    }
    pub fn full_length_mm(&self) -> f64 {
        self.tool.spec().length_mm
    }
    /// Cone radius at the retained axial station, with authored endpoints.
    pub fn cone_radius(&self, z: f64) -> f64 {
        if z == 0. {
            self.entry_radius_mm
        } else if z == self.entry_length_mm {
            self.tool.spec().major_radius_mm
        } else {
            ((self.tool.spec().major_radius_mm - self.entry_radius_mm) / self.entry_length_mm)
                .mul_add(z, self.entry_radius_mm)
        }
    }
    /// Physical intersection seam in the unchanged source phase chart.
    pub fn seam_station(&self, phase: f64) -> f64 {
        let radius = self.tool.radius_at_phase(phase);
        if radius <= self.entry_radius_mm {
            0.
        } else if radius == self.tool.spec().major_radius_mm {
            self.entry_length_mm
        } else {
            self.entry_length_mm
                * ((radius - self.entry_radius_mm)
                    / (self.tool.spec().major_radius_mm - self.entry_radius_mm))
        }
    }
    /// Material radius; no rescaling of the helical phase or source length.
    pub fn radius(&self, theta: f64, z: f64) -> f64 {
        if !theta.is_finite() || !z.is_finite() || z < 0. || z > self.full_length_mm() {
            return f64::NAN;
        }
        if z > self.material_length_mm() {
            return self
                .terminal
                .expect("positive terminal interval")
                .neck_radius_mm();
        }
        let original = self.tool.radius(theta, z);
        if z < self.entry_length_mm {
            original.min(self.cone_radius(z))
        } else {
            original
        }
    }
    pub fn face_count(&self) -> u32 {
        2 + 6 * self.tool.spec().starts + if self.terminal.is_some() { 2 } else { 0 }
    }
    /// Dense retained wall role for one local source phase band (0..=4).
    /// The pre-crest band joins the preceding start's post-crest support.
    pub fn wall_face_index(&self, start: u32, phase_band: usize) -> Option<u32> {
        let count = self.tool.spec().starts;
        if start >= count || phase_band > 4 {
            return None;
        }
        Some(
            2 + match phase_band {
                0 => 4 * ((start + count - 1) % count) + 3,
                1..=3 => 4 * start + phase_band as u32 - 1,
                _ => 4 * start + 3,
            },
        )
    }
    pub fn cone_face_index(&self, start: u32, half: ConeHalf) -> Option<u32> {
        (start < self.tool.spec().starts).then(|| {
            2 + 4 * self.tool.spec().starts + 2 * start + u32::from(half == ConeHalf::Falling)
        })
    }
    pub fn face(&self, index: u32) -> Option<Face> {
        let n = self.tool.spec().starts;
        if index == 0 {
            Some(Face::EntryCap)
        } else if index == 1 {
            Some(Face::EndCap)
        } else if index < 2 + 4 * n {
            let ordinal = index - 2;
            Some(Face::ThreadBand {
                start: ordinal / 4,
                band: ordinal % 4,
            })
        } else if index < 2 + 6 * n {
            let ordinal = index - 2 - 4 * n;
            Some(Face::ConeHalf {
                start: ordinal / 2,
                half: if ordinal.is_multiple_of(2) {
                    ConeHalf::Rising
                } else {
                    ConeHalf::Falling
                },
            })
        } else if self.terminal.is_some() && index == 2 + 6 * n {
            Some(Face::NeckWall)
        } else if self.terminal.is_some() && index == 3 + 6 * n {
            Some(Face::FarCap)
        } else {
            None
        }
    }
}
fn thread_error(error: crate::helical_thread::ThreadError) -> Error {
    match error {
        crate::helical_thread::ThreadError::Invalid => Error::Invalid,
        crate::helical_thread::ThreadError::Limit => Error::Limit,
    }
}
#[cfg(test)]
mod tests;
