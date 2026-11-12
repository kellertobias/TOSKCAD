use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{
        self, ThreadFamily, ThreadFit, ThreadHand, ThreadMode, ThreadParameters, ThreadRequest,
        length,
    },
    project_file::{self, ProjectFile},
    references::{Attachment, GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(s: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
    .unwrap()
}
fn params(family: ThreadFamily, left: bool) -> ThreadParameters {
    let mut p = if family == ThreadFamily::Round {
        model::round_thread_presets()[1].parameters()
    } else {
        let row = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        ThreadParameters {
            family,
            major_diameter: length(row.diameter_mm),
            pitch: length(row.pitch_mm),
            fit: if family == ThreadFamily::Bspp {
                ThreadFit::BsppClassBExternal
            } else {
                ThreadFit::WhitworthMediumExternal
            },
            ..Default::default()
        }
    };
    p.hand = if left {
        ThreadHand::Left
    } else {
        ThreadHand::Right
    };
    if left {
        p.frame = model::PrimitiveFrame {
            origin: [17., -8., 13.].map(length),
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        };
    }
    let s = Session::new(Document::new("resolve rounded preset").unwrap()).unwrap();
    p.length = length(
        p.resolve(s.parameters(), ThreadMode::NewExternal)
            .unwrap()
            .pitch_mm
            * 2.37,
    );
    p
}
#[test]
fn normal_commands_keep_real_family_fit_units_references_edit_history_and_reopen() {
    for family in [
        ThreadFamily::Bsw,
        ThreadFamily::Bsf,
        ThreadFamily::Bspp,
        ThreadFamily::Round,
    ] {
        for left in [false, true] {
            let mut s = Session::new(Document::new("Rounded thread workflow").unwrap()).unwrap();
            let p = params(family, left);
            let request = ThreadRequest::new("Standalone rounded thread", p.clone());
            let before = s.document().clone();
            let prepared = model::prepare_thread(&s, 0, actor(), &request).unwrap();
            let preview = Compiler {
                omit_display: true,
                ..Default::default()
            }
            .compile_with_backend(prepared.prepared.document(), 1, GeometryBackend::Rust, None)
            .unwrap();
            assert_eq!(s.document(), &before);
            let solid = preview.solid(request.feature).unwrap();
            let data = solid.rust_rounded_thread_data().unwrap();
            let resolved = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
            assert_eq!(data.thread.spec().major_radius_mm, resolved.major_radius_mm);
            assert_eq!(data.thread.spec().depth_mm, resolved.profile.depth_mm);
            assert_eq!(data.thread.spec().length_mm, resolved.length_mm);
            assert_eq!(data.thread.spec().left_hand, left);
            let topology = &preview.snapshot().shapes[&request.feature].topology;
            let cap = topology
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face
                        && e.origins.iter().any(|o| o.role == "cap.end")
                })
                .unwrap();
            let intent = GeometryIntent::capture(topology, cap.key).unwrap();
            let first = solid.volume_mm3().unwrap();
            s.commit(prepared.prepared).unwrap();
            let mut attached = intent.clone();
            attached.attachment = Some(Attachment {
                anchor: cap.frames[0].origin.clone(),
                offset_mm: [0.2, -0.3, 0.4],
                angle_rad: 0.37,
            });
            s.apply(Request {
                expected_revision: s.revision(),
                actor: actor(),
                label: "Attach rounded cap datum".into(),
                edits: vec![Mutation::Insert(Entity::new(
                    "Rounded cap datum",
                    EntityData::Plane {
                        origin_mm: [0.; 3],
                        normal: [0., 0., 1.],
                        anchor: Some(Link {
                            target: request.feature,
                            role: "face_attachment".into(),
                            geometry: Some(attached),
                        }),
                    },
                ))],
            })
            .unwrap();
            compile(&s);
            let initial = s.document().clone();
            let mut edited = p.clone();
            edited.length = length(resolved.pitch_mm * 3.43);
            let prepared =
                model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &edited)
                    .unwrap();
            s.commit(prepared.prepared).unwrap();
            let model = compile(&s);
            let solid = model.solid(request.feature).unwrap();
            assert!(solid.volume_mm3().unwrap() > first);
            assert!(matches!(
                resolve(
                    &intent,
                    &model.snapshot().shapes[&request.feature].topology,
                    s.revision()
                ),
                Resolution::Resolved { .. }
            ));
            let saved = s.document().clone();
            s.undo(s.revision(), actor()).unwrap();
            assert_eq!(s.document(), &initial);
            s.redo(s.revision(), actor()).unwrap();
            assert_eq!(s.document(), &saved);
            let definition = model::read_thread(s.document(), request.feature).unwrap();
            assert_eq!(definition.parameters.family, family);
            assert_eq!(definition.parameters.fit, p.fit);
            assert_eq!(definition.parameters.pitch, p.pitch);
            let mut bytes = vec![];
            project_file::write(
                &ProjectFile::new(saved).unwrap(),
                &mut bytes,
                Default::default(),
            )
            .unwrap();
            let doc = project_file::read(
                &mut bytes.as_slice(),
                Default::default(),
                &project_file::supported_capabilities(),
            )
            .unwrap()
            .project
            .document;
            let reopened = Session::new(doc).unwrap();
            let model = compile(&reopened);
            let solid = model.solid(request.feature).unwrap();
            assert!(solid.uses_rust_geometry());
            assert_eq!(solid.faces().unwrap().len(), 7);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("rounded.step");
            solid.write_step(&path).unwrap();
            let file = std::fs::read_to_string(path).unwrap();
            let imported = multicad_kernel::Solid::import_rust_step(
                &file,
                multicad_core::document::EntityId::new(),
                Default::default(),
            )
            .unwrap();
            assert!(imported.rust_rounded_thread_data().is_some());
            assert!((imported.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-6);
        }
    }
}
#[test]
fn rounded_mode_availability_keeps_multistart_and_external_receiving_unqualified() {
    let s = Session::new(Document::new("Rounded atomic refusals").unwrap()).unwrap();
    let before = s.document().clone();
    let mut p = params(ThreadFamily::Bsw, false);
    p.starts = 2;
    let request = ThreadRequest::new("unsupported multi", p);
    assert!(model::prepare_thread(&s, 0, actor(), &request).is_err());
    assert_eq!(s.document(), &before);
    for family in [
        ThreadFamily::Bsw,
        ThreadFamily::Bsf,
        ThreadFamily::Bspp,
        ThreadFamily::Round,
    ] {
        assert!(multicad_model::rust_thread_mode_supported(
            family,
            ThreadMode::NewExternal
        ));
        assert!(multicad_model::rust_thread_mode_supported(
            family,
            ThreadMode::CutInternal
        ));
        assert!(!multicad_model::rust_thread_mode_supported(
            family,
            ThreadMode::CutExternal
        ));
    }
}

