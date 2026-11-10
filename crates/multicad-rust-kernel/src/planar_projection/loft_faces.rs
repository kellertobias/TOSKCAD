use super::*;

pub(crate) struct LoftFaces {
    pub(crate) faces: Vec<TrimmedPlane>,
    pub(crate) tangent: Vec<Vec<bool>>,
    pub(crate) remaining: usize,
}

pub(crate) fn prepare_loft_faces(
    bodies: &[&Body],
    options: &Options,
    limit: usize,
) -> Result<LoftFaces, ProjectionError> {
    let mut work = projection_work(options, limit)?;
    let (faces, edges) = prepare(bodies.iter().copied(), &mut work)?;
    Ok(LoftFaces {
        faces,
        tangent: edges
            .into_iter()
            .map(|body| body.into_iter().map(|edge| edge.tangent).collect())
            .collect(),
        remaining: work.remaining,
    })
}

impl TrimmedPlane {
    pub(crate) fn boundary_len(&self) -> usize {
        self.boundary.len()
    }
    pub(crate) fn covers(&self, point: Point, tolerance: f64) -> bool {
        let delta = sub(self.point, point);
        let height = dot(self.normal, delta);
        let roundoff = (0..3)
            .map(|i| self.normal[i].abs() * (self.point[i].abs() + point[i].abs()))
            .sum::<f64>()
            * (16. * f64::EPSILON);
        if height <= (self.normal[2] * tolerance).max(roundoff)
            || !(0..2).all(|i| {
                (self.bounds.min[i] - tolerance..=self.bounds.max[i] + tolerance)
                    .contains(&point[i])
            })
        {
            return false;
        }
        let mut inside = false;
        for &[a, b] in &self.boundary {
            let edge = sub(b, a);
            let offset = sub(planar(point), a);
            let determinant = edge[0].mul_add(offset[1], -edge[1] * offset[0]);
            let length = edge[0].hypot(edge[1]);
            if determinant.abs() <= tolerance * length
                && dot(offset, edge) >= -tolerance * length
                && dot(sub(planar(point), b), edge) <= tolerance * length
                && length > tolerance
            {
                return true;
            }
            if (a[1] > point[1]) != (b[1] > point[1]) && determinant * edge[1] > 0. {
                inside = !inside;
            }
        }
        inside
    }
}
