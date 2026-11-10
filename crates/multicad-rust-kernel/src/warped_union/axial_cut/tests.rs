use super::*;
use crate::{
    display::Tessellation, manufacturing::certify, planar_boolean::ConvexPiece,
    step::write_warped_union,
};
use std::f64::consts::TAU;

const TOL: f64 = 1e-7;
const BAND: Band = Band {
    center_z_mm: 10.,
    radius_mm: 2.,
};

pub(super) fn attached_stock() -> Shell {
    let stock = ConvexPiece::from_box([0., 0., 3.], [10., 80., 30.], TOL).unwrap();
    let shell = remap(&Union::new(&[stock], TOL).unwrap(), BAND, TOL).unwrap();
    let bosses = [20., 60.].map(|y| Boss {
        start_mm: [9.95, y, 20.],
        end_mm: [13.5, y, 20.],
        radius_mm: 5.5,
        source: 1,
        wall_source_face: 2,
        cap_source_face: 1,
    });
    shell.attach_bosses(&bosses, TOL).unwrap()
}

pub(super) fn rail_cut(y: f64, source: usize) -> AxialCut {
    AxialCut {
        start_mm: [-0.05, y, 20.],
        end_mm: [13.55, y, 20.],
        radius_mm: 1.5,
        source,
        wall_source_face: 9,
        cap_source_faces: [7, 8],
        recess: Some(ConvexRecess {
            rim_mm: (0..6)
                .map(|i| {
                    let (sin, cos) = (TAU * i as f64 / 6.).sin_cos();
                    [11., y + 3. * cos, 20. + 3. * sin]
                })
                .collect(),
            travel_mm: [2.55, 0., 0.],
            source: source + 10,
            wall_source_faces: (10..16).collect(),
            cap_source_faces: [20, 21],
        }),
    }
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < TOL, "{actual} != {expected}");
}

