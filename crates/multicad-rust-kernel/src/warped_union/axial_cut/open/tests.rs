use super::*;
use crate::{
    display::Tessellation, manufacturing::certify, planar_boolean::ConvexPiece,
    step::write_warped_union,
};
use std::f64::consts::TAU;

const TOL: f64 = 1e-7;
const SIDE: OpenBoreSide = OpenBoreSide {
    point_mm: [0., 0., 8.],
    normal: [0., 0., 1.],
};

fn stock() -> ConvexPiece {
    ConvexPiece::from_box([0.; 3], [10., 20., 8.], TOL).unwrap()
}

fn shell() -> Shell {
    from_planar_union(&Union::new(&[stock()], TOL).unwrap(), TOL).unwrap()
}

fn bore(z: f64) -> AxialCut {
    AxialCut {
        start_mm: [10.05, 10., z],
        end_mm: [5., 10., z],
        radius_mm: 2.,
        source: 1,
        wall_source_face: 2,
        cap_source_faces: [3, 4],
        recess: None,
    }
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < TOL, "{actual} != {expected}");
}

#[test]
fn both_major_and_minor_segments_preserve_a_watertight_open_notch() {
    let base = shell();
    for (z, reverse) in [(6.5, false), (9.5, true)] {
        let mut tool = bore(z);
        if reverse {
            std::mem::swap(&mut tool.start_mm, &mut tool.end_mm);
        }
        let result = base.cut_open_axial(&tool, SIDE, TOL).unwrap();
        assert_eq!(result.bounds_mm(), base.bounds_mm());
        assert_eq!(result.shell_count(), 1);
        assert_eq!(result.solid_count(), 1);

        let offset = SIDE.point_mm[2] - z;
        let angle = TAU - 2. * (offset / 2.).acos();
        let half_chord = (4. - offset * offset).sqrt();
        let removed_area = 2. * angle + offset * half_chord;
        near(base.volume_mm3() - result.volume_mm3(), 5. * removed_area);
        let surface = |body: &Shell| body.faces().iter().map(|f| f.area_mm2).sum::<f64>();
        near(
            surface(&result) - surface(&base),
            10. * (angle - half_chord),
        );
        let floor = result
            .faces()
            .iter()
            .find(|face| {
                face.source == tool.source && face.source_face == if reverse { 3 } else { 4 }
            })
            .unwrap();
        assert_eq!(
            floor.support,
            Support::Plane {
                normal: [1., 0., 0.]
            }
        );
        near(floor.area_mm2, removed_area);

        let mesh = result
            .mesh(Tessellation {
                deflection: 0.0005,
                angle: 0.1,
                triangle_limit: 5000,
            })
            .unwrap();
        let proof = certify(&mesh).unwrap();
        assert!((proof.signed_volume_mm3 - result.volume_mm3()).abs() < 0.03);
        let step = write_warped_union(&result, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
        assert!(step.contains("=CYLINDRICAL_SURFACE("));
        assert!(step.contains("=CIRCLE("));
    }
}

#[test]
fn rejects_degenerate_tools_and_unproved_or_interrupted_openings() {
    let base = shell();
    let tool = bore(6.5);
    for side in [
        OpenBoreSide {
            point_mm: [f64::NAN, 0., 8.],
            ..SIDE
        },
        OpenBoreSide {
            normal: [0.; 3],
            ..SIDE
        },
    ] {
        assert_eq!(base.cut_open_axial(&tool, side, TOL), Err(Error::Invalid));
    }
    assert_eq!(base.cut_open_axial(&tool, SIDE, 0.), Err(Error::Invalid));
    for side in [
        OpenBoreSide {
            point_mm: [0., 0., 9.],
            ..SIDE
        },
        OpenBoreSide {
            normal: [1., 0., 0.],
            ..SIDE
        },
        OpenBoreSide {
            normal: [0., 0., -1.],
            ..SIDE
        },
    ] {
        assert_eq!(
            base.cut_open_axial(&tool, side, TOL),
            Err(Error::Unsupported)
        );
    }

    for (a, b) in [
        ([10.05, 10., 6.5], [-0.05, 10., 6.5]),
        ([10.05, 0.5, 6.5], [5., 0.5, 6.5]),
        ([9.95, 10., 6.5], [5., 10., 6.5]),
    ] {
        let changed = AxialCut {
            start_mm: a,
            end_mm: b,
            ..tool.clone()
        };
        assert_eq!(
            base.cut_open_axial(&changed, SIDE, TOL),
            Err(Error::Unsupported)
        );
    }
    let divided = [
        ConvexPiece::from_box([0.; 3], [10., 10., 8.], TOL).unwrap(),
        ConvexPiece::from_box([0., 10., 0.], [10., 20., 8.], TOL).unwrap(),
    ];
    let divided = from_planar_union(&Union::new(&divided, TOL).unwrap(), TOL).unwrap();
    assert_eq!(
        divided.cut_open_axial(&tool, SIDE, TOL),
        Err(Error::Unsupported)
    );

    let warped = remap(
        &Union::new(&[stock()], TOL).unwrap(),
        Band {
            center_z_mm: 2.,
            radius_mm: 8.,
        },
        TOL,
    )
    .unwrap();
    assert_eq!(
        warped.cut_open_axial(&tool, SIDE, TOL),
        Err(Error::Unsupported)
    );
}

#[test]
fn a_shallow_segment_still_meshes_at_coarse_tolerance() {
    let base = shell();
    let tool = bore(10. - 2e-7);
    let opened = base.cut_open_axial(&tool, SIDE, TOL).unwrap();
    assert!(opened.volume_mm3() < base.volume_mm3());
    let mesh = opened
        .mesh(Tessellation {
            deflection: 0.1,
            angle: 1.,
            triangle_limit: 1000,
        })
        .unwrap();
    assert!(certify(&mesh).is_ok());
}

#[test]
fn an_open_bore_certifies_each_affected_face_and_the_whole_tool_path() {
    let base = shell();
    let tool = bore(6.5);
    let other = AxialCut {
        start_mm: [-0.05, 16., 4.],
        end_mm: [1., 16., 4.],
        radius_mm: 1.,
        source: 2,
        ..tool.clone()
    };
    let opposite = base.cut_axial(std::slice::from_ref(&other), TOL).unwrap();
    assert!(opposite.cut_open_axial(&tool, SIDE, TOL).is_ok());
    let same = AxialCut {
        start_mm: [10.05, 16., 4.],
        end_mm: [9., 16., 4.],
        ..other
    };
    let same = base.cut_axial(&[same], TOL).unwrap();
    assert_eq!(
        same.cut_open_axial(&tool, SIDE, TOL),
        Err(Error::Unsupported)
    );

    let cavity = ConvexPiece::from_box([6., 9., 5.], [7., 11., 6.], TOL).unwrap();
    let stock_with_cavity = Union::subtract(&[stock()], &[cavity], TOL)
        .unwrap()
        .unwrap();
    let blocked = from_planar_union(&stock_with_cavity, TOL).unwrap();
    assert_eq!(
        blocked.cut_open_axial(&tool, SIDE, TOL),
        Err(Error::Unsupported)
    );

    let mut duplicate = base;
    let roof = duplicate
        .faces
        .iter()
        .find(|face| {
            matches!(
                face.support,
                Support::Plane {
                    normal: [0., 0., 1.]
                }
            )
        })
        .unwrap()
        .clone();
    duplicate.faces.push(roof);
    assert_eq!(
        duplicate.cut_open_axial(&tool, SIDE, TOL),
        Err(Error::Unsupported)
    );
}
