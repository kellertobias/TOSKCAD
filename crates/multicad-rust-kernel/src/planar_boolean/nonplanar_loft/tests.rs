use super::*;

const TOL: f64 = 1e-7;
const RECTANGLE: [[f64; 2]; 4] = [[-2., -1.], [2., -1.], [2., 1.], [-2., 1.]];

fn twisted() -> (NonplanarLoft, f64) {
    let (sin, cos) = std::f64::consts::FRAC_PI_6.sin_cos();
    let top = RECTANGLE.map(|[x, y]| [cos * x - sin * y + 1., sin * x + cos * y + 0.5]);
    let loft = NonplanarLoft::new([&RECTANGLE, &top], 6., TOL).unwrap();
    (loft, cos)
}

#[test]
fn rotated_sections_keep_exact_caps_walls_and_volume() {
    let (loft, cos) = twisted();
    assert!(!loft.clockwise());
    for cap in loft.cap_areas_mm2() {
        assert!((cap - 8.).abs() < 1e-12);
    }
    assert!((loft.volume_mm3() - 6. * 8. * (2. + cos) / 3.).abs() < 1e-12);
    assert_eq!(
        loft.bounds(),
        [
            [-2., -1.366_025_403_784_438_6, 0.],
            [3.232_050_807_568_877, 2.366_025_403_784_438_4, 6.]
        ]
    );
    for (i, wall) in loft.walls().enumerate() {
        let next = (i + 1) % 4;
        for (u, t, expected) in [
            (0., 0., loft.rings()[0][i]),
            (1., 0., loft.rings()[0][next]),
            (0., 1., loft.rings()[1][i]),
            (1., 1., loft.rings()[1][next]),
        ] {
            assert_eq!(wall.point(u, t), expected);
        }
        assert!(wall.area_mm2().unwrap() > 10.);
    }
}

#[test]
fn meshes_are_closed_and_keep_orientation_in_both_authored_windings() {
    let (loft, _) = twisted();
    let options = Tessellation {
        deflection: 0.00005,
        angle: 0.2,
        triangle_limit: 200_000,
    };
    let reversed = loft.rings().each_ref().map(|ring| {
        ring.iter()
            .rev()
            .map(|&[x, y, _]| [x, y])
            .collect::<Vec<_>>()
    });
    let reversed = NonplanarLoft::new([&reversed[0], &reversed[1]], 6., TOL).unwrap();
    assert!(reversed.clockwise());
    for geometry in [loft, reversed] {
        let mesh = geometry.mesh(options).unwrap();
        let proof = manufacturing::certify(&mesh).unwrap();
        assert!(
            (proof.signed_volume_mm3 - geometry.volume_mm3()).abs() < 0.002,
            "{proof:?}"
        );
        assert_eq!(
            mesh.face_indices
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            (0..6).collect()
        );
        assert_eq!(
            geometry.mesh(Tessellation {
                triangle_limit: 1,
                ..options
            }),
            Err(MeshError::BudgetExceeded)
        );
        assert_eq!(
            geometry.mesh(Tessellation {
                deflection: 0.,
                ..options
            }),
            Err(MeshError::InvalidOptions)
        );
    }
    let square = NonplanarLoft::new([&RECTANGLE; 2], 6., TOL).unwrap();
    assert_eq!(square.mesh(options).unwrap().face_indices.len(), 16);
    assert!((square.walls().next().unwrap().area_mm2().unwrap() - 24.).abs() < 1e-12);
}

#[test]
fn area_quadrature_handles_variation_in_both_parameters_and_bounds_work() {
    let loft =
        NonplanarLoft::new([&RECTANGLE, &RECTANGLE.map(|[x, y]| [x, y + 1.])], 1., TOL).unwrap();
    assert!((loft.walls().next().unwrap().area_mm2().unwrap() - 4. * 2_f64.sqrt()).abs() < 1e-12);
    let (loft, _) = twisted();
    let wall = loft.walls().next().unwrap();
    let (edge, rail, twist) = wall.vectors();
    let normal = |u, t| {
        norm(cross(
            add(edge, scale(twist, t)),
            add(rail, scale(twist, u)),
        ))
    };
    // Independently integrate the smooth Jacobian with composite Simpson.
    let quadrature = |f: &dyn Fn(f64) -> f64| {
        (0..=96)
            .map(|i| {
                f(i as f64 / 96.)
                    * if i == 0 || i == 96 {
                        1.
                    } else if i % 2 == 1 {
                        4.
                    } else {
                        2.
                    }
            })
            .sum::<f64>()
            / 288.
    };
    let numerical = quadrature(&|u| quadrature(&|t| normal(u, t)));
    assert!((wall.area_mm2().unwrap() - numerical).abs() < 1e-8);
    assert_eq!(
        area::patch([1., 0., 0.], [0.; 3], [0.; 3], 1., &mut 0),
        Err(Error::Limit)
    );
}

#[test]
fn rejects_collapsed_crossed_or_unbounded_correspondence() {
    let reversed = RECTANGLE.map(|[x, y]| [-x, -y]);
    assert_eq!(
        NonplanarLoft::new([&RECTANGLE, &reversed], 6., TOL).unwrap_err(),
        Error::Unresolved
    );
    let concave = [[0., 0.], [3., 0.], [1., 0.5], [0., 3.]];
    assert_eq!(
        NonplanarLoft::new([&concave; 2], 6., TOL).unwrap_err(),
        Error::Unresolved
    );
    let too_many = [[0.; 2]; MAX_EDGES + 1];
    assert_eq!(
        NonplanarLoft::new([&too_many; 2], 6., TOL).unwrap_err(),
        Error::Limit
    );
    for result in [
        NonplanarLoft::new([&RECTANGLE; 2], TOL, TOL),
        NonplanarLoft::new([&RECTANGLE; 2], f64::INFINITY, TOL),
        NonplanarLoft::new([&RECTANGLE; 2], 6., 0.),
        NonplanarLoft::new([&RECTANGLE[..2]; 2], 6., TOL),
        NonplanarLoft::new([&RECTANGLE[..3], &RECTANGLE], 6., TOL),
        NonplanarLoft::new([&RECTANGLE, &[[f64::NAN, 0.]; 4]], 6., TOL),
    ] {
        assert_eq!(result.unwrap_err(), Error::Invalid);
    }
}
