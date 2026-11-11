use super::*;
use crate::{
    Rational,
    helical_tube_cut::{Cut, Spec as CutSpec},
    spur_gear::{Hub, Spec, herringbone::Herringbone},
    step::Writer,
};
use std::f64::consts::PI;

const TOLERANCE: f64 = 1e-7;
const U: Point = [1., 0., 0.];
const V: Point = [0., 1., 0.];

fn gear(helical: bool, hand: f64, bore: f64) -> Herringbone {
    let pressure = 20_f64.to_radians();
    let spec = Spec {
        teeth: 12,
        pitch_radius_mm: 12.,
        base_radius_mm: 12. * pressure.cos(),
        tip_radius_mm: 14.,
        root_radius_mm: 9.5,
        root_fillet_radius_mm: 0.4,
        half_base_angle_rad: (PI - 0.05) / 24. + pressure.tan() - pressure,
        face_width_mm: 4.,
        bore_radius_mm: bore,
    };
    let create = if helical {
        Herringbone::new_helical
    } else {
        Herringbone::new
    };
    create(spec, hand * 0.02, 1e-5, 1e7).unwrap()
}

fn number(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn expected(gear: &Herringbone, [origin, u, v]: [Point; 3]) -> ExpectedStepMember<'_> {
    ExpectedStepMember::TwistedGear { gear, origin, u, v }
}

#[test]
fn mixed_compounds_verify_each_placed_twisted_gear_and_decode_the_analytic_slots() {
    let frame = [[20., 30., 40.], [0.8, 0., -0.6], V];
    let other_frame = [[-20., 10., 0.], U, [0., 0., 1.]];
    for (helical, hand, bore, [low, high]) in [
        (true, 1., 2., [-2., 6.]),
        (true, -1., 0., [-2., 3.]),
        (false, 1., 0., [1., 6.]),
        (false, -1., 2., [1., 3.]),
    ] {
        let gear = gear(helical, hand, bore)
            .with_hub(
                Hub {
                    radius_mm: 8.,
                    axial_start_mm: low,
                    axial_end_mm: high,
                },
                1e-5,
                1e7,
            )
            .unwrap();
        let other = self::gear(!helical, -hand, bore);
        let mut writer = Writer::new();
        writer.append_box([number(4); 3], [0.; 3], U, V).unwrap();
        writer
            .append_herringbone(&gear, frame[0], frame[1], frame[2], TOLERANCE)
            .unwrap();
        writer
            .append_herringbone(
                &other,
                other_frame[0],
                other_frame[1],
                other_frame[2],
                TOLERANCE,
            )
            .unwrap();
        let source = writer.finish().unwrap();
        let specification = [
            ExpectedStepMember::Analytic,
            expected(&gear, frame),
            expected(&other, other_frame),
        ];
        let verified = verify_authored_compound(&source, &specification, TOLERANCE).unwrap();
        let [
            VerifiedStepMember::Analytic { readback, imported },
            VerifiedStepMember::TwistedGear { volume_mm3 },
            VerifiedStepMember::TwistedGear {
                volume_mm3: other_volume,
            },
        ] = verified.as_slice()
        else {
            panic!("expected analytic stock followed by both authored gears");
        };
        assert_eq!(readback.volume_mm3, 64.);
        assert_eq!(readback.bounds_mm, [[0.; 3], [4.; 3]]);
        assert!(imported.is_none());
        assert!((volume_mm3 - gear.volume_mm3()).abs() < 1e-5);
        assert!((other_volume - other.volume_mm3()).abs() < 1e-5);
        assert!(
            verify_herringbone(&source, &gear, frame[0], frame[1], frame[2], TOLERANCE).is_err()
        );
        let face = source
            .lines()
            .find(|line| line.contains("=ADVANCED_FACE("))
            .unwrap();
        let damaged = source.replacen(face, &face.replace(",.T.);", ",.F.);"), 1);
        assert_ne!(damaged, source);
        assert!(verify_authored_compound(&damaged, &specification, TOLERANCE).is_err());

        for wrong in [
            specification[..1].to_vec(),
            vec![
                ExpectedStepMember::Analytic,
                expected(&gear, frame),
                expected(&other, other_frame),
                ExpectedStepMember::Analytic,
            ],
            vec![
                expected(&gear, frame),
                ExpectedStepMember::Analytic,
                expected(&other, other_frame),
            ],
            vec![
                ExpectedStepMember::Analytic,
                expected(&other, other_frame),
                expected(&gear, frame),
            ],
            vec![],
        ] {
            assert!(verify_authored_compound(&source, &wrong, TOLERANCE).is_err());
        }
    }
}

