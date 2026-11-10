use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    references::Origin,
};
use multicad_kernel::{
    CircularPocketTool, DrawingProjection, DrawingSolid, GeometryErrorKind, GeometryPolicy,
    PlaneFrame, Solid, project_drawing,
};
use multicad_rust_kernel::Rational;

fn block(origin: [f64; 3], size: [i64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|v| Rational::new(v, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}

fn stock(front_pocket: bool) -> Solid {
    let mut tools = vec![CircularPocketTool {
        frame: PlaneFrame {
            origin: [0., 0., 15.],
            ..Default::default()
        },
        center_uv: [10., 10.],
        radius_mm: 2.,
        displacement_mm: [0., 0., -7.],
        origin: Origin {
            source: EntityId::new(),
            role: "dog hole".into(),
        },
    }];
    if front_pocket {
        tools.push(CircularPocketTool {
            center_uv: [5., 5.],
            radius_mm: 1.,
            displacement_mm: [0., 0., -2.],
            origin: Origin {
                source: EntityId::new(),
                role: "front blind bore".into(),
            },
            ..tools[0].clone()
        });
    }
    block([0., 0., 10.], [20, 20, 4])
        .rust_drill_circular_machining(
            &tools,
            if front_pocket {
                &[true, false]
            } else {
                &[true]
            },
            GeometryPolicy::default(),
        )
        .unwrap()
}

fn drawing(
    solids: &[&Solid],
    frame: PlaneFrame,
) -> Result<Vec<multicad_kernel::ProjectedVector>, multicad_kernel::GeometryError> {
    let inputs: Vec<_> = solids
        .iter()
        .map(|&solid| DrawingSolid { solid, hatch: None })
        .collect();
    project_drawing(
        &inputs,
        DrawingProjection {
            frame,
            ..Default::default()
        },
    )
}

#[test]
fn aperture_rim_is_visible_only_when_not_occluded_in_front() {
    let plate = stock(false);
    let far_clear = block([4., 4., 0.], [3; 3]);
    let visible = drawing(&[&plate, &far_clear], PlaneFrame::default()).unwrap();
    assert_eq!(visible.len(), 5);
    assert!(visible.iter().any(|edge| matches!(
        edge.curve,
        VectorCurve::Ellipse {
            center: [10., 10.],
            major_mm: 2.,
            minor_mm: 2.,
            ..
        }
    ) && edge.source_index == 0
        && edge.role == VectorRole::Visible));

    let in_front = block([7., 7., 20.], [6, 6, 3]);
    let occluded = drawing(&[&plate, &in_front], PlaneFrame::default()).unwrap();
    assert_eq!(occluded.len(), 8);
    assert!(
        occluded
            .iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
    );
}

#[test]
fn projection_refuses_potential_visibility_behind_the_hole() {
    let plate = stock(false);
    let through_ray = block([9., 9., 0.], [2; 3]);
    let touching = block([12., 9., 0.], [2; 3]);
    for other in [&through_ray, &touching] {
        let error = drawing(&[&plate, other], PlaneFrame::default()).unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("overlaps the aperture"));
    }

    let side_view = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
    };
    let side = drawing(&[&plate, &through_ray], side_view).unwrap();
    assert!(
        side.iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
    );

    let a = std::f64::consts::FRAC_1_SQRT_2;
    let oblique = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., a, a],
    };
    let error = drawing(&[&plate], oblique).unwrap_err();
    assert!(error.message.contains("axial or edge-on"));

    let front_pocket = stock(true);
    let error = drawing(&[&front_pocket], PlaneFrame::default()).unwrap_err();
    assert!(error.message.contains("blind pockets"));
}
