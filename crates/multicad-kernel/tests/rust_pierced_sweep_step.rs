use multicad_kernel::{
    GeometryPolicy, Solid,
    rust_kernel::{
        normal_sweep::{Edge, PiercedSweep, Piercing, Sweep},
        step::write_pierced_normal_sweep,
        step_read::{ImportProfile, read_analytic_solid, read_import_solid},
    },
};
use std::f64::consts::PI;

#[test]
fn independent_native_reader_recovers_both_paired_openings() {
    let line = |a, b| Edge::Line { a, b };
    let sweep = Sweep::new(
        vec![vec![
            line([0., 0.], [24., 0.]),
            line([24., 0.], [24., 9.]),
            Edge::Arc {
                center: [21., 9.],
                a: [24., 9.],
                b: [21., 12.],
                clockwise: false,
            },
            line([21., 12.], [0., 12.]),
            line([0., 12.], [0., 0.]),
        ]],
        12.,
        1e-7,
        1e-5,
    )
    .unwrap();
    let opening = |outline| Piercing {
        sides: [0, 3],
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
        through: [0., 12., 0.],
        outline,
    };
    let rectangle = [[14., 5.], [18., 5.], [18., 8.], [14., 8.]];
    let pierced = PiercedSweep::new(
        &sweep,
        vec![
            opening(vec![Edge::Circle {
                center: [7., 3.],
                radius_mm: 1.5,
                clockwise: false,
            }]),
            opening(
                rectangle
                    .into_iter()
                    .zip(rectangle.into_iter().cycle().skip(1))
                    .map(|(a, b)| line(a, b))
                    .collect(),
            ),
        ],
        1e-7,
        1e-5,
    )
    .unwrap();
    let volume = sweep.volume_mm3() - (12. + 2.25 * PI) * 12.;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pierced-sweep.step");
    for reverse in [false, true] {
        let source = write_pierced_normal_sweep(
            &pierced,
            [900., -200., 500.],
            [0.8, 0.6, 0.],
            [-0.36, 0.48, 0.8],
            reverse,
        )
        .unwrap();
        let analytic = read_analytic_solid(&source, 1e-7).unwrap();
        let (readback, rebuilt) = read_import_solid(&source, 1e-7).unwrap();
        let ImportProfile::PiercedNormalSweep { sweep: rebuilt } = rebuilt.profile else {
            panic!("STEP import must retain both paired openings");
        };
        assert_eq!(analytic, readback);
        assert_eq!(rebuilt.openings().len(), 2);
        assert!((rebuilt.volume_mm3() - volume).abs() < 1e-7);
        assert_eq!(analytic.arcs.len(), 2);
        assert_eq!(analytic.circles.len(), 2);
        assert_eq!(analytic.face_count, 12);
        std::fs::write(&path, source).unwrap();
        let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(native.solid_count().unwrap(), 1);
        assert_eq!(native.faces().unwrap().len(), 12);
        assert!((native.volume_mm3().unwrap() - volume).abs() < 1e-7);
        assert!((analytic.volume_mm3 - volume).abs() < 1e-7);
        let bounds = native.bounds().unwrap();
        for i in 0..3 {
            assert!((bounds.min[i] - analytic.bounds_mm[0][i]).abs() < 1e-6);
            assert!((bounds.max[i] - analytic.bounds_mm[1][i]).abs() < 1e-6);
        }
    }
}
