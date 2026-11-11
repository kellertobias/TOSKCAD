use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::{
        self, BodyInput, PrimitiveParameters, PrimitiveRequest, ThreadFamily, ThreadFit,
        ThreadHand, ThreadMode, ThreadParameters, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
    references::{self, GeometryIntent, Resolution, TopologyKind},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compile(s: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
    .unwrap()
}
fn thread_parameters(family: ThreadFamily, hand: ThreadHand, internal: bool) -> ThreadParameters {
    let pitch = 25.4
        / if family == ThreadFamily::Unc {
            20.
        } else {
            28.
        };
    ThreadParameters {
        family,
        hand,
        fit: if internal {
            ThreadFit::Unified2B
        } else {
            ThreadFit::Unified2A
        },
        major_diameter: Dimension::Literal {
            value: 0.25,
            unit: "in".into(),
        },
        pitch: length(pitch),
        length: length(4. * pitch),
        ..Default::default()
    }
}
#[test]
fn sourced_unc_unf_profiles_keep_fits_material_history_and_verified_manufacturing_export() {
    for family in [ThreadFamily::Unc, ThreadFamily::Unf] {
        for hand in [ThreadHand::Right, ThreadHand::Left] {
            let mut s = Session::new(Document::new("Rust Unified threads").unwrap()).unwrap();
            let parameters = thread_parameters(family, hand, false);
            let resolved = parameters
                .resolve(s.parameters(), ThreadMode::NewExternal)
                .unwrap();
            let shaft = ThreadRequest::new("Unified 2A", parameters.clone());
            s.commit(
                model::prepare_thread(&s, s.revision(), actor(), &shaft)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
            let blank = PrimitiveRequest::new(
                "Nut stock",
                PrimitiveParameters::Cylinder {
                    radius: length(5.),
                    height: length(resolved.length_mm),
                    frame: Default::default(),
                },
            );
            s.commit(
                model::prepare_primitive(&s, s.revision(), actor(), &blank)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
            let internal = thread_parameters(family, hand, true);
            let tap = ThreadRequest::receiving(
                "Unified 2B",
                ThreadMode::CutInternal,
                BodyInput {
                    body: blank.body,
                    producer: blank.feature,
                },
                internal.clone(),
            )
            .unwrap();
            s.commit(
                model::prepare_thread(&s, s.revision(), actor(), &tap)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
            let model = compile(&s);
            let screw = model.solid(shaft.feature).unwrap();
            let nut = model.solid(tap.feature).unwrap();
            assert!(screw.uses_rust_geometry() && nut.uses_rust_geometry());
            let r = resolved.actual_pitch_diameter_mm.unwrap() / 2.;
            assert!(
                screw
                    .rust_thread_contains([r, 0., 2.20 * resolved.pitch_mm])
                    .unwrap()
            );
            assert!(
                !screw
                    .rust_thread_contains([r, 0., 2.30 * resolved.pitch_mm])
                    .unwrap()
            );
            assert_eq!(resolved.fit.as_ref().unwrap().class, ThreadFit::Unified2A);
            assert_eq!(
                internal
                    .resolve(s.parameters(), ThreadMode::CutInternal)
                    .unwrap()
                    .fit
                    .as_ref()
                    .unwrap()
                    .class,
                ThreadFit::Unified2B
            );
            let shape = &model.snapshot().shapes[&shaft.feature];
            let face = shape
                .topology
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face
                        && e.origins.iter().any(|o| o.role == "cap.start")
                })
                .unwrap();
            let intent = GeometryIntent::capture(&shape.topology, face.key).unwrap();
            let volume = screw.volume_mm3().unwrap();
            let dir = tempfile::tempdir().unwrap();
            for (i, solid) in [screw, nut].into_iter().enumerate() {
                let path = dir.path().join(format!("thread-{i}.step"));
                solid.write_step(&path).unwrap();
                let decoded = solid
                    .verify_helical_step(&std::fs::read_to_string(&path).unwrap())
                    .unwrap()
                    .unwrap();
                assert!((decoded - solid.volume_mm3().unwrap()).abs() < 1e-3);
                #[cfg(feature = "occt")]
                {
                    let imported =
                        multicad_kernel::Solid::read_step(&path, Default::default()).unwrap();
                    assert_eq!(imported.solid_count().unwrap(), 1);
                    assert!(
                        (imported.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-3
                    );
                }
            }
            #[cfg(feature = "occt")]
            if hand == ThreadHand::Right {
                let native = Compiler {
                    omit_display: true,
                    ..Default::default()
                }
                .compile_with_backend(
                    s.document(),
                    s.revision(),
                    GeometryBackend::OpenCascade,
                    None,
                )
                .unwrap();
                for id in [shaft.feature, tap.feature] {
                    let native = native.solid(id).unwrap();
                    let rust = model.solid(id).unwrap();
                    assert!(
                        (native.volume_mm3().unwrap() - rust.volume_mm3().unwrap()).abs() < 1e-3
                    );
                    for (a, b) in native
                        .bounds()
                        .unwrap()
                        .min
                        .into_iter()
                        .chain(native.bounds().unwrap().max)
                        .zip(
                            rust.bounds()
                                .unwrap()
                                .min
                                .into_iter()
                                .chain(rust.bounds().unwrap().max),
                        )
                    {
                        assert!((a - b).abs() < 1e-5);
                    }
                }
            }
            let before = s.document().clone();
            let edited = ThreadParameters {
                length: length(3.5 * resolved.pitch_mm),
                hand: if hand == ThreadHand::Right {
                    ThreadHand::Left
                } else {
                    ThreadHand::Right
                },
                ..parameters
            };
            s.commit(
                model::prepare_thread_edit(&s, s.revision(), actor(), shaft.feature, &edited)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
            let updated = compile(&s);
            assert!(
                (updated.solid(shaft.feature).unwrap().volume_mm3().unwrap() - volume * 3.5 / 4.)
                    .abs()
                    < 1e-7
            );
            assert!(matches!(
                references::resolve(
                    &intent,
                    &updated.snapshot().shapes[&shaft.feature].topology,
                    s.revision()
                ),
                Resolution::Resolved { .. }
            ));
            let after = s.document().clone();
            s.undo(s.revision(), actor()).unwrap();
            assert_eq!(s.document(), &before);
            s.redo(s.revision(), actor()).unwrap();
            assert_eq!(s.document(), &after);
            let mut bytes = vec![];
            project_file::write(
                &ProjectFile::new(after.clone()).unwrap(),
                &mut bytes,
                Default::default(),
            )
            .unwrap();
            let reopened = project_file::read(
                &mut bytes.as_slice(),
                Default::default(),
                &project_file::supported_capabilities(),
            )
            .unwrap()
            .project;
            assert_eq!(
                model::read_thread(&reopened.document, shaft.feature)
                    .unwrap()
                    .parameters,
                edited
            );
            let reopened = Session::new(reopened.document).unwrap();
            assert!(
                (compile(&reopened)
                    .solid(shaft.feature)
                    .unwrap()
                    .volume_mm3()
                    .unwrap()
                    - volume * 3.5 / 4.)
                    .abs()
                    < 1e-7
            );
            let invalid = ThreadParameters {
                minimum_wall: length(3.),
                ..internal
            };
            let candidate =
                model::prepare_thread_edit(&s, s.revision(), actor(), tap.feature, &invalid)
                    .unwrap();
            let failure = Compiler {
                omit_display: true,
                ..Default::default()
            }
            .compile_with_backend(
                candidate.prepared.document(),
                s.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .err()
            .expect("an insufficient receiver wall must fail before commit");
            assert_eq!(failure.entity, Some(tap.feature));
            assert!(
                failure.message.to_ascii_lowercase().contains("wall"),
                "{failure}"
            );
            assert_eq!(s.document(), &after);
        }
    }
}
