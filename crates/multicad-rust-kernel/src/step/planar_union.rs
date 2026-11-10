//! STEP for certified solids with arbitrarily oriented planar faces.

use super::*;
use crate::planar_boolean::Union;

/// Whether every shell can be written as an independent outward STEP solid.
pub fn supports_planar_union(union: &Union) -> bool {
    union.shell_count() == union.solid_count()
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
    placement: &Placement,
    union: &Union,
    face_indices: &[usize],
) -> Result<Entity, StepError> {
    let vertices = union.vertices();
    let placed = |[x, y, z]: Point| placement.point_f64([x, y], z);
    let mut local_indices = BTreeMap::new();
    let points = if union.shell_count() == 1 {
        vertices.iter().copied().map(placed).collect()
    } else {
        let mut points = Vec::new();
        for &index in face_indices
            .iter()
            .flat_map(|&face| &union.faces()[face].vertices)
        {
            local_indices.entry(index).or_insert_with(|| {
                let local = points.len();
                points.push(placed(vertices[index]));
                local
            });
        }
        points
    };
    let mut brep = BRep::new(file, points);
    let mut faces = Vec::with_capacity(face_indices.len());
    for &face in face_indices {
        let face = &union.faces()[face];
        let boundary = if local_indices.is_empty() {
            brep.bound(&face.vertices, true)?
        } else {
            let ring: Vec<_> = face.vertices.iter().map(|i| local_indices[i]).collect();
            brep.bound(&ring, true)?
        };
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
        let point = brep.vertices[local_indices.get(&a).copied().unwrap_or(a)].cartesian;
        let normal = placement.vector(face.normal);
        let reference = placement.vector(tangent);
        faces.push(brep.file.face(&[boundary], point, normal, reference));
    }
    Ok(brep.file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        planar_boolean::ConvexPiece,
        step_read::{read_analytic_compound, read_planar_solid},
    };

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
    fn exports_one_member_per_outward_shell_after_partial_fusion() {
        let union = Union::new(
            &[
                block([0.; 3], [2.; 3]),
                block([1., 0., 0.], [3., 2., 1.]),
                block([10., 3., 2.], [11., 5., 5.]),
            ],
            TOL,
        )
        .unwrap();
        assert!(supports_planar_union(&union));
        assert_eq!(union.solid_count(), 2);
        let origin = [13., -5., 41.];
        let text = write_planar_union(&union, origin, U, V).unwrap();
        assert_eq!(text.matches("MANIFOLD_SOLID_BREP(").count(), 2);
        let read = read_analytic_compound(&text, TOL).unwrap();
        assert_eq!(read.len(), 2);
        let placement = Placement::new(origin, U, V).unwrap();
        for ((shell, member), (volume, sources)) in union
            .shells()
            .iter()
            .zip(&read)
            .zip([(10., 0..2), (6., 2..3)])
        {
            assert!(shell.faces.is_sorted());
            assert!(
                shell
                    .faces
                    .iter()
                    .all(|&face| sources.contains(&union.faces()[face].source))
            );
            assert!((shell.signed_volume_mm3 - volume).abs() < TOL);
            assert!((member.volume_mm3 - volume).abs() < TOL);
            assert_eq!(member.face_count, shell.faces.len());
            assert_eq!(member.solid_count, 1);
            let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
            for &face in &shell.faces {
                for &index in &union.faces()[face].vertices {
                    let [x, y, z] = union.vertices()[index];
                    for (axis, coordinate) in placement.point_f64([x, y], z).into_iter().enumerate()
                    {
                        bounds[0][axis] = bounds[0][axis].min(coordinate);
                        bounds[1][axis] = bounds[1][axis].max(coordinate);
                    }
                }
            }
            for (actual, expected) in member
                .bounds_mm
                .into_iter()
                .flatten()
                .zip(bounds.into_iter().flatten())
            {
                assert!((actual - expected).abs() < TOL);
            }
        }
    }

    #[test]
    fn rolls_back_all_members_when_a_later_placed_shell_collapses() {
        let tolerance = 1e-10;
        let thin = ConvexPiece::from_box([10., 0., 0.], [10. + 1e-8, 1., 1.], tolerance).unwrap();
        let union = Union::new(&[block([0.; 3], [1.; 3]), thin], tolerance).unwrap();
        assert_eq!(union.shell_count(), 2);
        let unit = Union::new(&[block([0.; 3], [1.; 3])], TOL).unwrap();
        let mut writer = Writer::new();
        writer.append_planar_union(&unit, [0.; 3], U, V).unwrap();
        assert_eq!(
            writer.append_planar_union(&union, [1e10; 3], [1., 0., 0.], [0., 1., 0.]),
            Err(StepError::InvalidGeometry)
        );
        assert_eq!(
            writer.finish().unwrap(),
            write_planar_union(&unit, [0.; 3], U, V).unwrap()
        );
    }

    #[test]
    fn refuses_all_members_when_the_union_exceeds_remaining_capacity() {
        let union = Union::new(&[block([0.; 3], [1.; 3]), block([2.; 3], [3.; 3])], TOL).unwrap();
        let mut writer = Writer::new();
        for index in 0..MAX_COMPOUND_SOLIDS - 1 {
            writer
                .append_placed_circular_sweep(1., None, 2., [3. * index as f64, 0., 0.], U, V)
                .unwrap();
        }
        assert_eq!(
            writer.append_planar_union(&union, [0.; 3], U, V),
            Err(StepError::Limit)
        );
        assert_eq!(
            writer
                .finish()
                .unwrap()
                .matches("MANIFOLD_SOLID_BREP(")
                .count(),
            MAX_COMPOUND_SOLIDS - 1
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
        assert_eq!(union.shell_count(), 2);
        assert!((union.shells()[0].signed_volume_mm3 - 27.).abs() < TOL);
        assert!((union.shells()[1].signed_volume_mm3 + 1.).abs() < TOL);
        assert!(!supports_planar_union(&union));
        assert_eq!(
            write_planar_union(&union, [0.; 3], U, V),
            Err(StepError::Unsupported)
        );
    }
}
