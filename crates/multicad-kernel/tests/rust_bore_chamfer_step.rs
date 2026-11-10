use multicad_kernel::{
    GeometryPolicy, Solid,
    rust_kernel::{
        Circle, Rational, chamfered_bores::BoreChamfer, polygon::PolygonPrism,
        step::write_chamfered_polygon_prism, step_read::read_analytic_solid,
    },
};
use std::f64::consts::PI;

#[test]
fn independent_native_step_reader_recovers_both_exact_countersinks() {
    let exact = |value| Rational::new(value, 1).unwrap();
    let corners = [[0, 0], [24, 0], [24, 18], [0, 18]];
    let prism = PolygonPrism::with_circular_holes(
        vec![corners.map(|uv| uv.map(exact)).to_vec()],
        vec![Circle::new([6, 9].map(exact), exact(2)).unwrap()],
        exact(8),
    )
    .unwrap();
    let chamfers = [
        BoreChamfer {
            hole: 0,
            end: false,
            size_mm: 1.,
        },
        BoreChamfer {
            hole: 0,
            end: true,
            size_mm: 2.,
        },
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("countersinks.step");
    let placement = [900., -200., 500.];
    let u = [0.8, 0.6, 0.];
    let v = [-0.36, 0.48, 0.8];
    let volume = 24. * 18. * 8. - PI * (4. * 5. + (4. + 6. + 9.) / 3. + 2. * (4. + 8. + 16.) / 3.);
    for reverse in [false, true] {
        let source =
            write_chamfered_polygon_prism(&prism, &chamfers, placement, u, v, reverse).unwrap();
        let analytic = read_analytic_solid(&source, 1e-7).unwrap();
        std::fs::write(&path, source).unwrap();
        let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(native.solid_count().unwrap(), 1);
        assert_eq!(native.faces().unwrap().len(), 9);
        assert!((native.volume_mm3().unwrap() - volume).abs() < 1e-7);
        assert!((analytic.volume_mm3 - volume).abs() < 1e-7);
        let bounds = native.bounds().unwrap();
        for i in 0..3 {
            assert!((bounds.min[i] - analytic.bounds_mm[0][i]).abs() < 1e-6);
            assert!((bounds.max[i] - analytic.bounds_mm[1][i]).abs() < 1e-6);
        }
    }
}
