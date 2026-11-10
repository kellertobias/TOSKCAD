//! STEP for a certified solid with arbitrarily oriented planar faces.

use super::*;
use crate::planar_boolean::Union;

/// Whether the certified boundary fits a single STEP closed shell.
pub fn supports_planar_union(union: &Union) -> bool {
    union.shell_count() == 1
        && union
            .faces()
            .iter()
            .all(|face| face.vertices.len() <= MAX_VERTICES)
}

/// Write a planar union after applying the given rigid placement.
pub fn write_planar_union(
    union: &Union,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_planar_union(union, origin, u, v))
}

pub(super) fn encode(
    file: &mut StepFile,
    placement: Placement,
    union: &Union,
) -> Result<Entity, StepError> {
    let vertices = union.vertices();
    let points = vertices
        .iter()
        .map(|&[x, y, z]| placement.point_f64([x, y], z))
        .collect();
    let mut brep = BRep::new(file, points);
    let mut faces = Vec::with_capacity(union.faces().len());
    for face in union.faces() {
        let boundary = brep.bound(&face.vertices, true)?;
        // Use the longest available edge in local coordinates so a short
        // segment at a large world offset cannot amplify placement roundoff.
        let length_squared = |a: usize, b: usize| {
            let delta = std::array::from_fn(|i| vertices[b][i] - vertices[a][i]);
            dot(delta, delta)
        };
        let (&a, &b) = face
            .vertices
            .iter()
            .zip(face.vertices.iter().cycle().skip(1))
            .max_by(|&(&a, &b), &(&c, &d)| length_squared(a, b).total_cmp(&length_squared(c, d)))
            .expect("a certified planar face has a nonempty loop");
        let tangent = direction(vertices[a], vertices[b])?;
        let point = brep.vertices[a].cartesian;
        let normal = placement.vector(face.normal);
        let reference = placement.vector(tangent);
        faces.push(brep.file.face(&[boundary], point, normal, reference));
    }
    Ok(brep.file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{planar_boolean::ConvexPiece, step_read::read_planar_solid};

    const TOL: f64 = 1e-7;
    const U: Point = [0.8, 0.6, 0.];
    const V: Point = [-0.36, 0.48, 0.8];

    fn block(min: Point, max: Point) -> ConvexPiece {
        ConvexPiece::from_box(min, max, TOL).unwrap()
    }

    #[test]
    fn exports_the_sloped_boundary_of_a_tilted_union() {
        let wedge = ConvexPiece::new(
            vec![
                vec![[1., 0., 0.], [3., 0., 0.], [1., 0., 2.]],
                vec![[1., 2., 0.], [1., 2., 2.], [3., 2., 0.]],
                vec![[1., 0., 0.], [1., 2., 0.], [3., 2., 0.], [3., 0., 0.]],
                vec![[1., 0., 0.], [1., 0., 2.], [1., 2., 2.], [1., 2., 0.]],
                vec![[3., 0., 0.], [3., 2., 0.], [1., 2., 2.], [1., 0., 2.]],
            ],
            TOL,
        )
        .unwrap();
        let union = Union::new(&[block([0.; 3], [2.; 3]), wedge], TOL).unwrap();
        assert!(supports_planar_union(&union));
        let origin = [13., -5., 41.];
        let text = write_planar_union(&union, origin, U, V).unwrap();
        let read = read_planar_solid(&text, TOL).unwrap();
        assert_eq!(read.face_count, union.faces().len());
        assert_eq!(read.solid_count, 1);
        assert!((read.volume_mm3 - 9.).abs() < TOL);
        let placement = Placement::new(origin, U, V).unwrap();
        for axis in 0..3 {
            let [min, max] = union.vertices().iter().fold(
                [f64::INFINITY, f64::NEG_INFINITY],
                |[lo, hi], &[x, y, z]| {
                    let coordinate = placement.point_f64([x, y], z)[axis];
                    [lo.min(coordinate), hi.max(coordinate)]
                },
            );
            assert!((read.bounds_mm[0][axis] - min).abs() < TOL);
            assert!((read.bounds_mm[1][axis] - max).abs() < TOL);
        }
        assert_eq!(
            write_planar_union(&union, origin, U, U),
            Err(StepError::InvalidPlacement)
        );
        assert_eq!(
            write_planar_union(&union, [1e30; 3], U, V),
            Err(StepError::InvalidGeometry)
        );
    }

    #[test]
    fn rejects_a_hidden_cavity_shell_instead_of_claiming_one_closed_boundary() {
        let union = Union::new(
            &[
                block([0., 0., 0.], [3., 3., 1.]),
                block([0., 0., 2.], [3., 3., 3.]),
                block([0., 0., 1.], [1., 3., 2.]),
                block([2., 0., 1.], [3., 3., 2.]),
                block([1., 0., 1.], [2., 1., 2.]),
                block([1., 2., 1.], [2., 3., 2.]),
            ],
            TOL,
        )
        .unwrap();
        assert_eq!(union.solid_count(), 1);
        assert!(!supports_planar_union(&union));
        assert_eq!(
            write_planar_union(&union, [0.; 3], U, V),
            Err(StepError::Unsupported)
        );
    }
}
