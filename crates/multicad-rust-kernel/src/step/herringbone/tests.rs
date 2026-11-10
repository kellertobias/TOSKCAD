use super::*;

#[test]
fn refuses_unsafe_rotation_and_trim_inputs() {
    use crate::spur_gear::Involute;
    let curve = Edge::Involute(Involute::new(10., [0., 1.], 0.).unwrap());
    let ends = [curve.point(0.), curve.point(1.)];
    assert!(matches!(
        Chart::transverse(curve, ends, 1e-30),
        Err(StepError::Limit)
    ));
    assert!(matches!(
        Chart::transverse(curve, ends, 0.),
        Err(StepError::InvalidGeometry)
    ));
    let circle = Edge::Planar(PlanarEdge::Circle {
        center: [0.; 2],
        radius_mm: 2.,
        clockwise: false,
    });
    assert!(matches!(
        Chart::transverse(circle, [[0.; 2]; 2], 1e-7),
        Err(StepError::Unsupported)
    ));
    assert!(matches!(
        Chart::transverse(curve, [[0.; 2]; 2], 1e-7),
        Err(StepError::Limit)
    ));
    assert!(matches!(
        axial_rotation(1e-18, 1., 1e-8),
        Err(StepError::Limit)
    ));
}

#[test]
fn bounded_writer_keeps_one_shell_and_shared_three_station_boundaries() {
    use crate::spur_gear::Spec;
    let beta = 15_f64.to_radians();
    let pressure = (20_f64.to_radians().tan() / beta.cos()).atan();
    let module = 2. / beta.cos();
    let pitch = 12. * module;
    let spec = Spec {
        teeth: 24,
        pitch_radius_mm: pitch,
        base_radius_mm: pitch * pressure.cos(),
        tip_radius_mm: pitch + 2.,
        root_radius_mm: pitch - 2.5,
        root_fillet_radius_mm: 0.4,
        half_base_angle_rad: (std::f64::consts::PI * module / 2. - 0.05) / (2. * pitch)
            + pressure.tan()
            - pressure,
        face_width_mm: 4.,
        bore_radius_mm: 4.,
    };
    let gear = Herringbone::new(spec, beta.tan() / pitch, 1e-5, 1e7).unwrap();
    let many = Herringbone::new(
        Spec {
            teeth: 48,
            pitch_radius_mm: 2. * spec.pitch_radius_mm,
            base_radius_mm: 2. * spec.base_radius_mm,
            tip_radius_mm: 2. * spec.pitch_radius_mm + 2.,
            root_radius_mm: 2. * spec.pitch_radius_mm - 2.5,
            half_base_angle_rad: (std::f64::consts::PI * module / 2. - 0.05) / (4. * pitch)
                + pressure.tan()
                - pressure,
            ..spec
        },
        beta.tan() / (2. * pitch),
        1e-5,
        1e7,
    )
    .unwrap();
    assert!(matches!(
        prepare(&many, [0.; 3], 1e-9),
        Err(StepError::Limit)
    ));
    let placement = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let mut file = StepFile::new();
    file.next = MAX_STEP_ENTITIES;
    assert!(matches!(
        encode(&mut file, &gear, placement, 1e-7),
        Err(StepError::Limit)
    ));
    let count = gear.profile().outline().len();
    let write = |origin, tol| write_herringbone(&gear, origin, [1., 0., 0.], [0., 1., 0.], tol);
    assert!(supports_herringbone(
        &gear,
        [0.; 3],
        [1., 0., 0.],
        [0., 1., 0.],
        1e-7
    ));
    assert!(!supports_herringbone(
        &gear,
        [0.; 3],
        [1., 0., 0.],
        [1., 0., 0.],
        1e-7
    ));
    assert_eq!(write([0.; 3], 0.), Err(StepError::InvalidGeometry));
    assert_eq!(write([1e8; 3], 1e-7), Err(StepError::Limit));
    let text = write([0.; 3], 1e-7).unwrap();
    assert_eq!(text.matches("=MANIFOLD_SOLID_BREP(").count(), 1);
    assert_eq!(text.matches("=CLOSED_SHELL(").count(), 1);
    assert_eq!(text.matches("ADVANCED_FACE(").count(), 2 * count + 3);
    assert_eq!(text.matches("EDGE_CURVE(").count(), 5 * count + 3);
    assert_eq!(
        text.matches("B_SPLINE_SURFACE_WITH_KNOTS(").count(),
        2 * count
    );
    assert!(text.contains("RATIONAL_B_SPLINE_SURFACE"));
    assert!(text.len() < crate::step_read::MAX_PLANAR_STEP_BYTES);
}

#[test]
fn axial_rotation_preserves_the_apex_and_certifies_either_hand() {
    let eval = |mut poles: Vec<[f64; 2]>, t: f64| {
        for degree in (1..poles.len()).rev() {
            for i in 0..degree {
                poles[i] = std::array::from_fn(|c| poles[i][c] * (1. - t) + poles[i + 1][c] * t);
            }
        }
        poles[0]
    };
    for angle in [-0.02_f64, 0.02, -1.4, 1.4] {
        let (axial, z) = axial_rotation(angle, 4., 1e-10).unwrap();
        let reverse = axial.clone().reversed();
        assert_eq!(axial.poles.last(), reverse.poles.first());
        assert_eq!(axial.poles.first(), reverse.poles.last());
        assert_eq!(z.first(), Some(&0.));
        assert_eq!(z.last(), Some(&4.));
        assert!(z.windows(2).all(|w| w[0] < w[1]));
        let mut previous = -angle.signum();
        for i in 0..=100 {
            let t = f64::from(i) / 100.;
            let u = t * angle.abs();
            let index = axial
                .knots
                .partition_point(|x| *x <= u)
                .saturating_sub(1)
                .min(axial.knots.len() - 2);
            let [a, b] = [axial.knots[index], axial.knots[index + 1]];
            let start = index * axial.degree;
            let [x, y] = eval(
                axial.poles[start..=start + axial.degree].to_vec(),
                (u - a) / (b - a),
            );
            let signed = angle * t;
            assert!((x - signed.cos()).hypot(y - signed.sin()) < 1e-10);
            let actual = y.atan2(x);
            assert!(angle.signum() * (actual - previous) > 0.);
            previous = actual;
        }
    }
}

#[test]
fn exact_arc_chart_has_positive_weights_and_meets_the_authored_radius() {
    let a = [4., 2.];
    let b = [2., 4.];
    let center = [2., 2.];
    let chart = Chart::transverse(
        Edge::Planar(PlanarEdge::Arc {
            center,
            a,
            b,
            clockwise: false,
        }),
        [a, b],
        1e-7,
    )
    .unwrap();
    let reversed = chart.clone().reversed();
    assert_eq!(
        reversed.poles,
        chart.poles.iter().rev().copied().collect::<Vec<_>>()
    );
    assert_eq!(reversed.weights, chart.weights);
    let [w0, w1, w2] = chart.weights.unwrap().try_into().unwrap();
    assert_eq!([w0, w2], [1.; 2]);
    assert!((w1 - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-15);
    for i in 0..=20 {
        let t = f64::from(i) / 20.;
        let weights = [(1. - t).powi(2) * w0, 2. * t * (1. - t) * w1, t * t * w2];
        let sum = weights.into_iter().sum::<f64>();
        let [x, y] = std::array::from_fn(|axis| {
            chart
                .poles
                .iter()
                .zip(weights)
                .map(|(p, w)| p[axis] * w)
                .sum::<f64>()
                / sum
        });
        assert!(((x - 2.).hypot(y - 2.) - 2.).abs() < 1e-14);
    }
}