#[test]
fn unqualified_partial_and_external_receiving_tools_refuse_real_stock_atomically() {
    use model::{BodyInput, PrimitiveParameters, PrimitiveRequest};
    for family in [
        ThreadFamily::Bsw,
        ThreadFamily::Bsf,
        ThreadFamily::Bspp,
        ThreadFamily::Round,
    ] {
        let mut s = Session::new(Document::new("Rounded receiving refusal").unwrap()).unwrap();
        let p = params(family, false);
        let resolved = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
        let stock = PrimitiveRequest::new(
            "Receiving stock",
            PrimitiveParameters::Cylinder {
                radius: length(resolved.major_radius_mm + 2.),
                height: length(10. * resolved.pitch_mm),
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&s, 0, actor(), &stock).unwrap();
        s.commit(prepared.prepared).unwrap();
        let before = s.document().clone();
        let before_model = compile(&s);
        let before_volume = before_model
            .solid(stock.feature)
            .unwrap()
            .volume_mm3()
            .unwrap();
        for mode in [ThreadMode::CutInternal, ThreadMode::CutExternal] {
            let mut p = p.clone();
            if mode == ThreadMode::CutInternal {
                p.fit = match family {
                    ThreadFamily::Bspp => ThreadFit::BsppInternal,
                    ThreadFamily::Round => ThreadFit::NominalBasic,
                    _ => ThreadFit::WhitworthMediumInternal,
                };
            }
            let request = ThreadRequest::receiving(
                "Unsupported rounded receiver",
                mode,
                BodyInput {
                    body: stock.body,
                    producer: stock.feature,
                },
                p,
            )
            .unwrap();
            let prepared = model::prepare_thread(&s, s.revision(), actor(), &request).unwrap();
            let err = Compiler {
                omit_display: true,
                ..Default::default()
            }
            .compile_with_backend(
                prepared.prepared.document(),
                s.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .err()
            .unwrap();
            assert!(
                err.to_string().contains("not certified"),
                "{family:?} {mode:?}: {err}"
            );
            assert_eq!(s.document(), &before);
            assert_eq!(s.revision(), 1);
            assert_eq!(
                compile(&s)
                    .solid(stock.feature)
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
                before_volume
            );
        }
    }
}

#[test]
fn rounded_full_through_receivers_keep_canonical_internal_profiles_history_and_reopen() {
    use model::{BodyInput, PrimitiveParameters, PrimitiveRequest};
    for family in [
        ThreadFamily::Bsw,
        ThreadFamily::Bsf,
        ThreadFamily::Bspp,
        ThreadFamily::Round,
    ] {
        let mut s = Session::new(Document::new("Rounded through receiver").unwrap()).unwrap();
        let mut p = params(family, true);
        p.fit = match family {
            ThreadFamily::Bspp => ThreadFit::BsppInternal,
            ThreadFamily::Round => ThreadFit::NominalBasic,
            _ => ThreadFit::WhitworthMediumInternal,
        };
        let resolved = p.resolve(s.parameters(), ThreadMode::CutInternal).unwrap();
        let stock = PrimitiveRequest::new(
            "Receiving cylinder",
            PrimitiveParameters::Cylinder {
                radius: length(resolved.major_radius_mm + 2.),
                height: p.length.clone(),
                frame: p.frame.clone(),
            },
        );
        let prepared = model::prepare_primitive(&s, 0, actor(), &stock).unwrap();
        s.commit(prepared.prepared).unwrap();
        let stock_model = compile(&s);
        let before_volume = stock_model
            .solid(stock.feature)
            .unwrap()
            .volume_mm3()
            .unwrap();
        let request = ThreadRequest::receiving(
            "Internal rounded thread",
            ThreadMode::CutInternal,
            BodyInput {
                body: stock.body,
                producer: stock.feature,
            },
            p.clone(),
        )
        .unwrap();
        let before = s.document().clone();
        let prepared = model::prepare_thread(&s, s.revision(), actor(), &request).unwrap();
        let preview = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(prepared.prepared.document(), 2, GeometryBackend::Rust, None)
        .unwrap();
        assert_eq!(s.document(), &before);
        let data = preview
            .solid(request.feature)
            .unwrap()
            .rust_rounded_thread_bore_data()
            .unwrap();
        assert_eq!(
            data.bore.thread().spec().major_radius_mm,
            resolved.major_radius_mm
        );
        if let Some(round) = resolved.round {
            assert_eq!(
                data.bore.thread().spec().crest_radius_mm,
                round.cutter_crest_radius_mm
            );
            assert_eq!(
                data.bore.thread().spec().root_radius_mm,
                round.cutter_root_radius_mm
            );
        }
        s.commit(prepared.prepared).unwrap();
        let first = compile(&s);
        let solid = first.solid(request.feature).unwrap();
        assert!(solid.volume_mm3().unwrap() < before_volume);
        let top = &first.snapshot().shapes[&request.feature].topology;
        let cap = top
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == 0)
            .unwrap();
        let intent = GeometryIntent::capture(top, cap.key).unwrap();
        let original = s.document().clone();
        let mut edited = p.clone();
        edited.hand = ThreadHand::Right;
        let prepared =
            model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &edited)
                .unwrap();
        s.commit(prepared.prepared).unwrap();
        let next = compile(&s);
        assert!(
            !next
                .solid(request.feature)
                .unwrap()
                .rust_rounded_thread_bore_data()
                .unwrap()
                .bore
                .thread()
                .spec()
                .left_hand
        );
        assert!(matches!(
            resolve(
                &intent,
                &next.snapshot().shapes[&request.feature].topology,
                s.revision()
            ),
            Resolution::Resolved { .. }
        ));
        let saved = s.document().clone();
        s.undo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &original);
        s.redo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &saved);
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(saved).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = Session::new(
            project_file::read(
                &mut bytes.as_slice(),
                Default::default(),
                &project_file::supported_capabilities(),
            )
            .unwrap()
            .project
            .document,
        )
        .unwrap();
        let model = compile(&reopened);
        let solid = model.solid(request.feature).unwrap();
        assert_eq!(solid.faces().unwrap().len(), 8);
        let dir = tempfile::tempdir().unwrap();
        solid.write_step(&dir.path().join("through.step")).unwrap();
        let file = std::fs::read_to_string(dir.path().join("through.step")).unwrap();
        let imported =
            multicad_kernel::Solid::import_rust_step(&file, EntityId::new(), Default::default())
                .unwrap();
        assert!(imported.rust_rounded_thread_bore_data().is_some());
        imported.write_step(&dir.path().join("again.step")).unwrap();
    }
}
