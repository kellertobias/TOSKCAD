use super::*;
use crate::{
    display::{MeshError, Tessellation, axial_profile_stack},
    manufacturing::certify,
};

const TOLERANCE: f64 = 1e-7;
const MINIMUM: f64 = 1e-5;
const CIRCLE: Profile = Profile::Circle {
    center: [0.; 2],
    radius_mm: 6.,
};
const POST: Profile = Profile::Rectangle {
    min: [-6., -29.],
    max: [6., 6.],
};

fn operand(span_mm: [f64; 2], profile: Profile) -> Operand {
    Operand { span_mm, profile }
}

fn union(operands: &[Operand]) -> Result<ProfileStack, Error> {
    ProfileStack::union(operands, TOLERANCE, MINIMUM)
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

fn options() -> Tessellation {
    Tessellation {
        deflection: 0.001,
        angle: 0.15,
        triangle_limit: 20_000,
    }
}

#[test]
fn real_handle_stack_keeps_three_exact_transition_patches_and_one_manifold() {
    let stack = union(&[
        operand([102.5, 114.5], POST),
        operand([230.5, 242.5], POST),
        operand([97.5, 247.5], CIRCLE),
    ])
    .unwrap();
    assert_eq!(stack.bounds_mm(), [[-6., -29., 97.5], [6., 6., 247.5]]);
    near(stack.volume_mm3(), 24_330.264_276_683_27);
    near(
        stack.faces().iter().map(|face| face.area_mm2).sum(),
        8_459.893_421_169_307,
    );
    for (level, expected_sign, post) in [
        (102.5, -1., 0),
        (114.5, 1., 0),
        (230.5, -1., 1),
        (242.5, 1., 1),
    ] {
        let support = Support::Plane {
            normal: [0., 0., expected_sign],
        };
        let mut areas = stack
            .faces()
            .iter()
            .filter(|face| face.support == support && face.center_mm[2] == level)
            .map(|face| {
                assert_eq!(face.sources, [post, 2]);
                face.area_mm2
            })
            .collect::<Vec<_>>();
        areas.sort_by(f64::total_cmp);
        assert_eq!(areas.len(), 3);
        for (a, b) in
            areas
                .into_iter()
                .zip([7.725_666_117_692, 7.725_666_117_692, 291.451_332_235_384])
        {
            near(a, b);
        }
    }
    let mut uses = vec![[0; 2]; stack.edges().len()];
    for face in stack.faces() {
        let traversals: Vec<_> = face
            .boundary
            .iter()
            .map(|rim| {
                uses[rim.edge][usize::from(rim.forward)] += 1;
                let mut ends = stack.edges()[rim.edge].vertices;
                if !rim.forward {
                    ends.reverse();
                }
                ends
            })
            .collect();
        assert!(
            traversals
                .iter()
                .zip(traversals.iter().cycle().skip(1))
                .all(|(a, b)| a[1] == b[0])
        );
    }
    assert!(uses.into_iter().all(|directions| directions == [1, 1]));
    let mesh = axial_profile_stack(&stack, options()).unwrap();
    let proof = certify(&mesh).unwrap();
    assert!((proof.signed_volume_mm3 - stack.volume_mm3()).abs() < 4.);
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    assert_eq!(
        mesh.face_indices.iter().copied().max(),
        Some(stack.faces().len() as u32 - 1)
    );
    assert_eq!(
        axial_profile_stack(
            &stack,
            Tessellation {
                triangle_limit: mesh.face_indices.len() - 1,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn contacting_spans_coalesce_and_two_or_four_tangencies_keep_manifold_patches() {
    for rectangle in [
        Profile::Rectangle {
            min: [-6., -11.],
            max: [6., 14.],
        },
        Profile::Rectangle {
            min: [-9., -10.],
            max: [6., 6.],
        },
        Profile::Rectangle {
            min: [-6., -6.],
            max: [6., 6.],
        },
    ] {
        let stack = union(&[
            operand([-4., 0.], CIRCLE),
            operand([0., 8.], rectangle),
            operand([5., 10.], rectangle),
        ])
        .unwrap();
        let mesh = axial_profile_stack(&stack, options()).unwrap();
        assert!(certify(&mesh).is_ok());
        let cap = stack
            .faces()
            .iter()
            .find(|face| face.center_mm[2] == 10.)
            .unwrap();
        assert_eq!(cap.sources, [1, 2]);
    }
    let circular = union(&[operand([0., 3.], CIRCLE)]).unwrap();
    let rectangular = union(&[operand([0., 3.], POST)]).unwrap();
    near(circular.volume_mm3(), PI * 108.);
    near(rectangular.volume_mm3(), 12. * 35. * 3.);
    for stack in [circular, rectangular] {
        assert!(certify(&axial_profile_stack(&stack, options()).unwrap()).is_ok());
    }
}

#[test]
fn gaps_incompatible_profiles_and_unproven_contacts_are_rejected() {
    assert_eq!(union(&[]), Err(Error::Invalid));
    assert_eq!(
        union(&[operand([0., 2.], POST); MAX_OPERANDS + 1]),
        Err(Error::Limit)
    );
    assert_eq!(
        ProfileStack::union(&[operand([0., 2.], POST)], TOLERANCE, TOLERANCE),
        Err(Error::Invalid)
    );
    assert_eq!(union(&[operand([0., 0.], POST)]), Err(Error::Invalid));
    for profile in [
        Profile::Circle {
            center: [0.; 2],
            radius_mm: f64::NAN,
        },
        Profile::Rectangle {
            min: [2., 0.],
            max: [1., 9.],
        },
    ] {
        assert_eq!(union(&[operand([0., 2.], profile)]), Err(Error::Invalid));
    }
    assert_eq!(
        union(&[operand([0., 2.], POST), operand([3., 4.], POST)]),
        Err(Error::Disconnected)
    );
    for rectangle in [
        Profile::Rectangle {
            min: [-5., -29.],
            max: [6., 6.],
        },
        Profile::Rectangle {
            min: [-7., -29.],
            max: [7., 6.],
        },
        Profile::Rectangle {
            min: [-6., -29.],
            max: [6., 6. + MINIMUM * 0.5],
        },
    ] {
        assert_eq!(
            union(&[operand([0., 3.], CIRCLE), operand([2., 6.], rectangle)]),
            Err(Error::Unsupported)
        );
    }
    assert_eq!(
        union(&[
            operand([0., 3.], POST),
            operand(
                [2., 4.],
                Profile::Rectangle {
                    min: [-6., -6.],
                    max: [6., 6.]
                }
            )
        ]),
        Err(Error::Unsupported)
    );
    assert_eq!(
        union(&[operand([0., 3.], CIRCLE), operand([MINIMUM / 2., 4.], POST)]),
        Err(Error::Invalid)
    );
    let tiny = Profile::Circle {
        center: [0.; 2],
        radius_mm: MINIMUM,
    };
    let tight = Profile::Rectangle {
        min: [-MINIMUM; 2],
        max: [MINIMUM; 2],
    };
    assert_eq!(
        union(&[operand([0., 3.], tiny), operand([2., 4.], tight)]),
        Err(Error::Unsupported)
    );
}
