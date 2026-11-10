use super::*;
use crate::{
    helical_tube_cut::{BoundaryEdge, Spec},
    step::helical_trim::TopCavity,
};

const TOLERANCE: f64 = 1e-7;

fn fixture(phase: f64, pitch: f64) -> Cut {
    Cut::new(
        Spec {
            stock_radius_mm: 8.,
            stock_height_mm: 30.,
            tube_radius_mm: 0.6,
            pitch_mm: pitch,
            path_height_mm: 15.,
            start_height_mm: 16.2,
            start_angle_rad: phase,
        },
        TOLERANCE,
    )
    .unwrap()
}

#[test]
fn cavity_cap_keeps_the_complete_parameter_interval_and_rail_endpoints() {
    for (phase, pitch) in [(-TAU, 2.), (-0.1, 1.3), (0., 2.), (0.1, 8.), (TAU, 32.)] {
        let cut = fixture(phase, pitch);
        let surface = CavitySurface::new(cut, TOLERANCE).unwrap();
        let trim = CavityCapPcurve::new(cut, TOLERANCE).unwrap();
        let spline = trim.spline().unwrap();
        let bound = trim.certify(&spline).unwrap();
        assert!(bound > 0. && bound < TOLERANCE / 4.);
        let domain = cut.edge_domain(BoundaryEdge::TopCavity);
        assert_eq!(
            [spline.poles_uv[0], *spline.poles_uv.last().unwrap()],
            domain.map(|u| [u, surface.top_parameter(u).unwrap()])
        );
        let edge = TopCavity::new(cut, TOLERANCE).unwrap();
        let surface_error = surface.certify(&surface.surface().unwrap()).unwrap();
        let edge_error = edge.certify(&edge.spline().unwrap()).unwrap();
        assert!(surface_error + edge_error + bound < TOLERANCE);
        for (i, span) in spline.knots.windows(2).enumerate() {
            let j = i * DEGREE;
            let poles: [[f64; 2]; DEGREE + 1] = spline.poles_uv[j..=j + DEGREE].try_into().unwrap();
            for fraction in 0..=24 {
                let t = fraction as f64 / 24.;
                let alpha = if fraction == 24 {
                    span[1]
                } else {
                    (span[1] - span[0]).mul_add(t, span[0])
                };
                let [u, v] = evaluate(poles, t);
                let [x, y, z] = point(cut.spec(), u, v);
                let exact = cut.edge_point(BoundaryEdge::TopCavity, alpha);
                let error = (x - exact[0]).hypot(y - exact[1]).hypot(z - exact[2]);
                assert!(error <= bound, "{error} exceeds {bound}");
            }
        }
    }
}

fn evaluate(mut poles: [[f64; 2]; DEGREE + 1], t: f64) -> [f64; 2] {
    for end in (1..=DEGREE).rev() {
        for i in 0..end {
            poles[i] = std::array::from_fn(|axis| {
                (poles[i + 1][axis] - poles[i][axis]).mul_add(t, poles[i][axis])
            });
        }
    }
    poles[0]
}

fn point(spec: Spec, alpha: f64, t: f64) -> [f64; 3] {
    let rise = spec.pitch_mm / TAU;
    let length = spec.stock_radius_mm.hypot(rise);
    let radial = spec.stock_radius_mm + spec.tube_radius_mm * alpha.cos();
    let offset = rise / length * spec.tube_radius_mm * alpha.sin();
    let radius = radial.hypot(offset);
    let angle = spec.start_angle_rad + t + offset.atan2(radial);
    [
        radius * angle.cos(),
        radius * angle.sin(),
        spec.start_height_mm + rise * t
            - spec.stock_radius_mm / length * spec.tube_radius_mm * alpha.sin(),
    ]
}

#[test]
fn decoded_control_tampering_cannot_change_the_trim_silently() {
    let cut = fixture(0., 2.);
    assert_eq!(
        CavityCapPcurve::new(cut, 0.).unwrap_err(),
        StepError::InvalidGeometry
    );
    let trim = CavityCapPcurve::new(cut, TOLERANCE).unwrap();
    let original = trim.spline().unwrap();
    let original_bound = trim.certify(&original).unwrap();
    let invalid = Err(StepError::InvalidGeometry);

    let mut changed = original.clone();
    changed.poles_uv[5][1] += TOLERANCE * 0.05 / trim.sweep_speed;
    assert!(trim.certify(&changed).unwrap() > original_bound);
    changed.poles_uv[5][1] += TOLERANCE * 3. / trim.sweep_speed;
    assert_eq!(trim.certify(&changed), invalid);

    let mut changed = original.clone();
    changed.poles_uv[5][0] += TOLERANCE;
    assert_eq!(trim.certify(&changed), invalid);
    let mut changed = original.clone();
    changed.poles_uv[5][1] = changed.poles_uv[4][1] + TOLERANCE;
    assert_eq!(trim.certify(&changed), invalid);
    let mut changed = original.clone();
    changed.poles_uv[0][1] += TOLERANCE;
    assert_eq!(trim.certify(&changed), invalid);
    let mut changed = original.clone();
    changed.poles_uv[5][1] = f64::NAN;
    assert_eq!(trim.certify(&changed), invalid);
    let mut changed = original.clone();
    changed.poles_uv.pop();
    assert_eq!(trim.certify(&changed), invalid);
    let mut changed = original.clone();
    changed.knots[0] += 0.001;
    assert_eq!(trim.certify(&changed), invalid);
    let mut changed = original;
    changed.degree -= 1;
    assert_eq!(trim.certify(&changed), invalid);
}
