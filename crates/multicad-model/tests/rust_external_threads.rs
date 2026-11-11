use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Link, Mutation},
    model::{
        self, BodyInput, PrimitiveParameters, PrimitiveRequest, ThreadFamily, ThreadFit,
        ThreadHand, ThreadMode, ThreadParameters, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
    references::{self, Attachment, GeometryIntent, Resolution, TopologyKind},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(d: &Document, rev: u64) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(d, rev, GeometryBackend::Rust, None)
    .unwrap()
}
#[test]
fn modeled_external_cut_keeps_fitted_family_stock_history_and_reopen() {
    for (family, fit, diameter, pitch) in [
        (ThreadFamily::IsoMetric, ThreadFit::Metric6gExternal, 6., 1.),
        (ThreadFamily::Unc, ThreadFit::Unified2A, 6.35, 1.27),
        (ThreadFamily::Unf, ThreadFit::Unified2A, 6.35, 25.4 / 28.),
    ] {
        let mut session =
            Session::new(Document::new("Receiving external thread").unwrap()).unwrap();
        let stock = PrimitiveRequest::new(
            "Shaft",
            PrimitiveParameters::Cylinder {
                radius: length(diameter / 2.),
                height: length(8.),
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&session, 0, actor(), &stock).unwrap();
        session.commit(prepared.prepared).unwrap();
        let original = session.document().clone();
        let params = ThreadParameters {
            family,
            fit,
            major_diameter: length(diameter),
            pitch: length(pitch),
            length: length(3.75),
            hand: ThreadHand::Left,
            ..Default::default()
        };
        let req = ThreadRequest::receiving(
            "Fitted shaft",
            ThreadMode::CutExternal,
            BodyInput {
                body: stock.body,
                producer: stock.feature,
            },
            params.clone(),
        )
        .unwrap();
        let prepared = model::prepare_thread(&session, 1, actor(), &req).unwrap();
        let compiled = compile(prepared.prepared.document(), 2);
        let solid = compiled.solid(req.feature).unwrap();
        assert!(solid.uses_rust_geometry());
        let intervals = solid.rust_receiving_thread_intervals().unwrap();
        assert_eq!(intervals.physical_thread_length_mm, 3.75);
        assert_eq!(intervals.stock_length_mm, 8.);
        let snapshot = &compiled.snapshot().shapes[&req.feature].topology;
        let cap = snapshot
            .elements
            .iter()
            .find(|e| {
                e.key.kind == TopologyKind::Face
                    && e.frames.iter().any(|f| f.frame.origin_mm[2] == 8.)
            })
            .unwrap();
        let intent = GeometryIntent::capture(snapshot, cap.key).unwrap();
        assert_eq!(session.document(), &original);
        session.commit(prepared.prepared).unwrap();
        let anchor = cap
            .frames
            .iter()
            .find(|f| f.origin.source == stock.feature)
            .unwrap()
            .origin
            .clone();
        let mut attached = intent.clone();
        attached.origins = std::collections::BTreeSet::from([anchor.clone()]);
        attached.boundary = None;
        attached.attachment = Some(Attachment {
            anchor,
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        session
            .apply(Request {
                expected_revision: 2,
                actor: actor(),
                label: "Attach preserved stock far cap".into(),
                edits: vec![Mutation::Insert(Entity::new(
                    "Shaft far-cap datum",
                    EntityData::Plane {
                        origin_mm: [0.; 3],
                        normal: [0., 0., 1.],
                        anchor: Some(Link {
                            target: req.feature,
                            role: "face_attachment".into(),
                            geometry: Some(attached),
                        }),
                    },
                ))],
            })
            .unwrap();
        compile(session.document(), 3);
        let first = session.document().clone();
        let edited = ThreadParameters {
            length: length(5.),
            ..params
        };
        let prepared =
            model::prepare_thread_edit(&session, 3, actor(), req.feature, &edited).unwrap();
        let compiled = compile(prepared.prepared.document(), 4);
        let snapshot = &compiled.snapshot().shapes[&req.feature].topology;
        assert!(matches!(
            references::resolve(&intent, snapshot, 4),
            Resolution::Resolved { .. }
        ));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shaft.step");
        compiled
            .solid(req.feature)
            .unwrap()
            .write_step(&file)
            .unwrap();
        let imported = multicad_kernel::Solid::import_rust_step(
            &std::fs::read_to_string(file).unwrap(),
            multicad_core::document::EntityId::new(),
            Default::default(),
        )
        .unwrap();
        assert!(
            (imported.volume_mm3().unwrap()
                - compiled.solid(req.feature).unwrap().volume_mm3().unwrap())
            .abs()
                < 1e-6
        );
        session.commit(prepared.prepared).unwrap();
        let saved = session.document().clone();
        session.undo(4, actor()).unwrap();
        assert_eq!(session.document(), &first);
        session.redo(5, actor()).unwrap();
        assert_eq!(session.document(), &saved);
        let mut bytes = Vec::new();
        project_file::write(
            &ProjectFile::new(saved).unwrap(),
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
        let source = model::read_thread(&reopened.document, req.feature).unwrap();
        assert_eq!(source.parameters, edited);
        assert_eq!(source.target.unwrap().producer, stock.feature);
        assert_eq!(source.mode, ThreadMode::CutExternal);
        assert_eq!(
            compile(&reopened.document, 0).snapshot().bodies[&stock.body],
            req.feature
        );
    }
}
