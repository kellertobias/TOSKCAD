use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, Origin, TopologyKind},
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, NormalReliefMotif, PlaneFrame,
    Solid,
    rust_kernel::{Rational, normal_relief::Kind, polygon::PolygonPrism},
};
fn n(n: i64) -> Rational {
    Rational::new(n, 1).unwrap()
}
fn cap(stock: &Solid, face: u32) -> PlaneFrame {
    let topology = stock.topology_snapshot(EntityId::new(), 0).unwrap();
    let Some(AnalyticGeometry::Plane { frame }) = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == face)
        .unwrap()
        .geometry
    else {
        panic!("expected planar cap")
    };
    PlaneFrame {
        origin: frame.origin_mm,
        u: frame.u,
        v: frame.v,
    }
}
fn motif(feature: EntityId, center: [f64; 2]) -> NormalReliefMotif {
    let points = [
        [center[0] - 0.25, center[1] - 0.2],
        [center[0] + 0.25, center[1] - 0.2],
        [center[0] + 0.25, center[1] + 0.2],
        [center[0] - 0.25, center[1] + 0.2],
    ];
    NormalReliefMotif {
        boundary: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % 4],
            })
            .collect(),
        height_mm: 0.25,
        origin: Origin {
            source: feature,
            role: "source-test/motif".into(),
        },
    }
}
fn apply(stock: &Solid, face: u32, frame: PlaneFrame, center: [f64; 2]) -> Solid {
    let feature = EntityId::new();
    stock
        .rust_normal_relief_on_cap(
            face,
            frame,
            &[motif(feature, center)],
            Kind::Raised,
            feature,
            GeometryPolicy::default(),
            None,
        )
        .unwrap()
}
#[test]
fn large_anchor_preserves_retained_fractional_depth_and_rejects_shifted_cap() {
    let policy = GeometryPolicy::default();
    let height = Rational::new(10_000_000_001, 10_000_000_000).unwrap();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [n(12), n(10), height],
        PlaneFrame {
            origin: [100., 200., 9_999_990.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let frame = cap(&stock, 1);
    assert_eq!(frame.origin[2], 9_999_991.);
    assert_eq!(
        stock.rust_prismatic_cap_depth(frame).unwrap(),
        Some(height.as_f64())
    );
    let before = stock.bounds().unwrap();
    let volume = stock.volume_mm3().unwrap();
    let relief = apply(&stock, 1, frame, [3., 3.]);
    assert!((relief.volume_mm3().unwrap() - (volume + 0.05)).abs() < 1e-12);
    assert_eq!(stock.bounds().unwrap(), before);
    let shifted = PlaneFrame {
        origin: [frame.origin[0], frame.origin[1], frame.origin[2].next_up()],
        ..frame
    };
    assert!(stock.rust_prismatic_cap_depth(shifted).unwrap().is_none());
    assert!(
        stock
            .rust_retained_cap_contours(1, shifted)
            .unwrap()
            .is_none()
    );
    let feature = EntityId::new();
    assert_eq!(
        stock
            .rust_normal_relief_on_cap(
                1,
                shifted,
                &[motif(feature, [3., 3.])],
                Kind::Raised,
                feature,
                policy,
                None
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
}
#[test]
fn near_unit_cap_keeps_intrinsic_uv_instead_of_projecting_the_world_twice() {
    let policy = GeometryPolicy::default();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [n(12), n(10), n(3)],
        PlaneFrame {
            u: [(1. + 8e-11_f64).sqrt(), 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let frame = cap(&stock, 1);
    let contours = stock.rust_retained_cap_contours(1, frame).unwrap().unwrap();
    assert_eq!(contours.contours.frame.origin_mm, frame.origin);
    assert_eq!(contours.contours.frame.u, frame.u);
    assert_eq!(contours.contours.frame.v, frame.v);
    let points = contours.contours.loops[0]
        .edges
        .iter()
        .flat_map(|edge| match edge {
            multicad_core::manufacturing::PlanarEdge::Line { a, b } => [*a, *b],
            _ => panic!("box has line edges"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        points
            .iter()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max),
        12.
    );
    assert_eq!(
        points
            .iter()
            .map(|p| p[1])
            .fold(f64::NEG_INFINITY, f64::max),
        10.
    );
    let relief = apply(&stock, 1, frame, [3., 3.]);
    assert!((relief.volume_mm3().unwrap() - (stock.volume_mm3().unwrap() + 0.05)).abs() < 1e-12);
}
#[test]
fn all_six_box_faces_keep_their_retained_column_dimensions() {
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [n(12), n(10), n(3)],
        PlaneFrame {
            origin: [80., 60., 20.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        GeometryPolicy::default(),
    )
    .unwrap();
    for (face, depth) in [3., 3., 10., 12., 10., 12.].into_iter().enumerate() {
        let frame = cap(&stock, face as u32);
        assert_eq!(stock.rust_prismatic_cap_depth(frame).unwrap(), Some(depth));
        let contours = stock
            .rust_retained_cap_contours(face as u32, frame)
            .unwrap()
            .unwrap();
        let points = contours.contours.loops[0]
            .edges
            .iter()
            .flat_map(|edge| match edge {
                multicad_core::manufacturing::PlanarEdge::Line { a, b } => [*a, *b],
                _ => panic!("box has line edges"),
            })
            .collect::<Vec<_>>();
        let center = std::array::from_fn(|i| {
            points
                .iter()
                .map(|p| p[i])
                .fold(f64::INFINITY, f64::min)
                .midpoint(
                    points
                        .iter()
                        .map(|p| p[i])
                        .fold(f64::NEG_INFINITY, f64::max),
                )
        });
        let relief = apply(&stock, face as u32, frame, center);
        assert!((relief.volume_mm3().unwrap() - (360. + 0.05)).abs() < 1e-10);
    }
}
#[test]
fn reverse_normal_and_polygon_sources_keep_both_intrinsic_caps() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let points = [[-5., -4.], [5., -4.], [5., 4.], [-5., 4.]];
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: [70., 30., 10.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % 4],
            })
            .collect(),
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [2., 0.],
            radius_mm: 0.5,
        }]],
    };
    let origins = (0..5)
        .map(|i| Origin {
            source: owner,
            role: format!("curve/{i}"),
        })
        .collect::<Vec<_>>();
    let normal =
        Solid::rust_extrude_normal_profile_named(&profile, -3., owner, &origins, policy).unwrap();
    let polygon = Solid::from_polygon_prism_sweep(
        PolygonPrism::new(
            vec![vec![
                [n(-5), n(-4)],
                [n(5), n(-4)],
                [n(5), n(4)],
                [n(-5), n(4)],
            ]],
            n(3),
        )
        .unwrap(),
        profile.frame,
        true,
        owner,
        &origins[..4],
        &[false; 4],
        policy,
    )
    .unwrap();
    for stock in [&normal, &polygon] {
        for face in 0..2 {
            let frame = cap(stock, face);
            assert_eq!(stock.rust_prismatic_cap_depth(frame).unwrap(), Some(3.));
            let relief = apply(stock, face, frame, [0., 0.]);
            assert!(
                (relief.volume_mm3().unwrap() - (stock.volume_mm3().unwrap() + 0.05)).abs() < 1e-10
            );
        }
    }
}

#[test]
fn cylinder_retains_its_nonzero_rational_bottom_without_top_minus_height() {
    use multicad_kernel::rust_kernel::{Circle, cylinder::AxialCylinder};
    let policy = GeometryPolicy::default();
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([n(7), n(-2)], n(5)).unwrap(),
        Rational::new(3, 10).unwrap(),
        n(1),
    )
    .unwrap();
    let stock = Solid::from_axial_cylinder(
        cylinder,
        PlaneFrame {
            origin: [7., -2., 0.3],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    for face in 0..2 {
        let frame = cap(&stock, face);
        assert_eq!(stock.rust_prismatic_cap_depth(frame).unwrap(), Some(1.));
        let relief = apply(&stock, face, frame, [0., 0.]);
        assert!(
            (relief.volume_mm3().unwrap() - (stock.volume_mm3().unwrap() + 0.05)).abs() < 1e-10
        );
        if face == 1 {
            assert_eq!(
                relief.bounds().unwrap().min[2],
                stock.bounds().unwrap().min[2]
            );
        }
    }
}