#[test]
fn bevel_slots_verify_each_placed_pair_member_beside_other_members() {
    use crate::bevel_gear::{Bevel, tests::spec};
    let pinion = Bevel::new(spec(2., 24, 32, 6., 4., 0.2), 1e-3, 1e7).unwrap();
    let wheel = Bevel::new(spec(2., 32, 24, 6., 0., 0.2), 1e-3, 1e7).unwrap();
    let twisted = gear(true, 1., 2.);
    let frames = [
        [[10., -20., 30.], [0.8, 0., -0.6], V],
        [[10., -20., 30.], [0., 0.6, 0.8], [1., 0., 0.]],
        [[-40., 0., 0.], U, V],
    ];
    let mut writer = Writer::new();
    writer.append_box([number(4); 3], [0.; 3], U, V).unwrap();
    for (gear, [origin, u, v]) in [&pinion, &wheel].into_iter().zip(frames) {
        writer.append_bevel(gear, origin, u, v, TOLERANCE).unwrap();
    }
    let [origin, u, v] = frames[2];
    writer
        .append_herringbone(&twisted, origin, u, v, TOLERANCE)
        .unwrap();
    let source = writer.finish().unwrap();
    let bevel = |gear, [origin, u, v]: [Point; 3]| ExpectedStepMember::Bevel { gear, origin, u, v };
    let specification = [
        ExpectedStepMember::Analytic,
        bevel(&pinion, frames[0]),
        bevel(&wheel, frames[1]),
        expected(&twisted, frames[2]),
    ];
    let verified = verify_authored_compound(&source, &specification, TOLERANCE).unwrap();
    let [
        VerifiedStepMember::Analytic { readback, .. },
        VerifiedStepMember::Bevel { volume_mm3: first },
        VerifiedStepMember::Bevel { volume_mm3: second },
        VerifiedStepMember::TwistedGear { volume_mm3: third },
    ] = verified.as_slice()
    else {
        panic!("expected stock, both bevels and the twisted gear");
    };
    assert_eq!(readback.volume_mm3, 64.);
    for (actual, expected) in [
        (*first, pinion.volume_mm3()),
        (*second, wheel.volume_mm3()),
        (*third, twisted.volume_mm3()),
    ] {
        assert!(
            (actual - expected).abs() / expected < 1e-8,
            "{actual} {expected}"
        );
    }
    // Swapped members or placements cannot certify the same file.
    for wrong in [
        [
            ExpectedStepMember::Analytic,
            bevel(&wheel, frames[1]),
            bevel(&pinion, frames[0]),
            expected(&twisted, frames[2]),
        ],
        [
            ExpectedStepMember::Analytic,
            bevel(&pinion, frames[1]),
            bevel(&wheel, frames[0]),
            expected(&twisted, frames[2]),
        ],
        [
            ExpectedStepMember::Analytic,
            bevel(&pinion, frames[0]),
            expected(&twisted, frames[2]),
            bevel(&wheel, frames[1]),
        ],
    ] {
        assert!(verify_authored_compound(&source, &wrong, TOLERANCE).is_err());
    }
    // A single-solid bevel proof rejects the compound.
    let [origin, u, v] = frames[0];
    assert!(crate::step_read::verify_bevel(&source, &pinion, origin, u, v, TOLERANCE).is_err());
}

#[test]
fn analytic_slots_keep_the_independently_reconstructed_helical_cut() {
    let cut = Cut::new(
        CutSpec {
            stock_radius_mm: 8.,
            stock_height_mm: 30.,
            tube_radius_mm: 0.6,
            pitch_mm: 2.,
            path_height_mm: 15.,
            start_height_mm: 16.2,
            start_angle_rad: 0.1,
        },
        TOLERANCE,
    )
    .unwrap();
    let gear = gear(true, -1., 2.);
    let frame = [[30., 0., 0.], U, V];
    let mut writer = Writer::new();
    writer
        .append_helical_tube_cut(cut, [4., 5., 6.], U, [0., 0., 1.], TOLERANCE)
        .unwrap();
    writer
        .append_herringbone(&gear, frame[0], frame[1], frame[2], TOLERANCE)
        .unwrap();
    let source = writer.finish().unwrap();
    let verified = verify_authored_compound(
        &source,
        &[ExpectedStepMember::Analytic, expected(&gear, frame)],
        TOLERANCE,
    )
    .unwrap();
    let VerifiedStepMember::Analytic {
        readback,
        imported: Some(imported),
    } = &verified[0]
    else {
        panic!("expected independently reconstructed helical cut");
    };
    assert!((readback.volume_mm3 - cut.volume_mm3()).abs() < TOLERANCE);
    assert_eq!(
        [imported.origin_mm, imported.u, imported.v],
        [[4., 5., 6.], U, [0., 0., 1.]]
    );
    assert!(matches!(
        imported.profile,
        ImportProfile::HelicalTubeCut { .. }
    ));
}

