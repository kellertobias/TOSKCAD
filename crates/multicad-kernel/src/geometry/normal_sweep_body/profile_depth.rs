//! Full-column support from an unchanged exact prismatic source.
use super::*;

impl Solid {
    /// Depth behind a coincident cap of an unchanged normal extrusion.
    /// Callers must separately prove their entire window lies in that cap's
    /// finite profile. Side faces, shifted caps and partial cuts are unproved.
    pub fn rust_prismatic_cap_depth(
        &self,
        frame: PlaneFrame,
    ) -> Result<Option<f64>, GeometryError> {
        frame.checked(self.policy)?;
        let unchanged = match &self.body {
            SolidBody::Prism(_, _) | SolidBody::AxialCylinder(_, _, _) => true,
            SolidBody::PolygonPrism(body) => {
                body.pockets.is_empty() && body.bore_treatments.is_none()
            }
            SolidBody::NormalSweep(body, _) => body.openings.is_none(),
            _ => false,
        };
        if !unchanged {
            return Ok(None);
        }
        let normal = frame.normal();
        let squared = normal.iter().map(|v| v * v).sum::<f64>();
        let faces = self.faces()?;
        let mut stations = [0.; 2];
        for (index, face) in faces.iter().take(2).enumerate() {
            let Some((origin, cap_normal)) = face.plane else {
                return Ok(None);
            };
            if cross(normal, cap_normal) != [0.; 3] {
                return Ok(None);
            }
            stations[index] = origin
                .into_iter()
                .zip(frame.origin)
                .zip(normal)
                .map(|((p, o), n)| (p - o) * n)
                .sum::<f64>()
                / squared;
        }
        let top = stations[0].max(stations[1]);
        let bottom = stations[0].min(stations[1]);
        // Exact source coincidence only; no tolerance can hide an offset.
        Ok((top == 0. && bottom.is_finite() && bottom < 0.).then_some(-bottom))
    }
}
