//! ZA worms as certified helical thread shells with an inner bore wall.
use super::*;
use crate::worm_gear::Worm;

/// World frame of the worm's thread coordinates for a placed local frame.
pub(crate) fn thread_frame(worm: &Worm, [origin, u, v]: [Point; 3]) -> [Point; 3] {
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let (s, c) = worm.phase_rad().sin_cos();
    let half = worm.length_mm() / 2.;
    [
        std::array::from_fn(|i| origin[i] - half * n[i]),
        std::array::from_fn(|i| c * u[i] + s * v[i]),
        std::array::from_fn(|i| c * v[i] - s * u[i]),
    ]
}

pub fn write_za_worm(
    worm: &Worm,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_za_worm(worm, origin, u, v, tolerance_mm))
}

/// Whether the placed worm fits the certified helical STEP budget.
pub fn supports_za_worm(worm: &Worm, origin: Point, u: Point, v: Point, tolerance_mm: f64) -> bool {
    let [origin, ..] = thread_frame(worm, [origin, u, v]);
    helical::prepare(
        worm.thread(),
        helical::Closure::hollow(worm.bore_radius_mm()),
        origin,
        tolerance_mm,
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        step_read::verify_za_worm,
        worm_gear::{WormSpec, worm::tests::spec},
    };

    #[test]
    fn worms_with_and_without_bores_round_trip_their_volume() {
        let d = 0.5_f64.sqrt();
        let frame = [[3., -2., 7.], [d, d, 0.], [0., 0., 1.]];
        for spec in [spec(1., 20., 0., 0.02), spec(2., 16., 3., 0.1)] {
            let worm = Worm::new(spec, 1e-3).unwrap();
            let [o, u, v] = frame;
            assert!(supports_za_worm(&worm, o, u, v, 1e-7));
            let text = write_za_worm(&worm, o, u, v, 1e-7).unwrap();
            let volume = verify_za_worm(&text, &worm, o, u, v, 1e-7).unwrap();
            assert!(
                (volume - worm.volume_mm3()).abs() / worm.volume_mm3() < 1e-8,
                "{volume} {}",
                worm.volume_mm3()
            );
            // A different bore is rejected by the readback proof.
            let other = Worm::new(
                WormSpec {
                    bore_radius_mm: spec.bore_radius_mm + 0.5,
                    ..spec
                },
                1e-3,
            )
            .unwrap();
            assert!(verify_za_worm(&text, &other, o, u, v, 1e-7).is_err());
        }
        let worm = Worm::new(spec(1., 20., 1., 0.), 1e-3).unwrap();
        let [o, u, v] = frame;
        assert!(!supports_za_worm(&worm, o, u, v, 0.));
        assert!(write_za_worm(&worm, o, u, v, f64::NAN).is_err());
    }
}
