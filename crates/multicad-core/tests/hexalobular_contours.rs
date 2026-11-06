use multicad_core::model::{HexalobularArc, HexalobularContour};
use std::f64::consts::{PI, TAU};
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn sweep(arc: &HexalobularArc) -> (f64, f64) {
    let start = (arc.start[1] - arc.center[1]).atan2(arc.start[0] - arc.center[0]);
    let end = (arc.end[1] - arc.center[1]).atan2(arc.end[0] - arc.center[0]);
    let delta = if arc.convex {
        (end - start).rem_euclid(TAU)
    } else {
        -((start - end).rem_euclid(TAU))
    };
    (start, delta)
}
fn descriptor_area(arcs: &[HexalobularArc]) -> f64 {
    arcs.iter()
        .map(|arc| {
            let (t, d) = sweep(arc);
            let r = arc.radius;
            let [x, y] = arc.center;
            (r * r * d + r * x * ((t + d).sin() - t.sin()) - r * y * ((t + d).cos() - t.cos())) / 2.
        })
        .sum()
}
#[test]
fn nine_basic_dimension_rows_form_twelve_tangent_circular_arcs() {
    // Basic A/B scalar rows from ISO10664:2014 Table1. re=0.1A is an
    // EXPLICIT engineering choice suggested by historical informative AnnexA,
    // not a normative product/gauge radius or conformity claim.
    let rows = [
        (1.75, 1.27),
        (2.4, 1.75),
        (2.8, 2.05),
        (3.35, 2.4),
        (3.95, 2.85),
        (4.5, 3.25),
        (5.6, 4.05),
        (7.93, 5.64),
        (8.95, 6.45),
    ];
    for (a, b) in rows {
        let contour = HexalobularContour::new(a, b, a * 0.1).unwrap();
        assert_eq!(contour.arcs.len(), 12);
        for (i, arc) in contour.arcs.iter().enumerate() {
            let next = &contour.arcs[(i + 1) % 12];
            assert_eq!(
                arc.end, next.start,
                "closure must copy shared endpoints bit-exactly"
            );
            assert_eq!(arc.convex, i % 2 == 0);
            for p in [arc.start, arc.mid, arc.end] {
                assert!((distance(p, arc.center) - arc.radius).abs() < a * 2e-14);
            }
            let tangent = |part: &HexalobularArc, p: [f64; 2]| {
                let sign = if part.convex { 1. } else { -1. };
                [
                    sign * -(p[1] - part.center[1]) / part.radius,
                    sign * (p[0] - part.center[0]) / part.radius,
                ]
            };
            let t = tangent(arc, arc.end);
            let u = tangent(next, next.start);
            assert!(distance(t, u) < 3e-14, "tangents differ at {i}");
            assert!(
                (arc.mid[0].hypot(arc.mid[1]) - if arc.convex { a / 2. } else { b / 2. }).abs()
                    < a * 1e-14
            );
            let (start, delta) = sweep(arc);
            let mut last: Option<f64> = None;
            for j in 0..=256 {
                let t = start + delta * j as f64 / 256.;
                let p = [
                    arc.center[0] + arc.radius * t.cos(),
                    arc.center[1] + arc.radius * t.sin(),
                ];
                let rho = p[0].hypot(p[1]);
                assert!(rho >= b / 2. - a * 2e-14 && rho <= a / 2. + a * 2e-14);
                let angle = p[1].atan2(p[0]);
                if let Some(previous) = last {
                    assert!((angle - previous).rem_euclid(TAU) < 0.1);
                }
                last = Some(angle);
            }
        }
        // Independently solve tangency by bisection of center distance, then
        // integrate the radial intersection with the two circles (polar area).
        let re = a * 0.1;
        let c = a / 2. - re;
        let small = b / 2.;
        let cos = (PI / 6.).cos();
        let residual =
            |ri: f64| (c * c + (small + ri).powi(2) - 2. * c * (small + ri) * cos).sqrt() - re - ri;
        let mut lo = 0.;
        let mut hi = 10. * a;
        assert!(residual(lo) > 0. && residual(hi) < 0.);
        for _ in 0..100 {
            let mid = (lo + hi) / 2.;
            if residual(mid) > 0. {
                lo = mid
            } else {
                hi = mid
            }
        }
        let ri = (lo + hi) / 2.;
        assert!((ri - contour.inner_valley_radius_mm).abs() < a * 3e-14);
        let d = small + ri;
        let split = contour.arcs[0].end[1].atan2(contour.arcs[0].end[0]);
        let polar = |t: f64, outer: bool| {
            if outer {
                c * t.cos() + (re * re - c * c * t.sin().powi(2)).sqrt()
            } else {
                let t = t - PI / 6.;
                d * t.cos() - (ri * ri - d * d * t.sin().powi(2)).sqrt()
            }
        };
        let integrate = |start: f64, end: f64, outer: bool| {
            let n = 10000;
            let h = (end - start) / n as f64;
            let mut sum = 0.;
            for i in 0..=n {
                let weight = if i == 0 || i == n {
                    1.
                } else if i % 2 == 0 {
                    2.
                } else {
                    4.
                };
                sum += weight * polar(start + i as f64 * h, outer).powi(2) / 2.;
            }
            sum * h / 3.
        };
        let area = 12. * (integrate(0., split, true) + integrate(split, PI / 6., false));
        assert!((descriptor_area(&contour.arcs) - area).abs() < a * a * 2e-11);
    }
}
#[test]
fn t10_independent_radius_and_invalid_inputs() {
    let c = HexalobularContour::new(2.8, 2.05, 0.28).unwrap();
    assert!((c.inner_valley_radius_mm - 0.5295228096485656).abs() < 2e-15);
    for (a, b, r) in [
        (0., 1., 0.1),
        (2., 0., 0.1),
        (2., 2., 0.1),
        (2., 3., 0.1),
        (2., 1., 0.),
        (2., 1., 1.),
        (2.8, 2.7, 0.28),
        (2.8, 0.01, 0.28),
        (f64::NAN, 1., 0.1),
        (2., f64::INFINITY, 0.1),
        (2., 1., f64::NAN),
    ] {
        assert!(
            HexalobularContour::new(a, b, r).is_err(),
            "invalid {a}/{b}/{r}"
        );
    }
}
