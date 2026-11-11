use super::*;
use crate::{normal_relief::Kind, normal_sweep::Sweep};
const TOL: f64 = 1e-7;
const MIN: f64 = 1e-5;
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
fn rectangle(min: Point2, max: Point2) -> Vec<ProfileEdge> {
    let p = [min, [max[0], min[1]], max, [min[0], max[1]]];
    (0..4)
        .map(|i| ProfileEdge::Line {
            a: p[i],
            b: p[(i + 1) % 4],
        })
        .collect()
}
fn fixture(kind: Kind) -> Relief {
    let base = Sweep::new(
        vec![
            rectangle([0.; 2], [20., 16.]),
            vec![ProfileEdge::Circle {
                center: [10., 8.],
                radius_mm: 1.,
                clockwise: true,
            }],
        ],
        4.,
        TOL,
        MIN,
    )
    .unwrap();
    let motifs = vec![
        Sweep::new(
            vec![vec![ProfileEdge::Circle {
                center: [3., 3.],
                radius_mm: 0.75,
                clockwise: false,
            }]],
            1.5,
            TOL,
            MIN,
        )
        .unwrap(),
        Sweep::new(
            vec![vec![
                ProfileEdge::Line {
                    a: [12., 2.],
                    b: [14., 2.],
                },
                ProfileEdge::Arc {
                    center: [14., 3.],
                    a: [14., 2.],
                    b: [14., 4.],
                    clockwise: false,
                },
                ProfileEdge::Line {
                    a: [14., 4.],
                    b: [12., 4.],
                },
                ProfileEdge::Line {
                    a: [12., 4.],
                    b: [12., 2.],
                },
            ]],
            0.6,
            TOL,
            MIN,
        )
        .unwrap(),
    ];
    Relief::new(base, motifs, kind, TOL, MIN).unwrap()
}
#[test]
fn full_shared_analytic_relief_roundtrips_without_source_metadata() {
    for kind in [Kind::Raised, Kind::Recessed] {
        let relief = fixture(kind);
        for (origin, u, v) in [
            ([0.; 3], X, Y),
            ([80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
            ([1e6, -2e6, 3e6], X, Y),
        ] {
            let text = crate::step::write_normal_relief(&relief, origin, u, v).unwrap();
            let volume = verify_normal_relief(&text, &relief, origin, u, v, TOL).unwrap();
            assert!((volume - relief.volume_mm3()).abs() < 1e-5);
            let (readback, decoded) = read_import_solid(&text, TOL).unwrap();
            let ImportProfile::NormalRelief {
                relief: decoded_relief,
            } = decoded.profile
            else {
                panic!("expected independently reconstructed relief")
            };
            assert_eq!(decoded_relief.kind(), kind);
            assert_eq!(decoded_relief.motifs().len(), 2);
            assert_eq!(decoded_relief.base().rings().len(), 2);
            assert!((readback.volume_mm3 - relief.volume_mm3()).abs() < 1e-5);
            assert!(
                verify_normal_relief(
                    &text,
                    &decoded_relief,
                    decoded.origin_mm,
                    decoded.u,
                    decoded.v,
                    TOL
                )
                .is_ok()
            );
        }
    }
}
