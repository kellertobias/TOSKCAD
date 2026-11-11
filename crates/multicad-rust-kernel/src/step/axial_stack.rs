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
    // Circle directions in STEP are normalized, while shared vertices retain
    // their placed coordinates. In particular, a nearly rigid chart can turn
    // an exact tangent contact into a failed finite wire. Prove the actual
    // serialized shell at its original construction tolerance before touching
    // the destination member; neither document axes nor contact tolerances
    // are modified. The reader's entity/work budgets also bound this proof.
    if stack
        .edges()
        .iter()
        .any(|edge| matches!(edge.curve, Curve::Arc { .. }))
    {
        let mut candidate = StepFile::new();
        let solid = encode_shell(&mut candidate, &placement, stack)?;
        candidate.finish(vec![solid]);
        crate::step_read::read_import_solid(&candidate.data, stack.tolerance_mm())
            .map_err(|_| StepError::Unsupported)?;
    }
    encode_shell(file, &placement, stack)
}

fn encode_shell(
    file: &mut StepFile,
    placement: &Placement,
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
    fn affine_circle_charts_refuse_atomically_and_proper_charts_retry() {
        for size in [1., 1e-3] {
            let stack = ProfileStack::union(
                &[
                    Operand {
                        span_mm: [-10. * size, size],
                        profile: Profile::Rectangle {
                            min: [-size; 2],
                            max: [size; 2],
                        },
                    },
                    Operand {
                        span_mm: [0., 2. * size],
                        profile: Profile::Circle {
                            center: [0.; 2],
                            radius_mm: size,
                        },
                    },
                ],
                size * 1e-9,
                size * 1e-5,
            )
            .unwrap();
            let rectangular = ProfileStack::union(
                &[Operand {
                    span_mm: [0., size],
                    profile: Profile::Rectangle {
                        min: [-size; 2],
                        max: [size; 2],
                    },
                }],
                size * 1e-9,
                size * 1e-5,
            )
            .unwrap();
            let rectangular_step = write_axial_profile_stack(
                &rectangular,
                [0.; 3],
                [1. + 4e-11, 0., 0.],
                [0., 1., 0.],
            )
            .unwrap();
            assert!(crate::step_read::read_analytic_solid(&rectangular_step, size * 1e-9).is_ok());
            let mut writer = Writer::new();
            let dimensions = [3, 4, 5].map(|v| Rational::new(v, 1).unwrap());
            writer
                .append_box(dimensions, [0.; 3], [1., 0., 0.], [0., 1., 0.])
                .unwrap();
            for (u, v) in [
                ([1. + 4e-11, 0., 0.], [0., 1., 0.]),
                ([1., 0., 0.], [0., 1. + 4e-11, 0.]),
            ] {
                assert_eq!(
                    write_axial_profile_stack(&stack, [0.; 3], u, v),
                    Err(StepError::Unsupported)
                );
                assert_eq!(
                    writer.append_axial_profile_stack(&stack, [0.; 3], u, v),
                    Err(StepError::Unsupported)
                );
            }
            for (u, v) in [
                ([1., 0., 0.], [0., 1., 0.]),
                ([0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
            ] {
                let text = write_axial_profile_stack(&stack, [0.; 3], u, v).unwrap();
                let (read, _) = crate::step_read::read_import_solid(&text, size * 1e-9).unwrap();
                assert_eq!(read.face_count, stack.faces().len());
                assert!((read.volume_mm3 / stack.volume_mm3() - 1.).abs() < 1e-8);
            }
            writer
                .append_axial_profile_stack(&stack, [0.; 3], [1., 0., 0.], [0., 1., 0.])
                .unwrap();
            let mut expected = Writer::new();
            expected
                .append_box(dimensions, [0.; 3], [1., 0., 0.], [0., 1., 0.])
                .unwrap();
            expected
                .append_axial_profile_stack(&stack, [0.; 3], [1., 0., 0.], [0., 1., 0.])
                .unwrap();
            assert_eq!(writer.finish().unwrap(), expected.finish().unwrap());
        }
    }

    #[test]
    fn circular_exchange_uses_actual_size_and_original_tolerance() {
        let mut rejected = false;
        for size in [1e-3, 1., 5e6] {
            for tolerance in [1e-7, 1e-9] {
                let stack = ProfileStack::union(
                    &[
                        Operand {
                            span_mm: [0., size],
                            profile: Profile::Rectangle {
                                min: [-size; 2],
                                max: [size; 2],
                            },
                        },
                        Operand {
                            span_mm: [0., 2. * size],
                            profile: Profile::Circle {
                                center: [0.; 2],
                                radius_mm: size,
                            },
                        },
                    ],
                    tolerance,
                    1e-4,
                )
                .unwrap();
                for residual in [0., 2e-15, 4e-11] {
                    let result = write_axial_profile_stack(
                        &stack,
                        [0.; 3],
                        [1. + residual, 0., 0.],
                        [0., 1., 0.],
                    );
                    if residual == 0. {
                        assert!(result.is_ok(), "proper size={size}, tolerance={tolerance}");
                    }
                    match result {
                        Ok(text) => {
                            let (read, _) = crate::step_read::read_import_solid(&text, tolerance)
                                .expect(
                                    "every admitted file has independently proved finite incidence",
                                );
                            assert_eq!(read.face_count, stack.faces().len());
                        }
                        Err(error) => {
                            assert!(matches!(
                                error,
                                StepError::Unsupported | StepError::InvalidGeometry
                            ));
                            rejected = true;
                        }
                    }
                }
            }
        }
        assert!(rejected);
    }

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
