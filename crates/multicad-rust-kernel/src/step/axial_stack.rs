//! Encode the certified shared shell of a mixed axial profile stack.

use super::*;
use crate::axial_profile_stack::{Curve, ProfileStack, Support};

/// Write a single axial union with exact planar and partial cylindrical faces.
pub fn write_axial_profile_stack(
    stack: &ProfileStack,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_axial_profile_stack(stack, origin, u, v))
}

pub(super) fn encode(
    file: &mut StepFile,
    placement: Placement,
    stack: &ProfileStack,
) -> Result<Entity, StepError> {
    let placed = |[x, y, z]: Point| placement.point_f64([x, y], z);
    let vertices: Vec<_> = stack
        .vertices()
        .iter()
        .copied()
        .map(|p| file.vertex(placed(p)))
        .collect();
    let edges = stack
        .edges()
        .iter()
        .map(|edge| {
            let ends = edge.vertices.map(|index| vertices[index]);
            match edge.curve {
                Curve::Line => Ok((line_edge(file, ends[0], ends[1])?, None)),
                Curve::Arc {
                    center_mm,
                    radius_mm,
                    sweep_rad,
                } => {
                    let [x, y, _] = stack.vertices()[edge.vertices[0]];
                    let reference = placement.vector([
                        (x - center_mm[0]) / radius_mm,
                        (y - center_mm[1]) / radius_mm,
                        0.,
                    ]);
                    let normal = placement
                        .normal
                        .map(|component| component * sweep_rad.signum());
                    let (edge, axis) = circular_arc_edge(
                        file,
                        ends,
                        placed(center_mm),
                        normal,
                        reference,
                        radius_mm,
                        sweep_rad,
                    )?;
                    Ok((edge, Some(axis)))
                }
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let faces: Vec<_> = stack
        .faces()
        .iter()
        .map(|face| {
            let boundary: Vec<_> = face
                .boundary
                .iter()
                .map(|use_| file.oriented_edge(edges[use_.edge].0, use_.forward))
                .collect();
            let bound = file.bound(&boundary, true);
            let first = face.boundary[0].edge;
            match face.support {
                Support::Plane { normal } => {
                    let point = vertices[stack.edges()[first].vertices[0]].cartesian;
                    let reference = if normal[2] == 0. {
                        placement.normal
                    } else {
                        placement.u
                    };
                    file.face(&[bound], point, placement.vector(normal), reference)
                }
                Support::Cylinder { radius_mm, .. } => {
                    let axis = edges[first]
                        .1
                        .expect("a cylindrical stack wall starts with an arc rim");
                    let cylinder = file.entity(format_args!(
                        "CYLINDRICAL_SURFACE('',{axis},{radius_mm:.16E})"
                    ));
                    file.face_on(&[bound], cylinder, true)
                }
            }
        })
        .collect();
    Ok(file.solid(&faces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        axial_profile_stack::{Operand, Profile},
        step_read::read_analytic_solid,
    };

    #[test]
    fn tilted_stack_preserves_tangent_patches_and_exact_cylindrical_faces() {
        let section = |span_mm, profile| Operand { span_mm, profile };
        let circle = Profile::Circle {
            center: [0., 0.],
            radius_mm: 6.,
        };
        for (rectangle, expected_faces) in [
            (
                Profile::Rectangle {
                    min: [-6., -29.],
                    max: [6., 6.],
                },
                31,
            ),
            (
                Profile::Rectangle {
                    min: [-8., -29.],
                    max: [6., 6.],
                },
                24,
            ),
        ] {
            let stack = ProfileStack::union(
                &[
                    section([5., 17.], rectangle),
                    section([133., 145.], rectangle),
                    section([0., 150.], circle),
                ],
                1e-7,
                1e-5,
            )
            .unwrap();
            let step = write_axial_profile_stack(
                &stack,
                [13., -5., 41.],
                [0.8, 0.6, 0.],
                [-0.36, 0.48, 0.8],
            )
            .unwrap();
            let readback = read_analytic_solid(&step, 1e-7).unwrap();
            assert_eq!(readback.solid_count, 1);
            assert_eq!(readback.face_count, expected_faces);
            assert_eq!(readback.face_count, stack.faces().len());
            assert!((readback.volume_mm3 - stack.volume_mm3()).abs() < 1e-7);
            assert_eq!(
                write_axial_profile_stack(&stack, [1e30; 3], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
                Err(StepError::InvalidGeometry),
            );
        }
    }
}
