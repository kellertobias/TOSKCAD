use super::*;
use crate::{
    Rational,
    normal_relief::{Kind, Relief},
    normal_sweep::{Edge, Sweep},
    step::Writer,
};
const TOL: f64 = 1e-7;
const U: Point = [1., 0., 0.];
const V: Point = [0., 1., 0.];
fn rectangle(min: [f64; 2], max: [f64; 2]) -> Vec<Edge> {
    let p = [min, [max[0], min[1]], max, [min[0], max[1]]];
    (0..4)
        .map(|i| Edge::Line {
            a: p[i],
            b: p[(i + 1) % 4],
        })
        .collect()
}
fn relief(kind: Kind) -> Relief {
    let base = Sweep::new(vec![rectangle([0.; 2], [20., 16.])], 4., TOL, 1e-5).unwrap();
    let motifs = [([3., 3.], [6., 4.], 0.5), ([9., 5.], [12., 7.], 1.25)]
        .into_iter()
        .map(|(min, max, height)| Sweep::new(vec![rectangle(min, max)], height, TOL, 1e-5).unwrap())
        .collect();
    Relief::new(base, motifs, kind, TOL, 1e-5).unwrap()
}
fn expected(relief: &Relief, [origin, u, v]: [Point; 3]) -> ExpectedStepMember<'_> {
    ExpectedStepMember::NormalRelief {
        relief,
        origin,
        u,
        v,
    }
}
#[test]
fn mixed_line_relief_members_use_raw_finite_graph_proof_in_both_kinds() {
    for kind in [Kind::Raised, Kind::Recessed] {
        let relief = relief(kind);
        let frame = [[80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]];
        let mut writer = Writer::new();
        writer
            .append_box([Rational::new(2, 1).unwrap(); 3], [0.; 3], U, V)
            .unwrap();
        writer
            .append_normal_relief(&relief, frame[0], frame[1], frame[2])
            .unwrap();
        let source = writer.finish().unwrap();
        let specification = [ExpectedStepMember::Analytic, expected(&relief, frame)];
        let verified = verify_authored_compound(&source, &specification, TOL).unwrap();
        let [
            VerifiedStepMember::Analytic { readback, .. },
            VerifiedStepMember::NormalRelief { volume_mm3 },
        ] = verified.as_slice()
        else {
            panic!("expected independent stock and relief proofs")
        };
        assert_eq!(readback.volume_mm3, 8.);
        assert!((volume_mm3 - relief.volume_mm3()).abs() < 1e-6);
        let wrong_frame = [[80.001, 60., 20.], frame[1], frame[2]];
        assert!(
            verify_authored_compound(
                &source,
                &[ExpectedStepMember::Analytic, expected(&relief, wrong_frame)],
                TOL
            )
            .is_err()
        );
        assert!(
            verify_authored_compound(
                &source,
                &[expected(&relief, frame), ExpectedStepMember::Analytic],
                TOL
            )
            .is_err()
        );
        assert!(verify_authored_compound(&source, &specification[..1], TOL).is_err());
        assert!(
            verify_authored_compound(
                &source,
                &[
                    ExpectedStepMember::Analytic,
                    expected(&relief, frame),
                    ExpectedStepMember::Analytic
                ],
                TOL
            )
            .is_err()
        );
        let changed = self::relief(if kind == Kind::Raised {
            Kind::Recessed
        } else {
            Kind::Raised
        });
        assert!(
            verify_authored_compound(
                &source,
                &[ExpectedStepMember::Analytic, expected(&changed, frame)],
                TOL
            )
            .is_err()
        );
        assert!(verify_normal_relief(&source, &relief, frame[0], frame[1], frame[2], TOL).is_err());
        assert!(verify_authored_compound(&source, &specification, TOL).is_ok());
    }
}
#[test]
fn source_proof_does_not_require_legacy_import_to_reclassify_rectangular_pockets() {
    let relief = relief(Kind::Recessed);
    let text = crate::step::write_normal_relief(&relief, [0.; 3], U, V).unwrap();
    let (_, decoded) = read_import_solid(&text, TOL).unwrap();
    assert!(matches!(decoded.profile, ImportProfile::Orthogonal { .. }));
    assert!(verify_authored_compound(&text, &[expected(&relief, [[0.; 3], U, V])], TOL).is_ok());
}
