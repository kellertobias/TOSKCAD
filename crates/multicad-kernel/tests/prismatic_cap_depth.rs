use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    CircularPocketTool, GeometryPolicy, PlaneFrame, Solid,
    rust_kernel::{Rational, polygon::PolygonPrism},
};

fn integer(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

#[test]
fn exact_cap_support_does_not_extend_to_shifted_or_side_planes() {
    let policy = GeometryPolicy::default();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [10, 8, 3].map(integer),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let top = PlaneFrame {
        origin: [0., 0., 3.],
        ..Default::default()
    };
    assert_eq!(stock.rust_prismatic_cap_depth(top).unwrap(), Some(3.));
    let bottom = PlaneFrame {
        v: [0., -1., 0.],
        ..Default::default()
    };
    assert_eq!(stock.rust_prismatic_cap_depth(bottom).unwrap(), Some(3.));
    for frame in [
        PlaneFrame {
            origin: [0., 0., 3. + 1e-11],
            ..top
        },
        PlaneFrame {
            origin: [0., 0., 3. - 1e-11],
            ..top
        },
        PlaneFrame {
            u: [0., 1., 0.],
            v: [0., 0., 1.],
            ..Default::default()
        },
    ] {
        assert_eq!(stock.rust_prismatic_cap_depth(frame).unwrap(), None);
    }
    assert!(
        stock
            .rust_prismatic_cap_depth(PlaneFrame { u: [0.; 3], ..top })
            .is_err()
    );
}

#[test]
fn polygon_machining_cannot_claim_an_unchanged_full_column() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let polygon = PolygonPrism::new(
        vec![
            vec![[0, 0], [10, 0], [10, 8], [0, 8]]
                .into_iter()
                .map(|p| p.map(integer))
                .collect(),
        ],
        integer(3),
    )
    .unwrap();
    let origins = (0..4)
        .map(|edge| Origin {
            source: owner,
            role: format!("edge/{edge}"),
        })
        .collect::<Vec<_>>();
    let stock = Solid::from_polygon_prism(
        polygon,
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 4],
        policy,
    )
    .unwrap();
    let top = PlaneFrame {
        origin: [0., 0., 3.],
        ..Default::default()
    };
    assert_eq!(stock.rust_prismatic_cap_depth(top).unwrap(), Some(3.));
    let cut = stock
        .rust_drill_circular_pockets(
            &[CircularPocketTool {
                frame: PlaneFrame {
                    origin: [0., 0., 4.],
                    ..Default::default()
                },
                center_uv: [5., 4.],
                radius_mm: 1.,
                displacement_mm: [0., 0., -2.],
                origin: Origin {
                    source: EntityId::new(),
                    role: "pocket".into(),
                },
            }],
            policy,
        )
        .unwrap();
    assert_eq!(cut.rust_prismatic_cap_depth(top).unwrap(), None);
    assert!((cut.volume_mm3().unwrap() - (240. - std::f64::consts::PI)).abs() < 1e-8);
    assert_eq!(stock.volume_mm3().unwrap(), 240.);
}
