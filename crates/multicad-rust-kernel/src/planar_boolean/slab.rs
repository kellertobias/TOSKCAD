use super::*;

#[derive(Clone, Debug)]
pub struct Slab {
    pub piece: ConvexPiece,
    /// Input face index for a surviving plane, or no index for a new cap.
    pub source_faces: Vec<Option<usize>>,
}

impl ConvexPiece {
    /// Restrict material to a closed horizontal slab. A contact with no
    /// positive volume is empty. Generated cap vertices use the requested Z.
    pub fn clip_z_band(
        &self,
        [low, high]: [f64; 2],
        tolerance_mm: f64,
    ) -> Result<Option<Slab>, Error> {
        validate_tolerance(tolerance_mm)?;
        if !valid_height(low) || !valid_height(high) || high - low <= tolerance_mm {
            return Err(Error::Invalid);
        }
        let faces = self
            .faces
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, face)| (face, Some(i)))
            .collect();
        let Some(faces) = clip(faces, low, true, tolerance_mm)? else {
            return Ok(None);
        };
        let Some(faces) = clip(faces, high, false, tolerance_mm)? else {
            return Ok(None);
        };
        let (faces, source_faces) = faces.into_iter().unzip();
        let piece = Self::new(faces, tolerance_mm)?;
        Ok(Some(Slab {
            piece,
            source_faces,
        }))
    }
}

impl Slab {
    /// Translate all planes. Subtract the source anchor before adding the
    /// destination anchor so an authored seam lands on the exact destination.
    pub fn translate_z(self, from_mm: f64, to_mm: f64, tolerance_mm: f64) -> Result<Self, Error> {
        if !valid_height(from_mm) || !valid_height(to_mm) {
            return Err(Error::Invalid);
        }
        self.map_z(|z| (z - from_mm) + to_mm, tolerance_mm)
    }

    /// Map the rising quarter circle's vertical interval to its horizontal
    /// radius coordinate. Only horizontal caps and vertical side planes keep
    /// their planar supports under this nonlinear coordinate change.
    pub fn remap_quarter_circle(
        self,
        floor_mm: f64,
        radius_mm: f64,
        tolerance_mm: f64,
    ) -> Result<Self, Error> {
        validate_tolerance(tolerance_mm)?;
        let ceiling = floor_mm + radius_mm;
        let [min, max] = self.piece.bounds;
        if !valid_height(floor_mm)
            || !valid_height(ceiling)
            || radius_mm <= tolerance_mm
            || min[2] < floor_mm
            || max[2] > ceiling
        {
            return Err(Error::Invalid);
        }
        for (face, plane) in self.piece.faces.iter().zip(&self.piece.planes) {
            let [min, max] = bounds(face.iter().copied());
            if plane.normal[2].abs() * (max[2] - min[2]) > tolerance_mm * 1e-3 {
                return Err(Error::Unresolved);
            }
        }
        self.map_z(
            |z| {
                if z == ceiling {
                    radius_mm
                } else {
                    let height = z - floor_mm;
                    (height * (2. * radius_mm - height)).sqrt()
                }
            },
            tolerance_mm,
        )
    }

    fn map_z(self, f: impl Fn(f64) -> f64, tolerance_mm: f64) -> Result<Self, Error> {
        let mut faces = self.piece.faces;
        for point in faces.iter_mut().flatten() {
            point[2] = f(point[2]);
        }
        Ok(Self {
            piece: ConvexPiece::new(faces, tolerance_mm)?,
            source_faces: self.source_faces,
        })
    }
}

type MarkedFace = (Vec<Point>, Option<usize>);

fn valid_height(z: f64) -> bool {
    z.is_finite() && z.abs() <= MAX_COORDINATE
}

fn clip(
    mut faces: Vec<MarkedFace>,
    z: f64,
    keep_above: bool,
    tolerance: f64,
) -> Result<Option<Vec<MarkedFace>>, Error> {
    let signed = |p: Point| if keep_above { p[2] - z } else { z - p[2] };
    let mut inside = false;
    let mut outside = false;
    for point in faces.iter_mut().flat_map(|(face, _)| face) {
        let d = signed(*point);
        if d.abs() <= tolerance {
            let resolution = (8. * f64::EPSILON * z.abs().max(1.)).max(tolerance * 1e-3);
            if d.abs() > resolution {
                return Err(Error::Unresolved);
            }
            point[2] = z;
        } else {
            inside |= d > 0.;
            outside |= d < 0.;
        }
    }
    if !inside {
        return Ok(None);
    }
    if !outside {
        return Ok(Some(faces));
    }

    let mut cap = Vec::new();
    let mut result = Vec::with_capacity(faces.len() + 1);
    for (face, source) in faces {
        let mut polygon = Vec::new();
        let mut off_cap = false;
        for (&a, &b) in face.iter().zip(face.iter().cycle().skip(1)) {
            let da = signed(a);
            let db = signed(b);
            if da >= 0. {
                polygon.push(a);
                off_cap |= da > 0.;
            }
            if (da < 0. && db > 0.) || (da > 0. && db < 0.) {
                let (low, high) = if a[2] < b[2] { (a, b) } else { (b, a) };
                let fraction = (z - low[2]) / (high[2] - low[2]);
                let mut intersection = add(low, scale(sub(high, low), fraction));
                intersection[2] = z;
                polygon.push(intersection);
            }
        }
        if off_cap && polygon.len() >= 3 {
            for &point in &polygon {
                if point[2] == z
                    && !cap
                        .iter()
                        .any(|&other| norm(sub(point, other)) <= tolerance)
                {
                    cap.push(point);
                }
            }
            result.push((polygon, source));
        }
    }
    let center = scale(
        cap.iter().copied().fold([0.; 3], add),
        1. / cap.len() as f64,
    );
    let angle = |point: Point| (point[1] - center[1]).atan2(point[0] - center[0]);
    cap.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)));
    if keep_above {
        cap.reverse();
    }
    result.push((cap, None));
    Ok(Some(result))
}
