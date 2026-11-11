use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::{
        self, CustomThreadProfile, PrimitiveFrame, ThreadFamily, ThreadFit, ThreadHand, ThreadMode,
        ThreadParameters, ThreadRepresentation, ThreadRequest, length,
    },
};
use multicad_kernel::{BooleanOperation, MeshOptions, OCCT_LINKED, RigidTransform, Solid};
use multicad_model::{Compiler, GeometryBackend};
use std::f64::consts::PI;

fn fixtures() -> Vec<ThreadParameters> {
    let mut rows = vec![ThreadParameters::default()];
    for family in [ThreadFamily::Unc, ThreadFamily::Unf] {
        let row = model::unified_thread_presets()
            .into_iter()
            .find(|r| r.family == family)
            .unwrap();
        rows.push(ThreadParameters {
            family,
            fit: ThreadFit::Unified2A,
            major_diameter: length(row.diameter_mm),
            pitch: length(row.pitch_mm),
            length: length(4. * row.pitch_mm),
            ..Default::default()
        });
    }
    for family in [ThreadFamily::Bsw, ThreadFamily::Bsf, ThreadFamily::Bspp] {
        let row = model::whitworth_thread_presets()
            .into_iter()
            .find(|r| r.family == family)
            .unwrap();
        rows.push(ThreadParameters {
            family,
            fit: if family == ThreadFamily::Bspp {
                ThreadFit::BsppClassBExternal
            } else {
                ThreadFit::WhitworthMediumExternal
            },
            major_diameter: length(row.diameter_mm),
            pitch: length(row.pitch_mm),
            length: length(4. * row.pitch_mm),
            ..Default::default()
        });
    }
    for family in [ThreadFamily::Trapezoidal, ThreadFamily::Acme] {
        rows.push(
            model::lead_screw_presets()
                .into_iter()
                .find(|r| r.family == family)
                .unwrap()
                .parameters(),
        );
    }
    rows.push(
        model::buttress_thread_presets()[0]
            .parameters(model::ButtressLoadFlankSide::NegativeAxisSide),
    );
    rows.push(model::round_thread_presets()[0].parameters());
    rows.push(
        model::PipeThreadIntent {
            form: model::PipeThreadForm::R,
            size: model::PipeThreadSize::OneQuarter,
            gauge: model::PipeGaugeConvention::Iso7Nominal1994,
        }
        .parameters(),
    );
    rows.push(
        model::NptThreadIntent {
            form: model::NptThreadForm::External,
            size: model::NptThreadSize::OneQuarter,
            gauge: model::NptGaugeConvention::Nbs1959PlainFace,
            truncation: model::NptTruncationTarget::MidpointConeParallel,
        }
        .parameters(),
    );
    for k in [-0.03125, 0., 0.03125] {
        rows.push(ThreadParameters {
            family: ThreadFamily::Custom,
            custom: Some(CustomThreadProfile {
                radial_taper: Dimension::Literal {
                    value: k,
                    unit: "ratio".into(),
                },
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    rows
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn point(frame: &multicad_core::sketch::Frame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.n[i] * p[2]
    })
}
fn volume(thread: &model::ResolvedThread) -> f64 {
    let r = thread.major_radius_mm;
    let k = thread.profile.radial_taper;
    let h = thread.length_mm;
    let limit = thread
        .npt
        .as_ref()
        .and_then(|n| n.stock_crest_radius_limit_mm);
    let mut levels = vec![0., h];
    if let Some(maximum) = limit
        && k != 0.
    {
        let z = (maximum - r) / k;
        if z > 0. && z < h {
            levels.push(z);
        }
    }
    levels.sort_by(f64::total_cmp);
    levels
        .windows(2)
        .map(|ends| {
            let [a, b] =
                [ends[0], ends[1]].map(|z| limit.map_or(r + k * z, |m| (r + k * z).min(m)));
            PI * (ends[1] - ends[0]) * (a * a + a * b + b * b) / 3.
        })
        .sum()
}
#[test]
fn every_canonical_family_has_a_real_display_envelope_with_native_material_agreement() {
    let rows = fixtures();
    let families = rows
        .iter()
        .map(|r| format!("{:?}", r.family))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(families.len(), 13);
    for original in rows {
        for posed in [false, true] {
            let mut parameters = original.clone();
            parameters.representation = ThreadRepresentation::CosmeticEnvelope;
            parameters.hand =
                if posed && !matches!(parameters.family, ThreadFamily::Bspt | ThreadFamily::Npt) {
                    ThreadHand::Left
                } else {
                    ThreadHand::Right
                };
            if posed {
                parameters.frame = PrimitiveFrame {
                    origin: [7., -3., 2.].map(length),
                    u: [0.8, 0.6, 0.],
                    v: [-0.36, 0.48, 0.8],
                };
            }
            let mut session =
                Session::new(Document::new("Cosmetic family qualification").unwrap()).unwrap();
            let request = ThreadRequest::new("Display envelope", parameters.clone());
            let prepared =
                model::prepare_thread(&session, 0, Actor::local(Channel::Ui), &request).unwrap();
            session.commit(prepared.prepared).unwrap();
            let before = session.document().clone();
            let resolved = parameters
                .resolve(session.parameters(), ThreadMode::NewExternal)
                .unwrap();
            let model = compiler()
                .compile_with_backend(
                    session.document(),
                    session.revision(),
                    GeometryBackend::Rust,
                    None,
                )
                .unwrap_or_else(|e| panic!("{:?}: {e}", parameters.family));
            let solid = model.solid(request.feature).unwrap();
            let expected = volume(&resolved);
            assert!(
                (solid.volume_mm3().unwrap() - expected).abs() < expected.max(1.) * 1e-10,
                "{:?}",
                parameters.family
            );
            assert_eq!(solid.solid_count().unwrap(), 1);
            assert!(
                !solid
                    .mesh(MeshOptions::default())
                    .unwrap()
                    .indices
                    .is_empty()
            );
            let topology = solid.topology_snapshot(request.feature, 0).unwrap();
            topology.validate().unwrap();
            assert!(
                model::manufacturing_limitation_in_history(session.document(), request.feature)
                    .unwrap()
                    .is_some()
            );
            if OCCT_LINKED {
                let native_model = compiler()
                    .compile_with_backend(
                        session.document(),
                        session.revision(),
                        GeometryBackend::OpenCascade,
                        None,
                    )
                    .unwrap();
                let native = native_model.solid(request.feature).unwrap();
                assert!(
                    (native.volume_mm3().unwrap() - expected).abs() < expected.max(1.) * 2e-7,
                    "{:?}: {} vs {expected}",
                    parameters.family,
                    native.volume_mm3().unwrap()
                );
                let z = resolved.length_mm * 0.63;
                let radius = resolved.major_radius_mm + resolved.profile.radial_taper * z;
                let radius = resolved
                    .npt
                    .as_ref()
                    .and_then(|n| n.stock_crest_radius_limit_mm)
                    .map_or(radius, |r| radius.min(r));
                for (radial, inside) in [(radius - 0.02, true), (radius + 0.02, false)] {
                    let p = point(&resolved.frame, [radial, 0., z]);
                    let size = 0.002;
                    let probe = Solid::box_solid([size; 3], Default::default())
                        .unwrap()
                        .transformed(RigidTransform {
                            translation_mm: p.map(|v| v - size / 2.),
                            ..Default::default()
                        })
                        .unwrap();
                    let v = native
                        .boolean(&probe, BooleanOperation::Intersect, Default::default())
                        .unwrap()
                        .map_or(0., |s| s.volume_mm3().unwrap());
                    assert!(
                        (v / size.powi(3) - f64::from(inside)).abs() < 1e-3,
                        "{:?}",
                        parameters.family
                    );
                }
            }
            assert_eq!(session.document(), &before);
        }
    }
}

#[test]
fn a_small_resolvable_taper_is_a_cone_not_a_minimum_sized_radial_step() {
    for k in [-1e-6, 1e-6] {
        let parameters = ThreadParameters {
            family: ThreadFamily::Custom,
            representation: ThreadRepresentation::CosmeticEnvelope,
            major_diameter: length(6.),
            length: length(5.),
            custom: Some(CustomThreadProfile {
                radial_taper: Dimension::Literal {
                    value: k,
                    unit: "ratio".into(),
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut s = Session::new(Document::new("Small exact taper").unwrap()).unwrap();
        let request = ThreadRequest::new("Exact shallow cone", parameters);
        let prepared = model::prepare_thread(&s, 0, Actor::local(Channel::Ui), &request).unwrap();
        s.commit(prepared.prepared).unwrap();
        let model = compiler()
            .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
            .unwrap();
        let solid = model.solid(request.feature).unwrap();
        let end = 3. + 5. * k;
        let expected = PI * 5. * (9. + 3. * end + end * end) / 3.;
        assert!((solid.volume_mm3().unwrap() - expected).abs() < 1e-10);
        let z = 4.;
        let radius = 3. + k * z;
        assert!(
            solid
                .rust_revolution_contains([radius - 5e-7, 0., z])
                .unwrap()
        );
        assert!(
            !solid
                .rust_revolution_contains([radius + 5e-7, 0., z])
                .unwrap()
        );
        if OCCT_LINKED {
            // Use an independent native reader of the exact analytic cone.
            // Native's generic revolve builder can flatten a nearly cylindrical
            // side while retaining two different cap radii.
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("shallow-cone.step");
            solid.write_step(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("CONICAL_SURFACE"));
            let independent = Solid::read_step(&path, Default::default()).unwrap();
            assert!(
                (independent.volume_mm3().unwrap() - expected).abs() < 1e-7,
                "k={k}, native={}, expected={expected}",
                independent.volume_mm3().unwrap()
            );
        }
    }
}
