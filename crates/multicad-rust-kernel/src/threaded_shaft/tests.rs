use super::*;
use crate::{display::Tessellation, helical_thread::Spec, helical_thread_mesh, manufacturing};
pub(super) fn shaft(left: bool, starts: u32, tangent: bool, rounded: bool) -> Shaft {
    let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
    let thread = Thread::new(Spec {
        major_radius_mm: 3.,
        pitch_mm: 1.,
        length_mm: 2.37,
        depth_mm: root.map_or(5. * 3_f64.sqrt() / 16., |r| 0.875 * 3_f64.sqrt() / 2. - r),
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts,
        left_hand: left,
        root_radius_mm: root,
    })
    .unwrap();
    Shaft::new(thread, if tangent { 3. } else { 4. }, 5.).unwrap()
}
#[test]
fn exact_thread_prefix_and_plain_tail_share_one_closed_material_boundary() {
    let options = Tessellation {
        deflection: 0.07,
        angle: 0.3,
        triangle_limit: 200_000,
    };
    for n in [1, 2, 8] {
        for left in [false, true] {
            for tangent in [false, true] {
                for rounded in [false, true] {
                    let body = shaft(left, n, tangent, rounded);
                    assert!(body.contains([0., 0., 1.], 0.));
                    assert!(body.contains([2.99, 0., 4.], 0.));
                    assert!(!body.contains([body.radius_mm() + 1., 0., 4.], 0.));
                    assert!(!body.contains([0., 0., 6.], 0.));
                    assert!(!body.contains([0., 0., f64::NAN], 0.));
                    assert!(!body.contains([0., 0., 4.], -1.));
                    let expected = body.thread().volume_mm3()
                        + std::f64::consts::PI * body.radius_mm().powi(2) * (5. - 2.37);
                    assert_eq!(body.volume_mm3(), expected);
                    assert_eq!(body.support([0., 0., 1.]), 5.);
                    assert_eq!(body.support([0., 0., -1.]), 0.);
                    assert_eq!(body.support([1., 0., 0.]), body.radius_mm());
                    let mesh = helical_thread_mesh::tessellate_shaft(&body, options).unwrap();
                    let proof = manufacturing::certify(&mesh).unwrap();
                    assert!(
                        (proof.signed_volume_mm3 - expected).abs()
                            <= helical_thread_mesh::shaft_volume_error_bound(&body, options)
                    );
                    assert_eq!(
                        mesh.face_indices
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>(),
                        (0..body.face_count()).collect()
                    );
                    for &face in &mesh.face_indices {
                        assert!(face < body.face_count());
                    }
                }
            }
        }
    }
}
#[test]
fn invalid_dimensions_and_mesh_work_refuse_without_mutating_source() {
    let body = shaft(false, 1, true, false);
    for (radius, height, error) in [
        (f64::NAN, 5., ThreadError::Invalid),
        (3., f64::NAN, ThreadError::Invalid),
        (2., 5., ThreadError::Invalid),
        (3., 2.37, ThreadError::Invalid),
        (2e7, 5., ThreadError::Limit),
        (3., 2e7, ThreadError::Limit),
    ] {
        assert_eq!(Shaft::new(*body.thread(), radius, height), Err(error));
    }
    let options = Tessellation {
        deflection: 0.07,
        angle: 0.3,
        triangle_limit: 1,
    };
    assert!(helical_thread_mesh::tessellate_shaft(&body, options).is_err());
    let mesh = helical_thread_mesh::tessellate_shaft(
        &body,
        Tessellation {
            triangle_limit: 200_000,
            ..options
        },
    )
    .unwrap();
    assert!(manufacturing::certify(&mesh).is_ok());
    assert_eq!(body, shaft(false, 1, true, false));
}
