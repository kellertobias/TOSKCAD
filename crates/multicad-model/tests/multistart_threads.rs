use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Link, Mutation},
    model::{self, ThreadHand, ThreadRequest, length},
    project_file::{self, ProjectFile},
    references::{self, Attachment, GeometryIntent, Origin, Resolution, TopologyKind},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
#[test]
fn saved_sourced_lead_screw_starts_keep_cap_datums_history_and_verified_exchange() {
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    for row in [
        model::lead_screw_presets()[0].clone(),
        model::lead_screw_presets()[2].clone(),
    ] {
        let mut s =
            Session::new(Document::new("Saved multiple-start lead screw").unwrap()).unwrap();
        let mut p = row.parameters();
        p.starts = 2;
        p.hand = ThreadHand::Left;
        p.length = length(3.37 * row.pitch_mm);
        let request = ThreadRequest::new("Sourced lead screw", p.clone());
        s.commit(
            model::prepare_thread(&s, 0, actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let initial = compiler
            .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
            .unwrap();
        let topology = &initial.snapshot().shapes[&request.feature].topology;
        let cap = topology
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == 0)
            .unwrap();
        let mut intent = GeometryIntent::capture(topology, cap.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: Origin {
                source: request.feature,
                role: "cap.start".into(),
            },
            offset_mm: [0.25, -0.5, 0.75],
            angle_rad: 0.3,
        });
        let datum = Entity::new(
            "Saved cap datum",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(Link {
                    target: request.feature,
                    role: "face_attachment".into(),
                    geometry: Some(intent.clone()),
                }),
            },
        );
        let datum_id = datum.id;
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Attach cap datum".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();
        let before = s.document().clone();
        let mut edited = p.clone();
        edited.starts = 3;
        edited.hand = ThreadHand::Right;
        s.commit(
            model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &edited)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let after = s.document().clone();
        s.undo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &after);
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(s.document().clone()).unwrap(),
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
        assert!(reopened.document.get(datum_id).is_some());
        let saved = model::read_thread(&reopened.document, request.feature).unwrap();
        assert_eq!(saved.parameters.family, row.family);
        assert_eq!(saved.parameters.starts, 3);
        assert_eq!(saved.parameters.hand, ThreadHand::Right);
        let model = compiler
            .compile_with_backend(&reopened.document, 0, GeometryBackend::Rust, None)
            .unwrap();
        assert!(model.snapshot().references.all_resolved());
        let solid = model.solid(request.feature).unwrap();
        assert!(matches!(
            references::resolve(
                &intent,
                &model.snapshot().shapes[&request.feature].topology,
                0
            ),
            Resolution::Resolved { frame: Some(_), .. }
        ));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lead.step");
        solid.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 12);
        let imported =
            multicad_kernel::Solid::import_rust_step(&text, request.feature, Default::default())
                .unwrap();
        assert!((imported.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-5);
        let mut invalid = edited;
        invalid.starts = 9;
        assert!(
            model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &invalid)
                .is_err()
        );
        assert_eq!(s.document(), &after);
    }
}
