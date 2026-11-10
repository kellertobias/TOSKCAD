use super::*;
use crate::{spur_gear::Spec, step::write_herringbone};

const TOL: f64 = 1e-7;
const XY: [Point; 3] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];

fn fixture(teeth: u32, hand: f64, bore: f64) -> Herringbone {
    let beta = 15_f64.to_radians();
    let pressure = (20_f64.to_radians().tan() / beta.cos()).atan();
    let module = 2. / beta.cos();
    let pitch = f64::from(teeth) * module / 2.;
    Herringbone::new(
        Spec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: pitch * pressure.cos(),
            tip_radius_mm: pitch + 2.,
            root_radius_mm: pitch - 2.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: (PI * module / 2. - 0.05) / (2. * pitch) + pressure.tan()
                - pressure,
            face_width_mm: 4.,
            bore_radius_mm: bore,
        },
        hand * beta.tan() / pitch,
        1e-5,
        1e7,
    )
    .unwrap()
}

#[test]
fn readback_integrates_actual_authored_bands_and_optional_bore() {
    for (teeth, hand, bore, [origin, u, v]) in [
        (24, 1., 4., XY),
        (36, -1., 4., XY),
        (24, 1., 0., [[13., -7., 20.], [1., 0., 0.], [0., 0.6, 0.8]]),
    ] {
        let gear = fixture(teeth, hand, bore);
        let step = write_herringbone(&gear, origin, u, v, TOL).unwrap();
        let volume = verify_herringbone(&step, &gear, origin, u, v, TOL).unwrap();
        let expected = gear.volume_mm3();
        assert!((volume - expected).abs() < 1e-5, "{volume} != {expected}");
    }
}

fn rewrite(
    source: &str,
    predicate: impl Fn(&str) -> bool,
    change: impl FnOnce(&str) -> String,
) -> String {
    let line = source.lines().find(|line| predicate(line)).unwrap();
    source.replacen(line, &change(line), 1)
}

fn argument(line: &str, entity: &str, index: usize) -> std::ops::Range<usize> {
    let mut start = line.find(entity).unwrap() + entity.len();
    let mut depth = 0;
    let mut remaining = index;
    for (offset, byte) in line.bytes().enumerate().skip(start) {
        match byte {
            b'(' => depth += 1,
            b')' if depth > 0 => depth -= 1,
            b',' | b')' if depth == 0 => {
                if remaining == 0 {
                    return start..offset;
                }
                remaining -= 1;
                start = offset + 1;
            }
            _ => {}
        }
    }
    panic!("missing argument")
}

fn changed_argument(line: &str, entity: &str, index: usize, replacement: &str) -> String {
    let mut result = line.to_owned();
    result.replace_range(argument(line, entity, index), replacement);
    result
}

