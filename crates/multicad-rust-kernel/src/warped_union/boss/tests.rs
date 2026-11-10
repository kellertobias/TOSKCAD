use super::*;
use crate::{display::Tessellation, manufacturing::certify, planar_boolean::ConvexPiece};

const TOL: f64 = 1e-7;
const BAND: Band = Band {
    center_z_mm: 10.,
    radius_mm: 2.,
};

fn stock() -> ConvexPiece {
    ConvexPiece::from_box([0., 0., 3.], [10., 80., 30.], TOL).unwrap()
}

fn boss(y: f64, radius_mm: f64, source: usize) -> Boss {
    Boss {
        start_mm: [9.95, y, 20.],
        end_mm: [13.5, y, 20.],
        radius_mm,
        source,
        wall_source_face: 2,
        cap_source_face: 1,
    }
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < TOL, "{actual} != {expected}");
}

#[test]
fn two_round_bosses_share_one_planar_host_without_losing_area_or_provenance() {
    let old = remap(&Union::new(&[stock()], TOL).unwrap(), BAND, TOL).unwrap();
    let tools = [boss(20., 2., 10), boss(60., 3., 11)];
    let joined = old.attach_bosses(&tools, TOL).unwrap();
    let host = joined
        .faces()
        .iter()
        .find(|face| face.holes.len() == 2)
        .unwrap();
    assert_eq!(host.source, 0);
    assert_eq!(joined.faces().len(), old.faces().len() + 10);
    assert_eq!(joined.solid_count(), 1);
    assert_eq!(joined.shell_count(), 1);
    assert_eq!(joined.bounds_mm(), [[0., 0., 11.], [13.5, 80., 38.]]);
    near(joined.volume_mm3() - old.volume_mm3(), PI * 13. * 3.5);
    let area = |shell: &Shell| shell.faces().iter().map(|f| f.area_mm2).sum::<f64>();
    near(area(&joined) - area(&old), 2. * PI * 5. * 3.5);
    for tool in tools {
        let created: Vec<_> = joined
            .faces()
            .iter()
            .filter(|face| face.source == tool.source)
            .collect();
        assert_eq!(created.len(), 5);
        assert_eq!(
            created
                .iter()
                .filter(|f| f.source_face == tool.wall_source_face)
                .count(),
            4
        );
        assert_eq!(created[4].source_face, tool.cap_source_face);
    }
    let options = Tessellation {
        deflection: 0.001,
        angle: 0.1,
        triangle_limit: 2000,
    };
    let proof = certify(&joined.mesh(options).unwrap()).unwrap();
    let loss = joined.volume_mm3() - proof.signed_volume_mm3;
    assert!(loss > 0. && loss < 2. * PI * 5. * 3.5 * options.deflection);
    let again = old
        .attach_bosses(&tools[..1], TOL)
        .unwrap()
        .attach_bosses(&tools[1..], TOL)
        .unwrap();
    near(again.volume_mm3(), joined.volume_mm3());
    assert!(certify(&again.mesh(options).unwrap()).is_ok());
}

#[test]
fn any_unproven_sweep_or_host_rejects_the_entire_batch() {
    let old = remap(&Union::new(&[stock()], TOL).unwrap(), BAND, TOL).unwrap();
    let good = boss(20., 2., 10);
    let bad = [
        (
            Boss {
                start_mm: good.end_mm,
                ..good
            },
            Error::Invalid,
        ),
        (
            Boss {
                start_mm: [10.1, 20., 20.],
                ..good
            },
            Error::Unsupported,
        ),
        (
            Boss {
                radius_mm: 12.,
                ..good
            },
            Error::Unsupported,
        ),
        (
            Boss {
                end_mm: [13.5, 22., 20.],
                ..good
            },
            Error::Unsupported,
        ),
        (
            Boss {
                end_mm: [f64::INFINITY, 20., 20.],
                ..good
            },
            Error::Invalid,
        ),
        (
            Boss {
                radius_mm: 1e9,
                ..good
            },
            Error::Limit,
        ),
        (
            Boss {
                radius_mm: 0.,
                ..good
            },
            Error::Invalid,
        ),
    ];
    for (tool, expected) in bad {
        assert_eq!(old.attach_bosses(&[tool], TOL).unwrap_err(), expected);
    }
    assert_eq!(old.attach_bosses(&[], TOL).unwrap_err(), Error::Invalid);
    assert_eq!(
        old.attach_bosses(&[good; MAX_BOSSES + 1], TOL).unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        old.attach_bosses(&[good, boss(21., 2., 11)], TOL)
            .unwrap_err(),
        Error::Unsupported
    );
    let one = old.attach_bosses(&[good], TOL).unwrap();
    assert_eq!(
        one.attach_bosses(&[good], TOL).unwrap_err(),
        Error::Unsupported
    );

    let obstacle = ConvexPiece::from_box([12., 18., 10.], [14., 22., 16.], TOL).unwrap();
    let blocked = remap(&Union::new(&[stock(), obstacle], TOL).unwrap(), BAND, TOL).unwrap();
    assert_eq!(
        blocked.attach_bosses(&[good], TOL).unwrap_err(),
        Error::Unsupported
    );
    let across = Boss {
        end_mm: [17., 20., 20.],
        radius_mm: 1.,
        ..good
    };
    assert_eq!(
        blocked.attach_bosses(&[across], TOL).unwrap_err(),
        Error::Unsupported
    );
}

#[test]
fn covered_face_representatives_and_later_sweeps_keep_certified_material() {
    let old = remap(&Union::new(&[stock()], TOL).unwrap(), BAND, TOL).unwrap();
    let tool = Boss {
        start_mm: [9.95, 40., 24.5],
        end_mm: [13.5, 40., 24.5],
        ..boss(40., 2., 10)
    };
    let mut joined = old.attach_bosses(&[tool], TOL).unwrap();
    let host = joined
        .faces()
        .iter()
        .position(|face| !face.holes.is_empty())
        .unwrap();
    assert_eq!(old.faces()[host].point_mm, [10., 40., 24.5]);
    assert!(length3(sub(joined.faces()[host].point_mm, [10., 40., 24.5])) > tool.radius_mm);

    let far = Attachment {
        tool: Boss {
            start_mm: [15., 40., 24.5],
            end_mm: [18., 40., 24.5],
            ..tool
        },
        host,
        mouth: [16., 40., 24.5],
        axes: [[0., 1., 0.], [0., 0., 1.], [1., 0., 0.]],
        height: 2.,
    };
    let cylinder = joined
        .faces()
        .iter()
        .find(|face| matches!(face.support, Support::Cylinder { .. }))
        .unwrap();
    assert!(far.clears_face(&joined, cylinder, TOL));

    let rim = joined.faces()[host].holes[0][0].edge;
    joined.edges[rim].curve = Curve::Line;
    assert_eq!(
        joined.attach_bosses(&[boss(65., 2., 11)], TOL).unwrap_err(),
        Error::Unsupported
    );
}
