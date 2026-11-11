use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation};

impl RoundedCylinder {
    /// Shared angular rings join every analytic patch without duplicate seams.
    /// Each chart direction uses a quarter of the chord/normal budget; the
    /// remaining half covers finite-coordinate arithmetic.
    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let sampling = Tessellation {
            deflection: options.deflection / 4.,
            angle: options.angle / 4.,
            ..options
        };
        let u = sampling
            .arc_segments(self.radius_mm, TAU)?
            .next_multiple_of(4);
        let counts = self
            .fillets_mm
            .map(|f| f.map(|f| sampling.arc_segments(f, FRAC_PI_2)).transpose())
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let rings = 2 + counts.iter().flatten().sum::<usize>();
        let triangles = u
            .checked_mul(2)
            .and_then(|n| n.checked_mul(rings))
            .filter(|&n| n <= options.triangle_limit)
            .ok_or(MeshError::BudgetExceeded)?;
        // A meridian chord has length 2*f*sin(dv/2). Coordinate
        // subtraction must resolve its direction well inside the unused normal
        // budget. Otherwise a nominally fine grid could have inaccurate normals.
        let roundoff = 64. * f64::EPSILON * self.radius_mm.max(self.height_mm).max(1.);
        if roundoff > options.deflection / 4.
            || self.fillets_mm.into_iter().zip(&counts).any(|(f, count)| {
                f.zip(*count).is_some_and(|(f, n)| {
                    let chord = 2. * f * (FRAC_PI_2 / (2. * n as f64)).sin();
                    roundoff > chord * options.angle / 64.
                })
            })
        {
            return Err(MeshError::UnresolvedBoundary);
        }
        let mut meridian = Vec::with_capacity(rings);
        let mut faces = Vec::with_capacity(rings - 1);
        meridian.push([self.radius_mm - self.fillets_mm[0].unwrap_or(0.), 0.]);
        if let Some(count) = counts[0] {
            let f = self.fillets_mm[0].expect("selected lower fillet");
            let face = self
                .face_index(Face::StartFillet)
                .expect("selected start face");
            for i in 1..=count {
                let phase = -FRAC_PI_2 + FRAC_PI_2 * i as f64 / count as f64;
                meridian.push(if i == count {
                    [self.radius_mm, f]
                } else {
                    [self.radius_mm - f + f * phase.cos(), f + f * phase.sin()]
                });
                faces.push(face);
            }
        }
        meridian.push([
            self.radius_mm,
            self.height_mm - self.fillets_mm[1].unwrap_or(0.),
        ]);
        faces.push(Face::Wall as u32);
        if let Some(count) = counts[1] {
            let f = self.fillets_mm[1].expect("selected upper fillet");
            let face = self.face_index(Face::EndFillet).expect("selected end face");
            for i in 1..=count {
                let phase = FRAC_PI_2 * i as f64 / count as f64;
                meridian.push(if i == count {
                    [self.radius_mm - f, self.height_mm]
                } else {
                    [
                        self.radius_mm - f + f * phase.cos(),
                        self.height_mm - f + f * phase.sin(),
                    ]
                });
                faces.push(face);
            }
        }
        let mut mesh = DisplayMesh {
            vertices: Vec::with_capacity((u * meridian.len() + 2) * 3),
            indices: Vec::with_capacity(triangles * 3),
            face_indices: Vec::with_capacity(triangles),
        };
        mesh.vertices.extend([0., 0., 0., 0., 0., self.height_mm]);
        for &[r, z] in &meridian {
            for station in 0..u {
                let (sin, cos) = (TAU * station as f64 / u as f64).sin_cos();
                mesh.vertices.extend([r * cos, r * sin, z]);
            }
        }
        let index = |ring: usize, station: usize| 2 + ring * u + station % u;
        for station in 0..u {
            mesh.triangle(
                [0, index(0, station + 1), index(0, station)],
                Face::StartCap as u32,
            );
            let last = meridian.len() - 1;
            mesh.triangle(
                [1, index(last, station), index(last, station + 1)],
                Face::EndCap as u32,
            );
            for (ring, &face) in faces.iter().enumerate() {
                let [a, b, c, d] = [
                    index(ring, station),
                    index(ring, station + 1),
                    index(ring + 1, station + 1),
                    index(ring + 1, station),
                ];
                mesh.triangle([a, b, c], face);
                mesh.triangle([a, c, d], face);
            }
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests;