#[test]
fn rejects_invalid_shell_records_instead_of_repairing_them() {
    let [origin, u, v] = XY;
    let gear = fixture(12, 1., 2.);
    let source = write_herringbone(&gear, origin, u, v, TOL).unwrap();
    assert!(verify_herringbone(&source, &gear, origin, u, u, TOL).is_err());
    let numbered = |name: &str, index| {
        source
            .lines()
            .filter(|line| line.contains(name))
            .nth(index)
            .unwrap()
    };
    let by_id = |id: &str| {
        source
            .lines()
            .find(|line| line.split_once('=').unwrap_or_default().0 == id)
            .unwrap()
    };
    let arg = |line: &str, entity: &str, i| line[argument(line, entity, i)].to_owned();
    let id = |line: &str| line.split_once('=').unwrap().0.to_owned();
    let n = gear.profile().outline().len();
    let cap = numbered("=PLANE(", 0);
    let bore_support = numbered("=CYLINDRICAL_SURFACE(", 0);
    let bore_face = numbered("=ADVANCED_FACE(", 2 * n + 2);
    let hole = numbered("=FACE_BOUND(", 0);
    let hole_loop = by_id(&arg(hole, "FACE_BOUND(", 1));
    let hole_use = arg(hole_loop, "EDGE_LOOP(", 1)
        .trim_matches(&['(', ')'][..])
        .to_owned();
    let circle_edge = by_id(&arg(by_id(&hole_use), "ORIENTED_EDGE(", 3));
    let seam = numbered("=EDGE_CURVE(", 5 * n + 2);
    let tooth_line = source
        .lines()
        .filter(|line| line.contains("=EDGE_CURVE("))
        .find(|line| by_id(&arg(line, "EDGE_CURVE(", 3)).contains("=LINE("))
        .unwrap();
    let copy_record = |original: &str, target: &str| {
        format!("{}={}", id(target), original.split_once('=').unwrap().1)
    };

    let spline = numbered("=B_SPLINE_SURFACE_WITH_KNOTS(", 0);
    let mults = arg(spline, "B_SPLINE_SURFACE_WITH_KNOTS(", 8);
    let different_mults = format!("(99{}", &mults[mults.find(',').unwrap()..]);
    let bad = [
        (spline, "B_SPLINE_SURFACE_WITH_KNOTS(", 8, different_mults),
        (circle_edge, "EDGE_CURVE(", 4, ".F.".to_owned()),
        (
            numbered("=CLOSED_SHELL(", 0),
            "CLOSED_SHELL(",
            1,
            "()".to_owned(),
        ),
        (cap, "PLANE(", 0, "$".to_owned()),
        (cap, "PLANE(", 1, arg(numbered("=PLANE(", 1), "PLANE(", 1)),
        (
            hole_loop,
            "EDGE_LOOP(",
            1,
            format!("({hole_use},{hole_use})"),
        ),
        (bore_face, "ADVANCED_FACE(", 3, ".T.".to_owned()),
        (bore_support, "CYLINDRICAL_SURFACE(", 2, "$".to_owned()),
        (
            numbered("=B_SPLINE_SURFACE_WITH_KNOTS(", 0),
            "B_SPLINE_SURFACE_WITH_KNOTS(",
            4,
            ".T.".to_owned(),
        ),
        (
            numbered("=B_SPLINE_SURFACE_WITH_KNOTS(", 0),
            "B_SPLINE_SURFACE_WITH_KNOTS(",
            8,
            "$".to_owned(),
        ),
        (
            numbered("=B_SPLINE_SURFACE_WITH_KNOTS(", 0),
            "B_SPLINE_SURFACE_WITH_KNOTS(",
            3,
            "$".to_owned(),
        ),
        (
            numbered("RATIONAL_B_SPLINE_SURFACE(", 0),
            "B_SPLINE_SURFACE(",
            3,
            ".T.".to_owned(),
        ),
        (
            numbered("RATIONAL_B_SPLINE_CURVE(", 0),
            "B_SPLINE_CURVE(",
            3,
            ".T.".to_owned(),
        ),
        (
            numbered("=B_SPLINE_CURVE_WITH_KNOTS(", 0),
            "B_SPLINE_CURVE_WITH_KNOTS(",
            4,
            ".T.".to_owned(),
        ),
        (
            numbered("=EDGE_CURVE(", 0),
            "EDGE_CURVE(",
            4,
            ".F.".to_owned(),
        ),
    ];
    for (original, entity, index, replacement) in bad {
        let changed = changed_argument(original, entity, index, &replacement);
        assert_ne!(changed, original);
        assert!(
            verify_herringbone(
                &source.replacen(original, &changed, 1),
                &gear,
                origin,
                u,
                v,
                TOL
            )
            .is_err(),
            "{entity} arg {index}"
        );
    }
    for (original, changed) in [
        (circle_edge, copy_record(seam, circle_edge)),
        (seam, copy_record(tooth_line, seam)),
        (
            numbered("RATIONAL_B_SPLINE_SURFACE(", 0),
            numbered("RATIONAL_B_SPLINE_SURFACE(", 0).replace(" SURFACE()", ""),
        ),
        (
            numbered("RATIONAL_B_SPLINE_CURVE(", 0),
            numbered("RATIONAL_B_SPLINE_CURVE(", 0).replace(" CURVE()", ""),
        ),
    ] {
        assert_ne!(changed, original);
        assert!(
            verify_herringbone(
                &source.replacen(original, &changed, 1),
                &gear,
                origin,
                u,
                v,
                TOL
            )
            .is_err()
        );
    }
    let bore_loop = numbered("=EDGE_LOOP(", 2 * n + 4);
    let uses = arg(bore_loop, "EDGE_LOOP(", 1);
    let first = uses.trim_start_matches('(').split(',').next().unwrap();
    let edge = by_id(first);
    let changed = changed_argument(edge, "ORIENTED_EDGE(", 4, ".T.");
    assert_ne!(edge, changed);
    assert!(
        verify_herringbone(
            &source.replacen(edge, &changed, 1),
            &gear,
            origin,
            u,
            v,
            TOL
        )
        .is_err()
    );
}

