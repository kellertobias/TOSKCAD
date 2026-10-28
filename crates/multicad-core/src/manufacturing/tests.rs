use super::*;
fn frame() -> crate::references::ReferenceFrame {
    crate::references::ReferenceFrame {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    }
}
#[test]
fn exact_circle_and_inner_winding_keep_analytic_area_without_sampling() {
    let mut outer = PlanarLoop {
        outer: true,
        edges: vec![PlanarEdge::Arc {
            a: [10., 0.],
            b: [10., 0.],
            center: [0., 0.],
            sweep_rad: -std::f64::consts::TAU,
        }],
    };
    outer.normalize_winding();
    let mut inner = PlanarLoop {
        outer: false,
        edges: vec![PlanarEdge::Arc {
            a: [2., 0.],
            b: [2., 0.],
            center: [0., 0.],
            sweep_rad: std::f64::consts::TAU,
        }],
    };
    inner.normalize_winding();
    let contours = PlanarContours {
        frame: frame(),
        loops: vec![outer, inner],
    };
    contours.validate(1e-7).unwrap();
    assert!((contours.area_mm2() - 96. * std::f64::consts::PI).abs() < 1e-9);
    assert_eq!(contours.bounds_mm(), Some([[-10., -10.], [10., 10.]]));
}
#[test]
fn invalid_arc_domains_open_boundaries_and_zero_length_edges_are_rejected() {
    for edge in [
        PlanarEdge::Line {
            a: [0.; 2],
            b: [0.; 2],
        },
        PlanarEdge::Arc {
            a: [1., 0.],
            b: [0., 1.],
            center: [0.; 2],
            sweep_rad: std::f64::consts::PI,
        },
        PlanarEdge::Arc {
            a: [f64::NAN, 0.],
            b: [0., 1.],
            center: [0.; 2],
            sweep_rad: 1.,
        },
    ] {
        assert!(edge.validate(1e-7).is_err());
    }
    let open = PlanarContours {
        frame: frame(),
        loops: vec![PlanarLoop {
            outer: true,
            edges: vec![
                PlanarEdge::Line {
                    a: [0., 0.],
                    b: [10., 0.],
                },
                PlanarEdge::Line {
                    a: [10., 0.],
                    b: [10., 10.],
                },
            ],
        }],
    };
    assert!(open.validate(1e-7).is_err());
    let half = PlanarEdge::Arc {
        a: [0., -2.],
        b: [0., 2.],
        center: [0.; 2],
        sweep_rad: std::f64::consts::PI,
    };
    half.validate(1e-7).unwrap();
    assert_eq!(half.bounds(), [[0., -2.], [2., 2.]]);
}
