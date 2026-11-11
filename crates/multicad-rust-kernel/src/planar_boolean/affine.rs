//! Indexed affine maps retain the certified embedding and every boundary identity.
use super::*;

/// A finite orientation-preserving affine map with bounded matrix and inverse.
/// Unlike an arbitrary point callback, this map preserves planes and convexity.
#[derive(Clone, Copy, Debug)]
pub struct AffineMap {
    matrix: [Point; 3],
    translation: Point,
    cofactors: [Point; 3],
    determinant: f64,
    rigid_input: Option<RigidInput>,
}

#[derive(Clone, Copy, Debug)]
struct RigidInput {
    matrix: [Point; 3],
    translation: Point,
    cofactors: [Point; 3],
    determinant: f64,
}

// FMA captures product residuals; TwoSum captures each rounded addition.
// Accumulated absolute residuals bound error without assuming cancellation is safe.
fn evaluated(matrix: [Point; 3], translation: Point, point: Point) -> (Point, Point) {
    let mut error = [0.; 3];
    let mapped = std::array::from_fn(|row| {
        let mut sum = 0.;
        for (a, b) in matrix[row]
            .into_iter()
            .zip(point)
            .chain([(1., translation[row])])
        {
            let product = a * b;
            let next = sum + product;
            let z = next - sum;
            error[row] = (error[row]
                + a.mul_add(b, -product).abs()
                + ((sum - (next - z)) + (product - z)).abs())
            .next_up();
            sum = next;
        }
        sum
    });
    (mapped, error)
}

impl AffineMap {
    pub fn new(matrix: [Point; 3], translation: Point) -> Result<Self, Error> {
        if matrix
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e6)
            || translation
                .iter()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
        {
            return Err(Error::Invalid);
        }
        let cofactors = [
            cross(matrix[1], matrix[2]),
            cross(matrix[2], matrix[0]),
            cross(matrix[0], matrix[1]),
        ];
        let determinant = dot(matrix[0], cofactors[0]);
        if !determinant.is_finite()
            || determinant <= 0.
            || cofactors.iter().flatten().any(|v| {
                !(*v / determinant).is_finite() || (*v / determinant).abs() > 1e6 * (1. + 1e-12)
            })
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            matrix,
            translation,
            cofactors,
            determinant,
            rigid_input: None,
        })
    }

    /// Compose one independently validated rigid source chart before this map.
    /// Keep both factors so valid public coefficients are never tested against
    /// artificial bounds on their product, or rounded into one large offset.
    pub fn with_rigid_input(mut self, origin: Point, u: Point, v: Point) -> Result<Self, Error> {
        if self.rigid_input.is_some()
            || u.iter().chain(&v).any(|v| !v.is_finite())
            || (dot(u, u) - 1.).abs() > 1e-10
            || (dot(v, v) - 1.).abs() > 1e-10
            || dot(u, v).abs() > 1e-10
        {
            return Err(Error::Invalid);
        }
        let axes = [u, v, cross(u, v)];
        let pose = Self::new(std::array::from_fn(|i| axes.map(|axis| axis[i])), origin)?;
        self.rigid_input = Some(RigidInput {
            matrix: pose.matrix,
            translation: pose.translation,
            cofactors: pose.cofactors,
            determinant: pose.determinant,
        });
        Ok(self)
    }

    pub fn point(self, point: Point) -> Point {
        self.point_with_error(point).0
    }

    fn point_with_error(self, point: Point) -> (Point, Point) {
        let (point, input_error) = self.rigid_input.map_or((point, [0.; 3]), |pose| {
            evaluated(pose.matrix, pose.translation, point)
        });
        let (mapped, mut error) = evaluated(self.matrix, self.translation, point);
        for (row, error) in self.matrix.iter().zip(&mut error) {
            *error += row
                .iter()
                .zip(input_error)
                .map(|(a, b)| a.abs() * b)
                .sum::<f64>();
        }
        (
            mapped,
            error.map(|error| (error * (1. + 16. * f64::EPSILON)).next_up()),
        )
    }

    fn normal(self, normal: Point) -> Point {
        let normal = self
            .rigid_input
            .map_or(normal, |pose| pose.cofactors.map(|row| dot(row, normal)));
        let normal = self.cofactors.map(|row| dot(row, normal));
        scale(normal, 1. / norm(normal))
    }

    fn determinant(self) -> f64 {
        self.determinant * self.rigid_input.map_or(1., |pose| pose.determinant)
    }
}

impl Union {
    /// Map an already certified embedding without welding, clipping, reordering
    /// loops or interpreting its source labels. Positive invertibility preserves
    /// nesting and disjointness; the indexed shell proof checks the stored result.
    pub fn affine_transformed(&self, map: AffineMap, tolerance: f64) -> Result<Self, Error> {
        validate_tolerance(tolerance)?;
        let mut identities = BTreeMap::new();
        let vertices = self
            .vertices
            .iter()
            .map(|&point| {
                let (mapped, roundoff) = map.point_with_error(point);
                if roundoff
                    .into_iter()
                    .any(|e| !e.is_finite() || e > tolerance * 0.25)
                {
                    return Err(Error::Unresolved);
                }
                if mapped
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
                {
                    return Err(Error::Invalid);
                }
                let key = mapped.map(|v| if v == 0. { 0 } else { v.to_bits() });
                if identities
                    .insert(key, point)
                    .is_some_and(|other| other != point)
                {
                    return Err(Error::Unresolved);
                }
                Ok(mapped)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut budget = MAX_WORK;
        let faces = self
            .faces
            .iter()
            .map(|face| {
                charge(
                    &mut budget,
                    face.vertices.len().saturating_mul(face.vertices.len()),
                )?;
                let ring: Vec<_> = face.vertices.iter().map(|&i| vertices[i]).collect();
                let (plane, _) = planar_ring(&ring, tolerance)?;
                let normal = map.normal(face.normal);
                if dot(plane.normal, normal) < 1. - 1e-10 {
                    return Err(Error::Unresolved);
                }
                Ok(Face {
                    normal,
                    ..face.clone()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        verify_boundary::verify_indexed_embedding(&vertices, &faces, tolerance, &mut budget)?;
        let transformed = Self::from_indexed_boundary(vertices, faces)?;
        if transformed.solid_count != self.solid_count
            || transformed.shells.len() != self.shells.len()
            || transformed
                .shells
                .iter()
                .zip(&self.shells)
                .any(|(actual, source)| {
                    let expected = source.signed_volume_mm3 * map.determinant();
                    actual.faces != source.faces
                        || !expected.is_finite()
                        || expected == 0.
                        || (actual.signed_volume_mm3 - expected).abs() > expected.abs() * 1e-8
                })
        {
            return Err(Error::Unresolved);
        }
        Ok(transformed)
    }
}

#[cfg(test)]
mod tests;
