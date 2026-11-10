use super::*;
use crate::spur_gear::Spec;
use std::f64::consts::{PI, TAU};

const TOL: f64 = 1e-7;

fn gear(teeth: u32) -> Profile {
    let pitch = f64::from(teeth);
    let pressure = 20_f64.to_radians();
    Profile::new(
        Spec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * pressure.cos(),
            tip_radius_mm: pitch + 2.,
            root_radius_mm: pitch - 2.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: (PI - 0.05) / (2. * pitch) + pressure.tan() - pressure,
            face_width_mm: 5.,
            bore_radius_mm: 4.,
        },
        1e-6,
        1e7,
    )
    .unwrap()
}

fn place(x: f64, theta: f64) -> Placement {
    let (sin, cos) = theta.sin_cos();
    Placement {
        center_mm: [x, 0.],
        axes: [[cos, sin], [-sin, cos]],
        ..Placement::default()
    }
}

#[test]
fn nominal_sampled_pair_clears_and_opposed_teeth_share_material() {
    let a = gear(24);
    let b = gear(36);
    for i in 0..9 {
        let drive = TAU / 24. * f64::from(i) / 8.;
        let result = certify(
            [&a, &b],
            [
                place(0., drive),
                place(60., PI + PI / 36. - drive * 24. / 36.),
            ],
            TOL,
            None,
        );
        assert_eq!(result, Ok(ResultKind::Clear), "{i}");
    }
    assert_eq!(
        certify([&a, &b], [place(0., 0.), place(60., PI)], TOL, None),
        Ok(ResultKind::CommonInterior)
    );
}

#[test]
fn touching_and_nesting_do_not_count_as_positive_clearance() {
    let a = gear(24);
    let b = gear(36);
    for (x, expected) in [
        (64., ResultKind::Unresolved),
        (70., ResultKind::Clear),
        (1., ResultKind::CommonInterior),
    ] {
        assert_eq!(
            certify([&a, &b], [place(0., 0.), place(x, 0.)], TOL, None),
            Ok(expected)
        );
    }
}

#[test]
fn placement_cancellation_and_enclosure_budgets_fail_closed() {
    use std::cell::Cell;
    let a = gear(24);
    let b = gear(36);
    let placed = [place(0., 0.), place(60., 0.)];
    assert_eq!(
        certify([&a, &b], placed, f64::NAN, None),
        Err(Error::Invalid)
    );
    let mut reflected = placed;
    reflected[1].axes[0] = [-1., 0.];
    assert_eq!(certify([&a, &b], reflected, TOL, None), Err(Error::Invalid));
    assert_eq!(
        certify([&a, &b], [place(0., 0.); 2], TOL, None),
        Err(Error::Invalid)
    );
    let count = Cell::new(0);
    let stop = || {
        count.set(count.get() + 1);
        count.get() == 3
    };
    assert_eq!(
        certify([&a, &b], placed, TOL, Some(&stop)),
        Err(Error::Cancelled)
    );

    assert!(matches!(
        Rim::build(&a, place(1e7, 0.), TOL, None),
        Err(Error::Limit)
    ));
    let curve = a
        .outline
        .iter()
        .find(|edge| matches!(edge.edge, Edge::Involute(_)))
        .unwrap()
        .edge;
    assert_eq!(segment_count(curve, f64::MIN_POSITIVE), Err(Error::Limit));
    let mut malformed = a.clone();
    malformed
        .outline
        .resize(MAX_CHORDS + 1, malformed.outline[0]);
    assert!(matches!(
        Rim::build(&malformed, placed[0], 0.01, None),
        Err(Error::Limit)
    ));
}

#[test]
fn discontinuities_bores_and_pair_work_cannot_supply_a_false_proof() {
    let mut a = gear(24);
    let b = gear(36);
    let first = Rim::build(&a, place(0., 0.), 0.01, None).unwrap();
    let second = Rim::build(&b, place(60., PI / 36.), 0.01, None).unwrap();
    assert!(!first.contains_strictly([0., 0.]));
    assert_eq!(
        boundaries_clear(&first, &second, 0, None),
        Err(Error::Limit)
    );

    a.outline[0].edge = Edge::Planar(normal_sweep::Edge::Line {
        a: [10., 10.],
        b: [11., 10.],
    });
    assert!(matches!(
        Rim::build(&a, place(0., 0.), 0.01, None),
        Err(Error::Limit)
    ));
}
