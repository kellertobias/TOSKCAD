//! Shared-index manufacturing tessellation of the exact double-rounded profile.
use crate::{
    display::{DisplayMesh, MeshError, Tessellation},
    rounded_thread::Thread,
};
use std::{collections::HashMap, f64::consts::TAU};
mod bore;
mod certificate;
mod sampling;
pub use bore::{bore_volume_error_bound, tessellate_bore, tessellate_bore_with_interrupt};
use sampling::{Receipt, phase_strips};
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Mesh(MeshError),
    Cancelled,
}
impl From<MeshError> for Error {
    fn from(error: MeshError) -> Self {
        Self::Mesh(error)
    }
}

pub fn tessellate(thread: &Thread, options: Tessellation) -> Result<DisplayMesh, Error> {
    tessellate_with_interrupt(thread, options, None)
}
pub fn tessellate_with_interrupt<'a>(
    thread: &'a Thread,
    options: Tessellation,
    interrupt: Option<&'a dyn Fn() -> bool>,
) -> Result<DisplayMesh, Error> {
    tessellate_shape(thread, None, options, interrupt)
}
fn tessellate_shape<'a>(
    thread: &'a Thread,
    bore: Option<&crate::rounded_thread::ThroughBore>,
    options: Tessellation,
    interrupt: Option<&'a dyn Fn() -> bool>,
) -> Result<DisplayMesh, Error> {
    options.check()?;
    let mut work = Work::new(interrupt);
    work.checkpoint()?;
    let receipt = Receipt::new(thread, options)?;
    let max_angle = (options.angle / 4.)
        .min((options.deflection / (2. * thread.spec().major_radius_mm)).sqrt());
    let max_angle = if let Some(bore) = bore {
        bore::max_angle(bore, options, max_angle)?
    } else {
        max_angle
    };
    let count = (TAU / max_angle).ceil();
    if !count.is_finite() || count * 2. > options.triangle_limit as f64 {
        return Err(MeshError::BudgetExceeded.into());
    }
    let mut steps = count as usize;
    loop {
        work.checkpoint()?;
        let strips = phase_strips(thread, TAU / steps as f64, options, receipt, &mut work)?;
        match Builder::new(thread, steps, strips, options, receipt, &mut work).build(bore) {
            Ok(mesh) => return Ok(mesh),
            Err(AttemptError::Failure(error)) => return Err(error),
            Err(AttemptError::Refine) => {
                steps = steps
                    .checked_mul(2)
                    .filter(|n| *n <= options.triangle_limit / 2)
                    .ok_or(MeshError::BudgetExceeded)?;
            }
        }
    }
}

/// Bound signed-volume error of a successfully emitted closed radial graph.
/// The source and each mesh point share the same axial level; angular drift
/// charges the full meridian slope as well as positional interpolation error.
pub fn volume_error_bound(thread: &Thread, options: Tessellation) -> Result<f64, Error> {
    options.check()?;
    let receipt = Receipt::new(thread, options)?;
    let minimum = (thread.root_radius_mm() - receipt.point_error) * (options.angle / 8.).cos();
    let mut slope = 0_f64;
    for (band, pair) in thread.profile_breaks().windows(2).enumerate() {
        for phase in pair {
            slope = slope.max(sampling::derivatives(thread, *phase, band).0.abs());
        }
    }
    radial_graph_volume_bound(
        thread.spec().major_radius_mm,
        thread.spec().length_mm,
        minimum,
        options.deflection,
        thread.helical_rate().abs() * slope,
    )
}
// Numeric envelope shared by the public source-bound calculation. A radial
// graph with no strictly positive inner radius has no such angular receipt.
fn radial_graph_volume_bound(
    radius: f64,
    length: f64,
    minimum: f64,
    deflection: f64,
    radial_speed: f64,
) -> Result<f64, Error> {
    if minimum <= 0. {
        return Err(MeshError::UnresolvedBoundary.into());
    }
    let angular = 2. * (deflection / (2. * minimum)).min(1.).asin();
    let error = deflection + radial_speed * angular;
    let bound = std::f64::consts::PI * length * error * (2. * radius + error);
    if !bound.is_finite() {
        return Err(MeshError::UnresolvedBoundary.into());
    }
    Ok((bound * (1. + 128. * f64::EPSILON)).next_up())
}