#[test]
fn integrates_short_rational_arcs_without_cancellation() {
    let a = 1e-3_f64;
    let weight = a.cos();
    let chart = Chart {
        degree: 2,
        interior_mult: 2,
        knots: vec![0., 1.],
        poles: vec![],
        weights: Some(vec![1., weight, 1.]),
    };
    let [origin, u, v] = XY;
    let frame = Frame {
        origin,
        u,
        v,
        normal: cross(u, v),
    };
    let entities = BTreeMap::new();
    let shell = Shell::new(&entities, TOL, true);
    let points = [
        [weight, -a.sin(), 0.],
        [weight.recip(), 0., 0.],
        [weight, a.sin(), 0.],
    ];
    let area = section_area(&shell, 0, &chart, &points, &frame).unwrap();
    assert!((area - a).abs() < 1e-14);
}

#[test]
fn refuses_wrong_hand_tampered_surface_trim_and_incidence() {
    let [origin, u, v] = XY;
    let gear = fixture(12, 1., 2.);
    let source = write_herringbone(&gear, origin, u, v, TOL).unwrap();
    assert!(verify_herringbone(&source, &gear, origin, u, v, TOL).is_ok());
    let other = fixture(12, -1., 2.);
    assert!(verify_herringbone(&source, &other, origin, u, v, TOL).is_err());

    let oriented = |index| {
        source
            .lines()
            .filter(|line| line.contains("=ORIENTED_EDGE"))
            .nth(index)
            .unwrap()
    };
    let edge_id = |line: &str| line.split(',').nth(3).unwrap().to_owned();
    let bottom = oriented(0);
    let apex = oriented(4 * gear.profile().outline().len());
    let wrong_apex = source.replacen(apex, &apex.replace(&edge_id(apex), &edge_id(bottom)), 1);
    assert_ne!(source, wrong_apex);
    assert!(verify_herringbone(&wrong_apex, &gear, origin, u, v, TOL).is_err());
    let changed = [
        rewrite(
            &source,
            |line| line.contains("=ADVANCED_FACE"),
            |line| line.replace(",.T.);", ",.F.);"),
        ),
        rewrite(
            &source,
            |line| line.contains("RATIONAL_B_SPLINE_SURFACE"),
            |line| {
                line.replacen(
                    "RATIONAL_B_SPLINE_SURFACE(((1.",
                    "RATIONAL_B_SPLINE_SURFACE(((0.",
                    1,
                )
            },
        ),
        rewrite(
            &source,
            |line| line.contains("=ORIENTED_EDGE"),
            |line| line.replace(",.T.);", ",.F.);"),
        ),
        rewrite(
            &source,
            |line| line.contains("RATIONAL_B_SPLINE_CURVE"),
            |line| {
                line.replacen(
                    "RATIONAL_B_SPLINE_CURVE((1.",
                    "RATIONAL_B_SPLINE_CURVE((0.",
                    1,
                )
            },
        ),
        rewrite(
            &source,
            |line| line.contains("=CYLINDRICAL_SURFACE"),
            |line| line.replace("2.0000000000000000E0", "2.1000000000000000E0"),
        ),
    ];
    for (index, text) in changed.into_iter().enumerate() {
        assert_ne!(text, source, "mutation {index} must change the STEP");
        assert!(
            verify_herringbone(&text, &gear, origin, u, v, TOL).is_err(),
            "accepted mutation {index}"
        );
    }
}
