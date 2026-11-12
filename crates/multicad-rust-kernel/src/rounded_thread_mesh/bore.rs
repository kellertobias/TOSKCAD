//! Annular end caps retain every qualified inner-wall station and use a common
//! outer polygon. Linear angular sweeping creates a checked oriented annulus.
use super::*;
use crate::rounded_thread::ThroughBore;

pub fn tessellate_bore(bore: &ThroughBore, options: Tessellation) -> Result<DisplayMesh, Error> {
    tessellate_bore_with_interrupt(bore, options, None)
}
pub fn tessellate_bore_with_interrupt<'a>(
    bore: &'a ThroughBore,
    options: Tessellation,
    interrupt: Option<&'a dyn Fn() -> bool>,
) -> Result<DisplayMesh, Error> {
    tessellate_shape(bore.thread(), Some(bore), options, interrupt)
}
pub fn bore_volume_error_bound(bore: &ThroughBore, options: Tessellation) -> Result<f64, Error> {
    let inner = volume_error_bound(bore.thread(), options)?;
    let outer = std::f64::consts::PI
        * bore.length_mm()
        * options.deflection
        * (2. * bore.radius_mm() + options.deflection);
    let result = ((inner + outer) * (1. + 128. * f64::EPSILON)).next_up();
    if !result.is_finite() {
        return Err(MeshError::UnresolvedBoundary.into());
    }
    Ok(result)
}
fn point_error(radius: f64, length: f64) -> f64 {
    let scale = ((radius + length).next_up() + 1.).next_up();
    (64. * f64::EPSILON * scale).next_up()
}
pub(super) fn max_angle(
    bore: &ThroughBore,
    options: Tessellation,
    current: f64,
) -> Result<f64, Error> {
    let error = point_error(bore.radius_mm(), bore.length_mm());
    let gap = (bore.radius_mm() - bore.thread().spec().major_radius_mm).next_down();
    if error > options.deflection / 16. || gap <= 8. * error {
        return Err(MeshError::UnresolvedBoundary.into());
    }
    Ok(current
        .min((options.deflection / (2. * bore.radius_mm())).sqrt())
        .min((gap / (2. * bore.radius_mm())).sqrt()))
}
// Separate numeric contracts also guard callers that supply their own sampling
// grids; successful angular-density selection does not replace these receipts.
fn prove_outer_polygon(radius: f64, length: f64, inner: f64, steps: usize) -> Result<(), Error> {
    if [radius, length, inner].into_iter().any(|x| !x.is_finite())
        || radius <= 0.
        || length <= 0.
        || inner < 0.
        || steps < 3
    {
        return Err(MeshError::UnresolvedBoundary.into());
    }
    let error = point_error(radius, length);
    let delta = (TAU / steps as f64 + 16. * f64::EPSILON * TAU).next_up();
    let square = (delta * delta).next_up();
    let lower = (radius * (1. - (square / 8.).next_up()).next_down()).next_down();
    if (lower - error).next_down() <= inner {
        return Err(MeshError::UnresolvedBoundary.into());
    }
    Ok(())
}
fn closed_sweep(actual: [usize; 2], expected: [usize; 2]) -> Result<(), Error> {
    if actual != expected {
        return Err(MeshError::UnresolvedBoundary.into());
    }
    Ok(())
}
impl Builder<'_, '_> {
    pub(super) fn add_bore(&mut self, radius: f64) -> Result<(), AttemptError> {
        let mut caps = std::mem::take(&mut self.caps);
        for cap in &mut caps {
            Self::sort_ring(cap)?;
        }
        let needed = 4 * self.steps + caps[0].len() + caps[1].len();
        if needed > self.limit.saturating_sub(self.mesh.face_indices.len()) {
            return Err(MeshError::BudgetExceeded.into());
        }
        self.work.charge(needed)?;
        let inner = (self.thread.spec().major_radius_mm + self.receipt.point_error).next_up();
        prove_outer_polygon(radius, self.length, inner, self.steps)?;
        let mut outer: [Vec<u32>; 2] = std::array::from_fn(|_| Vec::with_capacity(self.steps));
        for (cap, ring) in outer.iter_mut().enumerate() {
            for column in 0..self.steps {
                self.work.charge(1)?;
                let angle = TAU * self.fraction(column);
                let (s, c) = angle.sin_cos();
                let id = self.mesh.vertices.len() as u32 / 3;
                self.mesh
                    .vertices
                    .extend([radius * c, radius * s, self.cap_z(cap)]);
                self.parameters.push([angle, self.cap_z(cap)]);
                ring.push(id);
            }
        }
        // Reversal preserves every already checked finite thread triangle and
        // its positional/normal receipt; only its material-side orientation changes.
        for chunk in self.mesh.indices.as_chunks_mut::<3>().0.chunks_mut(128) {
            self.work.charge(chunk.len())?;
            for triangle in chunk {
                triangle.swap(1, 2);
            }
        }
        for column in 0..self.steps {
            self.work.checkpoint()?;
            let next = (column + 1) % self.steps;
            let angle = TAU * self.fraction(column);
            let (s, c) = angle.sin_cos();
            for triangle in [
                [outer[0][column], outer[0][next], outer[1][next]],
                [outer[0][column], outer[1][next], outer[1][column]],
            ] {
                self.bore_triangle(triangle, 7, [c, s, 0.], true)?;
            }
        }
        for (cap, ring) in caps.into_iter().enumerate() {
            self.bore_cap(cap, &ring, &outer[cap])?;
        }
        Ok(())
    }
    fn bore_triangle(
        &mut self,
        triangle: [u32; 3],
        face: u32,
        normal: [f64; 3],
        wall: bool,
    ) -> Result<(), AttemptError> {
        self.work.charge(1)?;
        let points = triangle.map(|id| {
            let i = 3 * id as usize;
            [
                self.mesh.vertices[i],
                self.mesh.vertices[i + 1],
                self.mesh.vertices[i + 2],
            ]
        });
        certificate::facet(
            points,
            normal,
            self.options.angle * 0.5,
            32. * f64::EPSILON,
            wall,
        )?;
        self.emit_triangle(triangle, face)
    }
    fn bore_cap(
        &mut self,
        cap: usize,
        inner: &[(f64, u32)],
        outer: &[u32],
    ) -> Result<(), AttemptError> {
        if inner[0].0 != 0. || inner.last().expect("checked ring").0 >= TAU {
            return Err(MeshError::UnresolvedBoundary.into());
        }
        let mut a = 0;
        let mut b = 0;
        let mut proof = CapProof::default();
        while a < inner.len() || b < outer.len() {
            self.work.checkpoint()?;
            let ia = inner[a % inner.len()].1;
            let ob = outer[b % outer.len()];
            let next_inner = if a + 1 < inner.len() {
                inner[a + 1].0
            } else {
                TAU
            };
            let next_outer = if b + 1 < outer.len() {
                TAU * self.fraction(b + 1)
            } else {
                TAU
            };
            let mut triangles = [[0; 3]; 2];
            let mut count = 0;
            if next_inner <= next_outer {
                triangles[count] = [ia, ob, inner[(a + 1) % inner.len()].1];
                count += 1;
                a += 1;
            }
            if next_outer <= next_inner {
                let ia = inner[a % inner.len()].1;
                triangles[count] = [ia, ob, outer[(b + 1) % outer.len()]];
                count += 1;
                b += 1;
            }
            for mut triangle in triangles[..count].iter().copied() {
                if cap == 0 {
                    triangle.swap(1, 2);
                }
                self.bore_triangle(
                    triangle,
                    cap as u32,
                    [0., 0., if cap == 0 { -1. } else { 1. }],
                    false,
                )?;
                proof.add(triangle, self.work)?;
            }
        }
        closed_sweep([a, b], [inner.len(), outer.len()])?;
        let outer_edges = outer
            .iter()
            .zip(outer.iter().cycle().skip(1))
            .map(|(&a, &b)| [a, b]);
        let inner_edges = inner
            .iter()
            .zip(inner.iter().cycle().skip(1))
            .map(|(&(_, a), &(_, b))| [b, a]);
        proof.finish(
            outer_edges.chain(inner_edges).map(|mut edge| {
                if cap == 0 {
                    edge.swap(0, 1);
                }
                edge
            }),
            self.work,
        )?;
        Ok(())
    }
}
#[derive(Default)]
struct CapProof {
    edges: HashMap<[u32; 2], (u8, i8)>,
}
fn edge_key([a, b]: [u32; 2]) -> ([u32; 2], i8) {
    if a < b { ([a, b], 1) } else { ([b, a], -1) }
}
impl CapProof {
    fn add(&mut self, triangle: [u32; 3], work: &mut Work<'_>) -> Result<(), Error> {
        work.charge(3)?;
        for edge in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            let (key, sign) = edge_key(edge);
            let entry = self.edges.entry(key).or_default();
            entry.0 += 1;
            entry.1 += sign;
            if entry.0 > 2 {
                return Err(MeshError::UnresolvedBoundary.into());
            }
        }
        Ok(())
    }
    fn finish(
        mut self,
        boundary: impl Iterator<Item = [u32; 2]>,
        work: &mut Work<'_>,
    ) -> Result<(), Error> {
        for edge in boundary {
            work.charge(1)?;
            let (key, sign) = edge_key(edge);
            if self.edges.remove(&key) != Some((1, sign)) {
                return Err(MeshError::UnresolvedBoundary.into());
            }
        }
        for &entry in self.edges.values() {
            work.charge(1)?;
            if entry != (2, 0) {
                return Err(MeshError::UnresolvedBoundary.into());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