const MAX_WORK: usize = 24_000_000;
struct Work<'a> {
    used: usize,
    interrupt: Option<&'a dyn Fn() -> bool>,
}
impl<'a> Work<'a> {
    fn new(interrupt: Option<&'a dyn Fn() -> bool>) -> Self {
        Self { used: 0, interrupt }
    }
    fn checkpoint(&self) -> Result<(), Error> {
        if self.interrupt.is_some_and(|f| f()) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    fn charge(&mut self, units: usize) -> Result<(), Error> {
        let next = self
            .used
            .checked_add(units)
            .filter(|n| *n <= MAX_WORK)
            .ok_or(MeshError::BudgetExceeded)?;
        if next / 128 != self.used / 128 {
            self.checkpoint()?;
        }
        self.used = next;
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Strip {
    phase: f64,
    band: usize,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum VertexKey {
    Grid(usize, i64),
    CapColumn(usize, usize),
    CapRail(i64, usize),
}

#[derive(Clone, Copy)]
struct Corner {
    column: usize,
    rail: i64,
    z: f64,
}

#[derive(Debug, PartialEq, Eq)]
enum AttemptError {
    Failure(Error),
    Refine,
}
impl From<Error> for AttemptError {
    fn from(error: Error) -> Self {
        Self::Failure(error)
    }
}
impl From<MeshError> for AttemptError {
    fn from(error: MeshError) -> Self {
        Self::Failure(error.into())
    }
}
impl From<certificate::Refusal> for AttemptError {
    fn from(error: certificate::Refusal) -> Self {
        match error {
            certificate::Refusal::Precision => MeshError::UnresolvedBoundary.into(),
            certificate::Refusal::Angle => Self::Refine,
        }
    }
}

struct Builder<'a, 'b> {
    thread: &'a Thread,
    work: &'b mut Work<'a>,
    receipt: Receipt,
    options: Tessellation,
    parameters: Vec<[f64; 2]>,
    steps: usize,
    strips: Vec<Strip>,
    limit: usize,
    lead: f64,
    length: f64,
    pitch: f64,
    snap: f64,
    vertex_ids: HashMap<VertexKey, u32>,
    caps: [Vec<(f64, u32)>; 2],
    mesh: DisplayMesh,
}

impl<'a, 'b> Builder<'a, 'b> {
    fn new(
        thread: &'a Thread,
        steps: usize,
        strips: Vec<Strip>,
        options: Tessellation,
        receipt: Receipt,
        work: &'b mut Work<'a>,
    ) -> Self {
        let spec = thread.spec();
        let lead = if spec.left_hand { -1. } else { 1. } * thread.spec().pitch_mm;
        let smallest_strip = strips
            .iter()
            .map(|s| s.phase)
            .zip(
                strips
                    .iter()
                    .skip(1)
                    .map(|s| s.phase)
                    .chain([spec.pitch_mm]),
            )
            .map(|(lo, hi)| hi - lo)
            .fold(f64::INFINITY, f64::min);
        Self {
            thread,
            work,
            receipt,
            options,
            parameters: Vec::new(),
            steps,
            strips,
            limit: options.triangle_limit,
            lead,
            length: spec.length_mm,
            pitch: spec.pitch_mm,
            snap: (64. * f64::EPSILON * spec.length_mm.max(lead.abs()))
                .min(smallest_strip / 8.)
                .min(options.deflection * 1e-3),
            vertex_ids: HashMap::new(),
            caps: [Vec::new(), Vec::new()],
            mesh: DisplayMesh {
                vertices: Vec::new(),
                indices: Vec::new(),
                face_indices: Vec::new(),
            },
        }
    }

    fn build(
        mut self,
        bore: Option<&crate::rounded_thread::ThroughBore>,
    ) -> Result<DisplayMesh, AttemptError> {
        self.work.checkpoint()?;
        self.build_wall()?;
        if let Some(bore) = bore {
            self.add_bore(bore.radius_mm())?;
        } else {
            let caps = std::mem::take(&mut self.caps);
            for (cap, ring) in caps.into_iter().enumerate() {
                self.add_cap(cap, ring)?;
            }
        }
        self.work.checkpoint()?;
        Ok(self.mesh)
    }

    fn build_wall(&mut self) -> Result<(), AttemptError> {
        let per_period = self.strips.len() as i64;
        for column in 0..self.steps {
            self.work.checkpoint()?;
            let a = self.advance(column);
            let b = self.advance(column + 1);
            // Include the periodic cells straddling either end; the
            // actual axial planes clip them before adding any mesh data.
            let first = (-a.max(b) / self.pitch).floor() as i64;
            let last = ((self.length - a.min(b)) / self.pitch).floor() as i64;
            for period in first..=last {
                for strip in 0..self.strips.len() {
                    self.work.charge(1)?;
                    let rail = period * per_period + strip as i64;
                    let lo = self.rail_position(rail);
                    let hi = self.rail_position(rail + 1);
                    if hi + a.max(b) <= self.snap || lo + a.min(b) >= self.length - self.snap {
                        continue;
                    }
                    let corners = [
                        self.corner(column, rail, lo + a),
                        self.corner(column + 1, rail, lo + b),
                        self.corner(column + 1, rail + 1, hi + b),
                        self.corner(column, rail + 1, hi + a),
                    ];
                    let face = 2 + self.strips[strip].band as u32;
                    self.side(corners, face)?;
                }
            }
        }
        Ok(())
    }

    fn sort_ring(ring: &mut [(f64, u32)]) -> Result<(), AttemptError> {
        ring.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        if ring.len() < 3 || !ring.windows(2).all(|pair| pair[0].0 < pair[1].0) {
            return Err(MeshError::UnresolvedBoundary.into());
        }
        Ok(())
    }

    fn add_cap(&mut self, cap: usize, mut ring: Vec<(f64, u32)>) -> Result<(), AttemptError> {
        Self::sort_ring(&mut ring)?;
        let center = self.mesh.vertices.len() as u32 / 3;
        self.work.charge(1)?;
        self.mesh.vertices.extend([0., 0., self.cap_z(cap)]);
        self.parameters.push([0., self.cap_z(cap)]);
        for (&(_, a), &(_, b)) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let triangle = if cap == 0 {
                [center, b, a]
            } else {
                [center, a, b]
            };
            self.triangle(triangle, cap as u32)?;
        }
        Ok(())
    }

    fn advance(&self, column: usize) -> f64 {
        self.lead * self.fraction(column)
    }

    fn fraction(&self, column: usize) -> f64 {
        column as f64 / self.steps as f64
    }

    fn rail_phase(&self, rail: i64) -> f64 {
        let n = self.strips.len() as i64;
        self.strips[rail.rem_euclid(n) as usize].phase
    }

    fn rail_position(&self, rail: i64) -> f64 {
        let n = self.strips.len() as i64;
        (rail.div_euclid(n) as f64).mul_add(self.pitch, self.rail_phase(rail))
    }

    fn cap_z(&self, cap: usize) -> f64 {
        if cap == 0 { 0. } else { self.length }
    }

    fn corner(&self, column: usize, rail: i64, z: f64) -> Corner {
        let z = if z.abs() <= self.snap {
            0.
        } else if (z - self.length).abs() <= self.snap {
            self.length
        } else {
            z
        };
        Corner { column, rail, z }
    }

    fn corner_key(&self, corner: Corner) -> Option<VertexKey> {
        let column = corner.column % self.steps;
        if corner.z == 0. {
            Some(VertexKey::CapColumn(column, 0))
        } else if corner.z == self.length {
            Some(VertexKey::CapColumn(column, 1))
        } else if (0. ..self.length).contains(&corner.z) {
            // At one full revolution the phase changes by exactly one lead,
            // so use the corresponding zero-angle rail identity.
            let shift = if corner.column == self.steps {
                self.lead.signum() as i64
                    * i64::from(self.thread.spec().starts)
                    * self.strips.len() as i64
            } else {
                0
            };
            Some(VertexKey::Grid(column, corner.rail + shift))
        } else {
            None
        }
    }

    fn side(&mut self, corners: [Corner; 4], face: u32) -> Result<(), AttemptError> {
        let mut polygon = [0; 6];
        let mut count = 0;
        for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
            if let Some(key) = self.corner_key(a) {
                polygon[count] = self.vertex(key)?;
                count += 1;
            }
            let cap_order = if a.z <= b.z { [0, 1] } else { [1, 0] };
            for cap in cap_order {
                let z = self.cap_z(cap);
                if (a.z < z && z < b.z) || (b.z < z && z < a.z) {
                    let key = if a.column == b.column {
                        VertexKey::CapColumn(a.column % self.steps, cap)
                    } else {
                        debug_assert_eq!(a.rail, b.rail);
                        VertexKey::CapRail(a.rail, cap)
                    };
                    polygon[count] = self.vertex(key)?;
                    count += 1;
                }
            }
        }
        self.triangulate_polygon(&polygon[..count], face)
    }

    fn triangulate_polygon(&mut self, polygon: &[u32], face: u32) -> Result<(), AttemptError> {
        let count = polygon.len();
        if count > 6 {
            return Err(MeshError::BudgetExceeded.into());
        }
        if count < 3 {
            return Ok(());
        }
        if count - 2 > self.limit.saturating_sub(self.mesh.face_indices.len()) {
            return Err(MeshError::BudgetExceeded.into());
        }
        let mut angle_failure = false;
        for pivot in 0..count {
            let mut triangles = [[0; 3]; 4];
            for i in 1..count - 1 {
                triangles[i - 1] = [
                    polygon[pivot],
                    polygon[(pivot + i) % count],
                    polygon[(pivot + i + 1) % count],
                ];
            }
            let triangles = &triangles[..count - 2];
            let mut valid = true;
            for &triangle in triangles {
                self.work.charge(1)?;
                if let Err(error) = self.check_triangle(triangle, face) {
                    match error {
                        certificate::Refusal::Angle => angle_failure = true,
                        certificate::Refusal::Precision => {}
                    }
                    valid = false;
                    break;
                }
            }
            if valid {
                for &triangle in triangles {
                    self.emit_triangle(triangle, face)?;
                }
                return Ok(());
            }
        }
        if angle_failure {
            Err(AttemptError::Refine)
        } else {
            Err(MeshError::UnresolvedBoundary.into())
        }
    }

    fn vertex(&mut self, key: VertexKey) -> Result<u32, AttemptError> {
        if let Some(&id) = self.vertex_ids.get(&key) {
            return Ok(id);
        }
        self.work.charge(1)?;
        let (angle, z, cap) = match key {
            VertexKey::Grid(column, rail) => (
                TAU * self.fraction(column),
                self.rail_position(rail) + self.advance(column),
                None,
            ),
            VertexKey::CapColumn(column, cap) => {
                (TAU * self.fraction(column), self.cap_z(cap), Some(cap))
            }
            VertexKey::CapRail(rail, cap) => {
                let z = self.cap_z(cap);
                (
                    TAU * ((z - self.rail_position(rail)) / self.lead),
                    z,
                    Some(cap),
                )
            }
        };
        let radius = self.thread.radius(angle, z);
        let (sin, cos) = angle.sin_cos();
        let id = self.mesh.vertices.len() as u32 / 3;
        self.mesh.vertices.extend([radius * cos, radius * sin, z]);
        self.parameters.push([angle, z]);
        self.vertex_ids.insert(key, id);
        if let Some(cap) = cap {
            self.caps[cap].push((angle, id));
        }
        Ok(id)
    }

    fn triangle(&mut self, triangle: [u32; 3], face: u32) -> Result<(), AttemptError> {
        self.work.charge(1)?;
        self.check_triangle(triangle, face)?;
        self.emit_triangle(triangle, face)
    }
    fn emit_triangle(&mut self, triangle: [u32; 3], face: u32) -> Result<(), AttemptError> {
        self.work.charge(1)?;
        if self.mesh.face_indices.len() == self.limit {
            return Err(MeshError::BudgetExceeded.into());
        }
        self.mesh.indices.extend(triangle);
        self.mesh.face_indices.push(face);
        Ok(())
    }
    fn check_triangle(&self, triangle: [u32; 3], face: u32) -> Result<(), certificate::Refusal> {
        if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
            return Err(certificate::Refusal::Precision);
        }
        let points = triangle.map(|i| {
            let i = i as usize * 3;
            [
                self.mesh.vertices[i],
                self.mesh.vertices[i + 1],
                self.mesh.vertices[i + 2],
            ]
        });
        let [angle, z] = self.parameters[triangle[0] as usize];
        let normal = if face == 0 {
            [0., 0., -1.]
        } else if face == 1 {
            [0., 0., 1.]
        } else {
            self.thread.normal(angle, z)
        };
        certificate::facet(
            points,
            normal,
            self.options.angle * 0.5,
            self.receipt.normal_error,
            face >= 2,
        )?;
        Ok(())
    }
}
