use super::*;
use crate::spur_gear::Spec;
use std::f64::consts::{FRAC_1_SQRT_2, PI};

const ORIGIN: Point = [11., -7., 23.];
const U: Point = [FRAC_1_SQRT_2, 0., FRAC_1_SQRT_2];
const V: Point = [0., 1., 0.];

fn profile(teeth: u32, bore: f64) -> Profile {
    let pressure = 20_f64.to_radians();
    let pitch = f64::from(teeth);
    Profile::new(
        Spec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * pressure.cos(),
            tip_radius_mm: pitch + 2.,
            root_radius_mm: pitch - 2.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: PI / (2. * pitch) + pressure.tan()
                - pressure
                - 0.05 / (2. * pitch),
            face_width_mm: 8.,
            bore_radius_mm: bore,
        },
        1e-5,
        1e7,
    )
    .unwrap()
}

#[test]
fn spur_export_has_shared_spline_edges_and_exact_analytic_stock() {
    for (teeth, bore) in [(20, 2.), (64, 0.)] {
        let profile = profile(teeth, bore);
        let step = write_spur_gear(&profile, ORIGIN, U, V, SPUR_STEP_MAX_ERROR_MM).unwrap();
        assert_eq!(step.matches("=CLOSED_SHELL(").count(), 1);
        assert_eq!(
            step.matches("=ADVANCED_FACE(").count(),
            2 + profile.outline().len() + usize::from(bore != 0.)
        );
        assert_eq!(
            step.matches("=B_SPLINE_CURVE_WITH_KNOTS(").count(),
            4 * teeth as usize
        );
        assert_eq!(
            step.matches("=SURFACE_OF_LINEAR_EXTRUSION(").count(),
            2 * teeth as usize
        );
        assert_eq!(
            step.matches("=FACE_BOUND(").count(),
            2 * usize::from(bore != 0.)
        );
        assert_eq!(
            step.matches("=CYLINDRICAL_SURFACE(").count(),
            4 * teeth as usize + usize::from(bore != 0.)
        );
        assert!(!step.contains("=POLYLINE("));
        assert_eq!(
            step,
            write_spur_gear(&profile, ORIGIN, U, V, SPUR_STEP_MAX_ERROR_MM * 2.).unwrap()
        );
    }
}

#[test]
fn uncertifiable_spur_exchange_leaves_the_writer_unchanged() {
    let profile = profile(20, 2.);
    let mut writer = Writer::new();
    writer
        .append_spur_gear(&profile, ORIGIN, U, V, SPUR_STEP_MAX_ERROR_MM)
        .unwrap();
    assert_eq!(
        writer.append_spur_gear(&profile, ORIGIN, U, U, SPUR_STEP_MAX_ERROR_MM),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        writer.append_spur_gear(&profile, ORIGIN, U, V, f64::NAN),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        writer.append_spur_gear(&profile, ORIGIN, U, V, f64::from_bits(1)),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        writer.append_spur_gear(&profile, ORIGIN, U, V, 1e-13),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer.finish().unwrap(),
        write_spur_gear(&profile, ORIGIN, U, V, SPUR_STEP_MAX_ERROR_MM).unwrap()
    );
}

#[test]
fn spline_rejects_collapsed_edges_and_uncertifiable_world_rounding() {
    let flank = Involute::new(20., [0.2, 0.5], 0.).unwrap();
    let spline = FlankSpline::new(flank, SPUR_STEP_MAX_ERROR_MM).unwrap();
    let placement = Placement::new([1e16, 0., 0.], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let mut file = StepFile::new();
    let from = file.vertex(placement.point_f64(flank.point(0.), 0.));
    let to = file.vertex(placement.point_f64(flank.point(1.), 0.));
    assert!(matches!(
        spline.encode(&mut file, [from, from], &placement, 0.),
        Err(StepError::InvalidGeometry)
    ));
    assert!(matches!(
        spline.encode(&mut file, [from, to], &placement, 0.),
        Err(StepError::Limit)
    ));
}
