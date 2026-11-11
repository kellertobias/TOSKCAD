use super::*;
use crate::spur_gear::{
    PI, Spec, TAU,
    internal::{Ring, RingSpec},
};

const TOL: f64 = 1e-7;

fn ring(teeth: u32) -> Ring {
    let a = 20_f64.to_radians();
    let pitch = f64::from(teeth) / 2.;
    Ring::new(
        RingSpec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * a.cos(),
            tip_radius_mm: pitch - 1.,
            root_radius_mm: pitch + 1.25,
            root_fillet_radius_mm: 0.2,
            half_base_angle_rad: (PI / 2. - 0.05) / (2. * pitch) - a.tan() + a,
            face_width_mm: 2.,
            outer_radius_mm: pitch + 4.,
        },
        1e-5,
        1e7,
        TOL,
    )
    .unwrap()
}

fn pinion(bore: f64) -> Profile {
    let a = 20_f64.to_radians();
    let pitch = 12.;
    Profile::new(
        Spec {
            teeth: 24,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * a.cos(),
            tip_radius_mm: pitch + 1.,
            root_radius_mm: pitch - 1.25,
            root_fillet_radius_mm: 0.2,
            half_base_angle_rad: (PI / 2. - 0.05) / (2. * pitch) + a.tan() - a,
            face_width_mm: 2.,
            bore_radius_mm: bore,
        },
        1e-5,
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

/// The canonical ring-first phase: ring at the drive, the pinion at
/// pi / z_pinion plus the same pitch-line travel.
fn pose(teeth: u32, drive: f64, phase_error: f64, center_error: f64) -> [Placement; 2] {
    let center = f64::from(teeth - 24) / 2.;
    [
        place(0., drive),
        place(
            center + center_error,
            PI / 24. + drive * f64::from(teeth) / 24. + phase_error,
        ),
    ]
}

#[test]
fn nested_sampled_cycle_clears_and_wrong_poses_never_clear() {
    for (teeth, bore) in [(48, 0.), (60, 3.)] {
        let ring = ring(teeth);
        let pinion = pinion(bore);
        for i in 0..9 {
            let drive = TAU / f64::from(teeth) * f64::from(i) / 8.;
            assert_eq!(
                certify_internal(&ring, &pinion, pose(teeth, drive, 0., 0.), TOL, None),
                Ok(ResultKind::Clear),
                "{teeth} {i}"
            );
        }
        // Half a pinion pitch puts teeth on teeth at the pitch point.
        assert_eq!(
            certify_internal(&ring, &pinion, pose(teeth, 0., PI / 24., 0.), TOL, None),
            Ok(ResultKind::CommonInterior)
        );
        // The whole pose turned by pi with only the ring off by half its
        // pitch: the collision lies beside the line of centers.
        let center = f64::from(teeth - 24) / 2.;
        let turned = [
            place(0., PI + PI / f64::from(teeth)),
            place(-center, PI + PI / 24.),
        ];
        assert_eq!(
            certify_internal(&ring, &pinion, turned, TOL, None),
            Ok(ResultKind::CommonInterior)
        );
        // Pinion tips pushed into the outward ring roots.
        assert_ne!(
            certify_internal(&ring, &pinion, pose(teeth, 0., 0., 0.4), TOL, None),
            Ok(ResultKind::Clear)
        );
    }
}

#[test]
fn outside_or_enclosing_poses_and_invalid_inputs_are_explicit() {
    let ring = ring(48);
    let pinion = pinion(0.);
    // A pinion outside the rim has disjoint boundaries but is not nested.
    assert_eq!(
        certify_internal(&ring, &pinion, [place(0., 0.), place(60., 0.)], TOL, None),
        Ok(ResultKind::Unresolved)
    );
    for (placements, tolerance) in [
        (pose(48, 0., 0., 0.), 0.),
        (pose(48, 0., 0., 0.), 1e-2),
        ([place(0., 0.), place(0., 0.)], TOL),
        (
            [
                Placement {
                    uncertainty_mm: 1.,
                    ..place(0., 0.)
                },
                place(12., 0.),
            ],
            TOL,
        ),
    ] {
        assert_eq!(
            certify_internal(&ring, &pinion, placements, tolerance, None),
            Err(Error::Invalid)
        );
    }
    let stop = || true;
    assert_eq!(
        certify_internal(&ring, &pinion, pose(48, 0., 0., 0.), TOL, Some(&stop)),
        Err(Error::Cancelled)
    );
}

#[test]
fn grid_pairing_refuses_excess_pair_work() {
    let ring = ring(48);
    let pinion = pinion(0.);
    let [a, b] = pose(48, 0., 0., 0.);
    let cavity = Rim::from_outline(ring.cavity(), 0., a, 1e-3, None).unwrap();
    let teeth = Rim::build(&pinion, b, 1e-3, None).unwrap();
    assert_eq!(
        grid_clear(&cavity, &teeth, 1e-6, 10, None),
        Err(Error::Limit)
    );
    assert_eq!(
        grid_clear(&cavity, &teeth, 1e-6, MAX_PAIR_TESTS, None),
        Ok(true)
    );
    // Teeth on teeth: the sampled boundaries cross near the pitch point.
    let [_, b] = pose(48, 0., PI / 24., 0.);
    let crossing = Rim::build(&pinion, b, 1e-3, None).unwrap();
    assert_eq!(
        grid_clear(&cavity, &crossing, 1e-6, MAX_PAIR_TESTS, None),
        Ok(false)
    );
}
