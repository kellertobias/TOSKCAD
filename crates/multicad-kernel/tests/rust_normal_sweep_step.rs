use multicad_kernel::{
    GeometryPolicy, Solid,
    rust_kernel::{
        normal_sweep::{Edge, Sweep},
        step::write_normal_sweep,
        step_read::{ImportProfile, read_import_solid},
    },
};
use std::f64::consts::PI;

#[test]
fn independent_native_reader_preserves_both_curvatures_in_an_oblique_sweep() {
    let line = |a, b| Edge::Line { a, b };
    let arc = |center, a, b, clockwise| Edge::Arc {
        center,
        a,
        b,
        clockwise,
    };
    let sweep = Sweep::new(
        vec![
            vec![
                line([0., 0.], [24., 0.]),
                line([24., 0.], [24., 15.]),
                arc([21., 15.], [24., 15.], [21., 18.], false),
                line([21., 18.], [0., 18.]),
                line([0., 18.], [0., 0.]),
            ],
            vec![
                line([3., 4.], [6., 7.]),
                arc([3., 7.], [6., 7.], [3., 4.], true),
            ],
            vec![Edge::Circle {
                center: [16., 10.],
                radius_mm: 1.5,
                clockwise: true,
            }],
        ],
        5.,
        1e-7,
        1e-5,
    )
    .unwrap();
    let volume = (24. * 18. - (9. - 9. * PI / 4.) - (9. * PI / 4. - 4.5) - 2.25 * PI) * 5.;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed-sweep.step");
    for reverse in [false, true] {
        let source = write_normal_sweep(
            &sweep,
            [900., -200., 500.],
            [0.8, 0.6, 0.],
            [-0.36, 0.48, 0.8],
            reverse,
        )
        .unwrap();
        let (analytic, reconstructed) = read_import_solid(&source, 1e-7).unwrap();
        let ImportProfile::NormalSweep { sweep: rebuilt } = reconstructed.profile else {
            panic!("the readback should retain exact circular arcs");
        };
        assert_eq!(analytic.arcs.len(), 4);
        assert_eq!(analytic.circles.len(), 2);
        assert_eq!(analytic.face_count, 10);
        assert!((rebuilt.volume_mm3() - volume).abs() < 1e-7);
        std::fs::write(&path, source).unwrap();
        let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(native.solid_count().unwrap(), 1);
        assert_eq!(native.faces().unwrap().len(), 10);
        assert!((native.volume_mm3().unwrap() - volume).abs() < 1e-7);
        assert!((analytic.volume_mm3 - volume).abs() < 1e-7);
        let bounds = native.bounds().unwrap();
        for i in 0..3 {
            assert!((bounds.min[i] - analytic.bounds_mm[0][i]).abs() < 1e-6);
            assert!((bounds.max[i] - analytic.bounds_mm[1][i]).abs() < 1e-6);
        }
    }
}