#[test]
fn coaxial_round_and_hex_cuts_preserve_through_paths_shoulders_and_holes() {
    let shell = attached_stock();
    let a = rail_cut(20., 2);
    let mut b = rail_cut(60., 3);
    std::mem::swap(&mut b.start_mm, &mut b.end_mm);
    let b_recess = b.recess.as_mut().unwrap();
    for vertex in &mut b_recess.rim_mm {
        vertex[0] = 13.55;
    }
    b_recess.travel_mm[0] = -2.55;
    let blind = AxialCut {
        start_mm: [5., 40., 20.],
        end_mm: [-0.05, 40., 20.],
        radius_mm: 1.8,
        source: 4,
        wall_source_face: 2,
        cap_source_faces: [31, 32],
        recess: None,
    };
    let cut = shell.cut_axial(&[a, b, blind], TOL).unwrap();
    assert_eq!(cut.bounds_mm(), shell.bounds_mm());
    assert_eq!(cut.solid_count(), 1);
    assert_eq!(cut.shell_count(), 1);
    let hex_area = 27. * 3_f64.sqrt() / 2.;
    let circle_area = PI * 1.5_f64.powi(2);
    near(
        shell.volume_mm3() - cut.volume_mm3(),
        2. * (circle_area * 11. + hex_area * 2.5) + PI * 1.8_f64.powi(2) * 5.,
    );
    let area = |body: &Shell| body.faces().iter().map(|face| face.area_mm2).sum::<f64>();
    near(
        area(&cut) - area(&shell),
        2. * (2. * PI * 1.5 * 11. + 18. * 2.5 - 2. * circle_area) + 2. * PI * 1.8 * 5.,
    );
    let outer = cut
        .faces()
        .iter()
        .find(|face| face.holes.len() == 3)
        .unwrap();
    assert_eq!(outer.source, 0);
    for (source, shoulder_source) in [(12, 20), (13, 21)] {
        let shoulder = cut
            .faces()
            .iter()
            .find(|face| face.source == source && face.source_face == shoulder_source)
            .unwrap();
        assert_eq!(shoulder.holes.len(), 1);
        near(shoulder.area_mm2, hex_area - circle_area);
        assert_eq!(
            shoulder.support,
            Support::Plane {
                normal: [1., 0., 0.]
            }
        );
    }
    let floor = cut
        .faces()
        .iter()
        .find(|f| f.source == 4 && f.source_face == 31)
        .unwrap();
    assert_eq!(
        floor.support,
        Support::Plane {
            normal: [-1., 0., 0.]
        }
    );
    assert_eq!(floor.point_mm, [5., 40., 20.]);
    let options = Tessellation {
        deflection: 0.001,
        angle: 0.1,
        triangle_limit: 5000,
    };
    let certified = certify(&cut.mesh(options).unwrap()).unwrap();
    assert!((certified.signed_volume_mm3 - cut.volume_mm3()).abs() < 0.2);
    let step = write_warped_union(&cut, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    assert!(step.contains("=FACE_BOUND("));
    assert!(step.contains("=CYLINDRICAL_SURFACE("));
}

#[test]
fn malformed_or_unproven_cutters_refuse_the_transaction() {
    let shell = attached_stock();
    let good = rail_cut(20., 2);
    assert_eq!(shell.cut_axial(&[], TOL), Err(Error::Invalid));
    assert_eq!(
        shell.cut_axial(std::slice::from_ref(&good), 0.),
        Err(Error::Invalid)
    );
    assert_eq!(
        shell.cut_axial(&vec![good.clone(); MAX_CUTS + 1], TOL),
        Err(Error::Limit)
    );
    assert_eq!(
        shell.cut_axial(&[good.clone(), good.clone()], TOL),
        Err(Error::Unsupported)
    );
    for (cut, expected) in [
        (
            AxialCut {
                radius_mm: 0.,
                ..good.clone()
            },
            Error::Invalid,
        ),
        (
            AxialCut {
                end_mm: good.start_mm,
                ..good.clone()
            },
            Error::Invalid,
        ),
        (
            AxialCut {
                end_mm: [1e9, 20., 20.],
                ..good.clone()
            },
            Error::Limit,
        ),
        (
            AxialCut {
                start_mm: [0., 20., 20.],
                ..good.clone()
            },
            Error::Unsupported,
        ),
        (
            AxialCut {
                end_mm: [13.5, 20., 20.],
                ..good.clone()
            },
            Error::Unsupported,
        ),
        (
            AxialCut {
                end_mm: [5., 20., 20.],
                ..good.clone()
            },
            Error::Unsupported,
        ),
        (
            AxialCut {
                radius_mm: 5.6,
                ..good.clone()
            },
            Error::Unsupported,
        ),
        (
            AxialCut {
                start_mm: [-3., 20., 20.],
                end_mm: [-2., 20., 20.],
                ..good.clone()
            },
            Error::Unsupported,
        ),
    ] {
        assert_eq!(shell.cut_axial(&[cut], TOL), Err(expected));
    }
    let bad_profile = |change: fn(&mut ConvexRecess)| {
        let mut invalid = good.clone();
        change(invalid.recess.as_mut().unwrap());
        invalid
    };
    for (change, expected) in [
        (
            (|p: &mut ConvexRecess| p.rim_mm.clear()) as fn(&mut ConvexRecess),
            Error::Invalid,
        ),
        (
            |p| p.rim_mm.resize(MAX_RECESS_SIDES + 1, [0.; 3]),
            Error::Limit,
        ),
        (
            |p| {
                p.wall_source_faces.pop();
            },
            Error::Invalid,
        ),
        (|p| p.rim_mm[0][0] += 0.1, Error::Unsupported),
        (|p| p.travel_mm[1] = 0.1, Error::Unsupported),
        (|p| p.travel_mm[0] = 0.5, Error::Unsupported),
        (|p| p.rim_mm[1] = p.rim_mm[0], Error::Unsupported),
        (
            |p| {
                p.rim_mm[0][1] = 21.75;
                p.rim_mm[1][1] = 22.;
                p.rim_mm[5][1] = 22.;
            },
            Error::Unsupported,
        ),
    ] {
        assert_eq!(shell.cut_axial(&[bad_profile(change)], TOL), Err(expected));
    }
}
