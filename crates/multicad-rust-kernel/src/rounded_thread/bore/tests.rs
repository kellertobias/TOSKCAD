use super::*;
use crate::rounded_thread::Spec;
fn thread(left_hand: bool) -> Thread {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    Thread::new(Spec {
        major_radius_mm: 3.175,
        pitch_mm: p,
        length_mm: 3.7 * p,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand,
    })
    .unwrap()
}
#[test]
fn literal_through_material_and_cylindrical_support_are_independent_of_hand() {
    for left in [false, true] {
        let t = thread(left);
        let b = ThroughBore::new(t, 5.).unwrap();
        assert_eq!(b.thread(), &t);
        assert_eq!(b.radius_mm(), 5.);
        assert_eq!(b.length_mm(), t.spec().length_mm);
        assert_eq!(b.face_count(), 8);
        assert_eq!(b.outer_wall_face(), 7);
        assert!((b.volume_mm3() + t.volume_mm3() - PI * 25. * b.length_mm()).abs() < 1e-12);
        for z in [0., 1., b.length_mm()] {
            for theta in [0_f64, 0.31, 1.7, 4.2] {
                let (s, c) = theta.sin_cos();
                let r = t.radius(theta, z);
                assert!(!b.contains([0., 0., z], 0.));
                assert!(b.contains([4.9 * c, 4.9 * s, z], 0.));
                assert!(!b.contains([(r - 1e-6) * c, (r - 1e-6) * s, z], 0.));
                assert!(b.contains([(r + 1e-6) * c, (r + 1e-6) * s, z], 0.));
                assert!(!b.contains([5.1 * c, 5.1 * s, z], 0.));
            }
        }
        for d in [
            [1., 0., 0.],
            [0., 0., 1.],
            [0., 0., -1.],
            [0.4, -0.2, 0.7],
            [-2., 3., -1.],
            [0.; 3],
        ] {
            let bounds = b.support_bounds(d, 1e-10).unwrap();
            let expected = 5. * d[0].hypot(d[1]) + b.length_mm() * d[2].max(0.);
            assert!(
                bounds[0] <= expected && bounds[1] >= expected,
                "{d:?} {bounds:?} {expected}"
            );
            assert!(bounds[1] - bounds[0] <= 1e-10);
        }
    }
}
#[test]
fn invalid_domains_precision_and_query_refusals_do_not_modify_the_source() {
    let t = thread(false);
    let before = t;
    for r in [
        f64::NAN,
        f64::INFINITY,
        0.,
        t.spec().major_radius_mm,
        1e7 + 1.,
    ] {
        assert_eq!(ThroughBore::new(t, r), Err(Error::Invalid));
    }
    let b = ThroughBore::new(t, 5.).unwrap();
    for (d, tol) in [
        ([f64::NAN, 0., 0.], 1e-7),
        ([1., 0., 0.], 0.),
        ([1., 0., 0.], f64::NAN),
    ] {
        assert_eq!(b.support_bounds(d, tol), Err(Error::Invalid));
    }
    assert_eq!(
        b.support_bounds([1., 1., 1.], 1e-30),
        Err(Error::Unresolved)
    );
    for (p, tol) in [
        ([f64::NAN, 0., 0.], 0.),
        ([4., 0., f64::NAN], 0.),
        ([4., 0., -1.], 0.),
        ([4., 0., b.length_mm() + 1.], 0.),
        ([4., 0., 1.], -1.),
        ([4., 0., 1.], f64::NAN),
    ] {
        assert!(!b.contains(p, tol));
    }
    assert_eq!(t, before);
    assert!(b.contains([4., 0., 1.], 0.));
    assert!(b.volume_mm3() > 0.);
}
