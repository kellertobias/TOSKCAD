use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{self, length, *},
};
use multicad_kernel::{BooleanOperation, RigidTransform, Solid};
use multicad_model::Compiler;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compile(s: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(s.document(), s.revision())
    .unwrap()
}
fn pair(
    d: f64,
    pitch: f64,
    hand: ThreadHand,
    treated: bool,
    wall_outer: Option<f64>,
) -> (Session, ThreadRequest, ThreadRequest) {
    let mut s = Session::new(Document::new("Metric fit metrology").unwrap()).unwrap();
    let mut p = ThreadParameters {
        major_diameter: length(d),
        pitch: length(pitch),
        length: length(1.1 * d),
        hand,
        fit: ThreadFit::Metric6gExternal,
        ..Default::default()
    };
    if treated {
        p.end_treatments = Some(ThreadEndTreatments {
            start: ThreadStartTreatment::Chamfer45 {
                radial_depth: length(0.8 * pitch),
            },
            end: ThreadEndTreatment::CylindricalRelief {
                length: length(0.6 * pitch),
                radial_depth: length(0.1 * pitch),
            },
        });
    }
    let male = ThreadRequest::new("6g shaft", p.clone());
    let q = model::prepare_thread(&s, s.revision(), actor(), &male).unwrap();
    s.commit(q.prepared).unwrap();
    let blank = PrimitiveRequest::new(
        "Receiver",
        PrimitiveParameters::Cylinder {
            radius: length(wall_outer.unwrap_or(d / 2. + 2.)),
            height: p.length.clone(),
            frame: Default::default(),
        },
    );
    let q = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(q.prepared).unwrap();
    p.fit = ThreadFit::Metric6HInternal;
    let female = ThreadRequest::receiving(
        "6H internal",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        p,
    )
    .unwrap();
    let q = model::prepare_thread(&s, s.revision(), actor(), &female).unwrap();
    s.commit(q.prepared).unwrap();
    (s, male, female)
}
fn material(s: &Solid, point: [f64; 3]) -> f64 {
    let edge = 0.00002;
    let q = Solid::box_solid([edge; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|v| v - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    s.boolean(&q, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |v| v.volume_mm3().unwrap())
        / edge.powi(3)
}
#[test]
fn metric_fitted_native_root_circles_mating_and_step_are_real_for_both_sizes_and_hands() {
    let dir = tempfile::tempdir().unwrap();
    for row in metric_fit_presets() {
        for hand in [ThreadHand::Right, ThreadHand::Left] {
            let (s, male, female) = pair(row.diameter_mm, row.pitch_mm, hand, false, None);
            let built = compile(&s);
            let a = built.solid(male.feature).unwrap();
            let b = built.solid(female.feature).unwrap();
            assert!(
                a.boolean(b, BooleanOperation::Intersect, Default::default())
                    .unwrap()
                    .is_none()
            );
            let r = male
                .parameters
                .resolve(s.parameters(), ThreadMode::NewExternal)
                .unwrap();
            let radius = r
                .metric_fit
                .as_ref()
                .unwrap()
                .external_root_radius_mm
                .unwrap();
            let cr = r.major_radius_mm - r.profile.depth_mm + radius;
            let path = dir
                .path()
                .join(format!("metric-{}-{hand:?}.step", row.diameter_mm));
            a.write_step(&path).unwrap();
            let back = Solid::read_step(&path, Default::default()).unwrap();
            let volume = a.volume_mm3().unwrap();
            let area = a.faces().unwrap().iter().map(|f| f.area_mm2).sum::<f64>();
            let budget = multicad_kernel::GeometryPolicy::default().linear_tolerance_mm * area
                + 1e-7 * volume;
            let delta = (back.volume_mm3().unwrap() - volume).abs();
            let x = radius * (std::f64::consts::PI / 3.).sin();
            let primitive = |x: f64| {
                (cr * cr + radius * radius) * x
                    - x.powi(3) / 3.
                    - cr * (x * (radius * radius - x * x).sqrt()
                        + radius * radius * (x / radius).asin())
            };
            let tangent = cr - radius / 2.;
            let squared = r.profile.crest_width_mm * r.major_radius_mm.powi(2)
                + 2. / 3_f64.sqrt() * (r.major_radius_mm.powi(3) - tangent.powi(3)) / 3.
                + primitive(x)
                - primitive(-x);
            let expected = std::f64::consts::PI * r.length_mm * squared / r.pitch_mm;
            assert!(
                (volume - expected).abs() <= budget,
                "native versus analytic {} {hand:?}: {volume}/{expected}, budget{budget}",
                row.diameter_mm
            );
            assert!(
                (back.volume_mm3().unwrap() - expected).abs() <= budget,
                "STEP versus analytic {} {hand:?}",
                row.diameter_mm
            );
            assert!(
                delta <= budget,
                "untreated {} {hand:?} native{volume} STEP{} analytic{expected} delta{delta} budget{budget}",
                row.diameter_mm,
                back.volume_mm3().unwrap()
            );
            for (feature, shape) in [(male.feature, a), (female.feature, b)] {
                let topology = shape.topology_snapshot(feature, 0).unwrap();
                assert!(topology.elements.iter().all(|e| !e.origins.is_empty()));
                let bounds = shape.bounds().unwrap();
                assert!(bounds.min[2].abs() < 16e-7);
                assert!((bounds.max[2] - r.length_mm).abs() < 16e-7);
            }
            for shape in [a, &back] {
                for angle in [-0.8_f64, -0.4, 0., 0.4, 0.8] {
                    let z = 2.37 * row.pitch_mm;
                    let relative = row.pitch_mm / 2. + radius * angle.sin();
                    let boundary = cr - radius * angle.cos();
                    let phi = if hand == ThreadHand::Left { -1. } else { 1. }
                        * std::f64::consts::TAU
                        * (z - relative)
                        / row.pitch_mm;
                    for (offset, inside) in [(-0.001, true), (0.001, false)] {
                        let radial = boundary + offset;
                        let value = material(shape, [radial * phi.cos(), radial * phi.sin(), z]);
                        assert!(
                            if inside { value > 0.999 } else { value < 1e-5 },
                            "circle {} {hand:?} {angle} {offset} {value}",
                            row.diameter_mm
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn custom_entry_and_terminal_relief_change_native_sections_and_enforce_the_largest_wall_envelope() {
    for hand in [ThreadHand::Right, ThreadHand::Left] {
        let (s, male, female) = pair(3., 0.5, hand, true, None);
        let built = compile(&s);
        let a = built.solid(male.feature).unwrap();
        let b = built.solid(female.feature).unwrap();
        assert!(
            a.boolean(b, BooleanOperation::Intersect, Default::default())
                .unwrap()
                .is_none()
        );
        for (request, mode, shape, internal) in [
            (&male, ThreadMode::NewExternal, a, false),
            (&female, ThreadMode::CutInternal, b, true),
        ] {
            let r = request.parameters.resolve(s.parameters(), mode).unwrap();
            let t = r.end_treatments.as_ref().unwrap();
            for (z, boundary) in [
                (
                    0.01,
                    if internal {
                        r.major_radius_mm - r.profile.depth_mm + t.start_chamfer_radial_depth_mm
                            - 0.01
                    } else {
                        r.major_radius_mm - t.start_chamfer_radial_depth_mm + 0.01
                    },
                ),
                (
                    r.length_mm - 0.1,
                    if internal {
                        r.major_radius_mm + t.end_relief_radial_depth_mm
                    } else {
                        r.major_radius_mm - r.profile.depth_mm - t.end_relief_radial_depth_mm
                    },
                ),
            ] {
                for phi in [0_f64, 0.43, 2.1] {
                    for (offset, inside) in [(-0.001, !internal), (0.001, internal)] {
                        let radius = boundary + offset;
                        let value = material(shape, [radius * phi.cos(), radius * phi.sin(), z]);
                        assert!(
                            if inside { value > 0.999 } else { value < 1e-5 },
                            "treatment {hand:?} {mode:?} z{z} offset{offset}: {value}"
                        );
                    }
                }
            }
        }
    }
    let (s, _, _) = pair(3., 0.5, ThreadHand::Right, true, Some(2.55));
    assert!(
        Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(s.document(), s.revision())
        .is_err()
    );
}
