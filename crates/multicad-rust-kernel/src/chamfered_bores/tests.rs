use super::*;
use crate::{
    Circle,
    display::{MeshError, Tessellation, chamfered_polygon_prism},
    manufacturing::certify,
};

const CLEARANCE: f64 = 1e-7;
const MINIMUM: f64 = 1e-5;
const OPTIONS: Tessellation = Tessellation {
    deflection: 0.001,
    angle: 0.14,
    triangle_limit: 20_000,
};

fn value(n: i64) -> Rational {
    Rational::new(n, 1).unwrap()
}

fn prism(height: i64) -> PolygonPrism {
    let [outer, hole] = [
        [[0, 0], [20, 0], [20, 16], [0, 16]],
        [[14, 6], [14, 10], [17, 10], [17, 6]],
    ]
    .map(|ring| ring.into_iter().map(|point| point.map(value)).collect());
    let bores = [[4, 8], [9, 8], [3, 3], [18, 13]]
        .map(|center| Circle::new(center.map(value), value(1)).unwrap());
    PolygonPrism::with_circular_holes(vec![outer, hole], bores.into(), value(height)).unwrap()
}

fn chamfer(hole: usize, end: bool, size_mm: f64) -> BoreChamfer {
    BoreChamfer { hole, end, size_mm }
}

fn fit(prism: &PolygonPrism, cuts: &[BoreChamfer]) -> Result<ChamferedBores, PocketError> {
    ChamferedBores::new(prism, cuts, CLEARANCE, MINIMUM)
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

#[test]
fn opposite_chamfers_keep_exact_cones_and_original_bore_faces() {
    let stock = prism(6);
    let cuts = [
        chamfer(1, true, 2.),
        chamfer(0, false, 2.),
        chamfer(0, true, 0.5),
    ];
    let bores = fit(&stock, &cuts).unwrap();
    assert_eq!(bores.chamfers(), &[cuts[1], cuts[2], cuts[0]]);
    let [first, second, plain, other_plain] = bores.stacks() else {
        panic!("Expected four bores");
    };
    let dimensions = |stack: &BoreStack| {
        stack
            .layers
            .iter()
            .map(|layer| [layer.z_mm, layer.radius_mm])
            .collect::<Vec<_>>()
    };
    assert_eq!(first.center_mm, [4., 8.]);
    assert_eq!(
        dimensions(first),
        [[0., 3.], [2., 1.], [5.5, 1.], [6., 1.5]]
    );
    assert_eq!(dimensions(second), [[0., 1.], [4., 1.], [6., 3.]]);
    assert_eq!(dimensions(plain), [[0., 1.], [6., 1.]]);
    assert_eq!(first.face_indices, [14, 10, 15]);
    assert_eq!(second.face_indices, [11, 16]);
    assert_eq!(plain.face_indices, [12]);
    assert_eq!(other_plain.face_indices, [13]);

    let delta = cuts
        .iter()
        .map(|cut| {
            let d = cut.size_mm;
            PI * ((1. + d).powi(3) - 1.) / 3. - PI * d
        })
        .sum::<f64>();
    near(bores.volume_mm3(), stock.volume().as_f64() - delta, 1e-10);
    let (area, center) = bores.cap_properties(false);
    let removed_area = 8. * PI;
    near(area, stock.area().as_f64() - removed_area, 1e-10);
    for (i, coordinate) in center.into_iter().enumerate() {
        near(
            coordinate,
            (stock.centroid()[i] * stock.area().as_f64() - first.center_mm[i] * removed_area)
                / area,
            1e-10,
        );
    }
    near(
        bores.cap_properties(true).0,
        stock.area().as_f64() - 9.25 * PI,
        1e-10,
    );

    let mesh = chamfered_polygon_prism(&stock, &bores, OPTIONS).unwrap();
    let proof = certify(&mesh).unwrap();
    assert_eq!(mesh.face_indices.iter().copied().max(), Some(16));
    near(proof.signed_volume_mm3, bores.volume_mm3(), 0.1);
    assert!(proof.signed_volume_mm3 >= bores.volume_mm3());
    assert_eq!(
        mesh.face_indices.iter().filter(|&&face| face == 14).count(),
        mesh.face_indices.iter().filter(|&&face| face == 15).count()
    );
}

#[test]
fn invalid_dimensions_contacts_and_disappearing_cylinders_are_rejected() {
    use PocketError::{BudgetExceeded, InvalidInput, UnresolvedBoundary};
    let stock = prism(6);
    assert_eq!(fit(&stock, &[]), Err(InvalidInput));
    assert_eq!(
        ChamferedBores::new(&stock, &[chamfer(0, false, 1.)], f64::NAN, MINIMUM),
        Err(InvalidInput)
    );
    assert_eq!(
        ChamferedBores::new(&stock, &[chamfer(0, false, 1.)], CLEARANCE, 0.),
        Err(InvalidInput)
    );
    assert_eq!(
        fit(&stock, &[chamfer(0, false, 1.); 9]),
        Err(BudgetExceeded)
    );
    for cuts in [
        vec![chamfer(4, false, 1.)],
        vec![chamfer(0, false, 1.); 2],
        vec![chamfer(0, false, 0.)],
        vec![chamfer(0, false, f64::INFINITY)],
    ] {
        assert_eq!(fit(&stock, &cuts), Err(InvalidInput));
    }
    for cuts in [
        vec![chamfer(0, false, 3.), chamfer(0, true, 3.)],
        vec![chamfer(0, false, 2.), chamfer(1, false, 2.)],
        vec![chamfer(2, false, 2.)], // Outside straight cap boundary.
        vec![chamfer(1, true, 4.)],  // Touches rectangular inner ring.
    ] {
        assert_eq!(fit(&stock, &cuts), Err(UnresolvedBoundary));
    }
}

#[test]
fn tessellation_checks_stock_identity_and_budgets() {
    let stock = prism(6);
    let bores = fit(&stock, &[chamfer(1, true, 1.)]).unwrap();
    assert_eq!(
        chamfered_polygon_prism(&prism(7), &bores, OPTIONS),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(
        chamfered_polygon_prism(
            &stock,
            &bores,
            Tessellation {
                angle: 0.,
                ..OPTIONS
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        chamfered_polygon_prism(
            &stock,
            &bores,
            Tessellation {
                triangle_limit: 200,
                ..OPTIONS
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
}