fn records<'a>(text: &'a str, kind: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
    text.lines().filter_map(move |line| {
        line.split_once('=')
            .filter(|(_, args)| args.starts_with(kind))
    })
}

#[test]
fn duplicate_members_shared_shells_and_orphan_topology_fail_the_whole_proof() {
    let gear = gear(false, 1., 2.)
        .with_hub(
            Hub {
                radius_mm: 8.,
                axial_start_mm: -2.,
                axial_end_mm: 6.,
            },
            1e-5,
            1e7,
        )
        .unwrap();
    let frame = [[0.; 3], U, V];
    let mut writer = Writer::new();
    for _ in 0..2 {
        writer
            .append_herringbone(&gear, frame[0], frame[1], frame[2], TOLERANCE)
            .unwrap();
    }
    let source = writer.finish().unwrap();
    let specification = [expected(&gear, frame); 2];
    assert!(verify_authored_compound(&source, &specification, TOLERANCE).is_ok());
    let solids = records(&source, "MANIFOLD_SOLID_BREP(").collect::<Vec<_>>();
    let shells = records(&source, "CLOSED_SHELL(")
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    let ids = [solids[0].0, solids[1].0].join(",");
    let duplicate = source.replace(&ids, &[solids[0].0; 2].join(","));
    let missing = source.replace(&ids, solids[0].0);
    let shared = source.replace(solids[1].1, &solids[1].1.replace(shells[1], shells[0]));
    let orphan = source.replace(
        "ENDSEC;\nEND-ISO-10303-21;",
        "#999999=VERTEX_POINT('',#1);\nENDSEC;\nEND-ISO-10303-21;",
    );
    for invalid in [duplicate, missing, shared, orphan] {
        assert_ne!(invalid, source);
        assert!(verify_authored_compound(&invalid, &specification, TOLERANCE).is_err());
    }
}

#[test]
fn worm_pairs_verify_each_member_beside_analytic_slots() {
    use crate::worm_gear::{wheel::tests::spec as wheel_spec, worm::tests::spec as worm_spec};
    let worm = Worm::new(worm_spec(1., 20., 2., 0.04), 1e-3).unwrap();
    let wheel = Wheel::new(wheel_spec(1., 40, 1.5, 4., 0.04), 1e-3, 1e7).unwrap();
    // Worm axis along wheel +Y at center distance 25, wheel tilted.
    let worm_frame = [[25., 0., 0.], [1., 0., 0.], [0., 0., -1.]];
    let wheel_frame = [[0.; 3], [0.8, 0.6, 0.], [-0.6, 0.8, 0.]];
    let mut writer = Writer::new();
    writer
        .append_za_worm(
            &worm,
            worm_frame[0],
            worm_frame[1],
            worm_frame[2],
            TOLERANCE,
        )
        .unwrap();
    writer.append_box([number(4); 3], [40.; 3], U, V).unwrap();
    writer
        .append_worm_wheel(
            &wheel,
            wheel_frame[0],
            wheel_frame[1],
            wheel_frame[2],
            TOLERANCE,
        )
        .unwrap();
    let source = writer.finish().unwrap();
    let [o, u, v] = worm_frame;
    let [p, q, r] = wheel_frame;
    let members = [
        ExpectedStepMember::Worm {
            worm: &worm,
            origin: o,
            u,
            v,
        },
        ExpectedStepMember::Analytic,
        ExpectedStepMember::WormWheel {
            wheel: &wheel,
            origin: p,
            u: q,
            v: r,
        },
    ];
    let verified = verify_authored_compound(&source, &members, TOLERANCE).unwrap();
    let [
        VerifiedStepMember::Worm { volume_mm3: a },
        VerifiedStepMember::Analytic { readback, .. },
        VerifiedStepMember::WormWheel { volume_mm3: b },
    ] = verified.as_slice()
    else {
        panic!("{verified:?}");
    };
    assert!((a - worm.volume_mm3()).abs() / a < 1e-8, "{a}");
    assert!((b - wheel.volume_mm3()).abs() / b < 1e-8, "{b}");
    assert!((readback.volume_mm3 - 64.).abs() < 1e-9);
    // Swapped expectations and a moved wheel are refused.
    let swapped = [members[2], members[1], members[0]];
    assert!(verify_authored_compound(&source, &swapped, TOLERANCE).is_err());
    let moved = [
        members[0],
        members[1],
        ExpectedStepMember::WormWheel {
            wheel: &wheel,
            origin: [0., 0., 1.],
            u: q,
            v: r,
        },
    ];
    assert!(verify_authored_compound(&source, &moved, TOLERANCE).is_err());
}
