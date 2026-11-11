//! Exact manufacturing outputs run on a bounded native owner thread.
use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::*,
};
use multicad_model::{
    GeometryBackend,
    export::{ExportWorker, Exporter},
};
use std::path::Path;
pub struct Backend {
    worker: Result<ExportWorker, String>,
}
impl Backend {
    pub fn new() -> Self {
        Self::with_geometry_backend(GeometryBackend::default())
    }
    pub fn with_geometry_backend(backend: GeometryBackend) -> Self {
        Self {
            worker: ExportWorker::with_geometry_backend(Exporter::default(), backend),
        }
    }
    fn worker(&mut self) -> Result<&mut ExportWorker, ExportError> {
        self.worker.as_mut().map_err(|e| ExportError {
            kind: ErrorKind::Unsupported,
            entity: None,
            message: e.clone(),
        })
    }
}
impl multicad_ui::exchange::ExportBackend for Backend {
    fn planar_catalog(
        &mut self,
        document: &Document,
        revision: u64,
        scope: &multicad_core::exchange::Scope,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?
            .planar_catalog(document, revision, scope, actor)
    }
    fn preview(
        &mut self,
        d: &Document,
        r: u64,
        s: EntityId,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?.preview(d, r, s, a)
    }
    fn run(
        &mut self,
        d: &Document,
        r: u64,
        k: &PreviewKey,
        p: &Path,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?.run(d, r, k, p, a)
    }
    fn schedule(
        &mut self,
        d: &Document,
        r: u64,
        spec: multicad_core::schedules::ScheduleSpec,
        target: Option<(&Path, multicad_core::project_file::ContentHash)>,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?.schedule(d, r, spec, target, a)
    }
    fn cancel(&mut self, id: EntityId) -> bool {
        self.worker.as_mut().is_ok_and(|w| w.cancel(id))
    }
    fn poll(&mut self) -> Option<ExportReply> {
        self.worker.as_mut().ok().and_then(|w| w.poll())
    }
    fn pending(&self) -> bool {
        self.worker.as_ref().is_ok_and(|w| w.pending())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{self, PrimitiveParameters, PrimitiveRequest, length},
    };
    use multicad_ui::exchange::{ExportTool, ExportWorkspace};
    use std::time::{Duration, Instant};
    fn settle(workspace: &mut ExportWorkspace, session: &Session) {
        let end = Instant::now() + Duration::from_secs(15);
        while workspace.pending() {
            assert!(Instant::now() < end, "Native export timeout");
            workspace.poll(session);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn create_step_set(workspace: &mut ExportWorkspace, session: &mut Session) {
        for tool in [ExportTool::NewSet, ExportTool::SaveSet] {
            workspace.execute(session, tool, None, Channel::Ui).unwrap();
        }
    }
    fn preview_step_set(workspace: &mut ExportWorkspace, session: &mut Session) {
        workspace
            .execute(session, ExportTool::Preview, None, Channel::Ui)
            .unwrap();
        settle(workspace, session);
    }
    #[cfg(feature = "occt")]
    #[test]
    fn native_workspace_runs_production_adapter_and_exact_step_without_mutating_source() {
        let mut session = Session::new(Document::new("Native workspace fixture").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let primitive = PrimitiveRequest::new(
            "Stock",
            PrimitiveParameters::Box {
                size: [length(12.), length(8.), length(3.)],
                frame: Default::default(),
            },
        );
        let p = model::prepare_primitive(&session, 0, actor.clone(), &primitive).unwrap();
        session.commit(p.prepared).unwrap();
        let mut workspace = ExportWorkspace::default();
        workspace.set_backend(Box::new(Backend::new())).unwrap();
        create_step_set(&mut workspace, &mut session);
        let before = session.document().clone();
        let revision = session.revision();
        preview_step_set(&mut workspace, &mut session);
        let dir = tempfile::tempdir().unwrap();
        workspace
            .execute(
                &mut session,
                ExportTool::Run,
                Some(dir.path().to_str().unwrap()),
                Channel::Ui,
            )
            .unwrap();
        settle(&mut workspace, &session);
        assert!(!workspace.writing());
        let notice = workspace.take_notice().unwrap();
        assert!(!notice.1, "{}", notice.0);
        assert!(notice.0.contains("Exported 1 file"));
        let bundle = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.is_dir())
            .unwrap();
        let step = std::fs::read_dir(&bundle)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|s| s == "step"))
            .unwrap();
        let solid =
            multicad_kernel::Solid::read_step(&step, multicad_kernel::GeometryPolicy::default())
                .unwrap();
        assert!((solid.volume_mm3().unwrap() - 288.).abs() < 1e-7);
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
    }
    #[test]
    fn rust_workspace_exports_a_placed_box_and_keeps_native_bundle_identity_separate() {
        let mut session = Session::new(Document::new("STEP backend isolation").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let box_request = PrimitiveRequest::new(
            "Placed stock",
            PrimitiveParameters::Box {
                size: [length(12.), length(8.), length(3.)],
                frame: model::PrimitiveFrame {
                    origin: [length(5.), length(-7.), length(2.)],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
            },
        );
        let prepared = model::prepare_primitive(&session, 0, actor, &box_request).unwrap();
        session.commit(prepared.prepared).unwrap();
        let mut workspace = ExportWorkspace::default();
        create_step_set(&mut workspace, &mut session);
        let before = session.document().clone();
        let revision = session.revision();
        let dir = tempfile::tempdir().unwrap();
        let backends = [
            GeometryBackend::Rust,
            #[cfg(feature = "occt")]
            GeometryBackend::OpenCascade,
        ];
        for &backend in &backends {
            workspace
                .set_backend(Box::new(Backend::with_geometry_backend(backend)))
                .unwrap();
            preview_step_set(&mut workspace, &mut session);
            workspace
                .execute(
                    &mut session,
                    ExportTool::Run,
                    Some(dir.path().to_str().unwrap()),
                    Channel::Ui,
                )
                .unwrap();
            settle(&mut workspace, &session);
            let (notice, error) = workspace.take_notice().unwrap();
            assert!(!error, "{notice}");
            assert!(notice.contains("Exported 1 file"), "{notice}");
        }
        let bundles: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect();
        assert_eq!(bundles.len(), backends.len());
        let mut rust_outputs = 0;
        for bundle in bundles {
            let manifest = std::fs::read(bundle.join("multicad-export.json")).unwrap();
            let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
            let provider = manifest["provider"].as_str().unwrap();
            let step = std::fs::read_dir(&bundle)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.extension().is_some_and(|e| e == "step"))
                .unwrap();
            let text = std::fs::read_to_string(&step).unwrap();
            if provider.starts_with("step-files.rust-analytic.1;") {
                rust_outputs += 1;
                assert!(!provider.contains("occt:"));
                assert!(text.contains("MANIFOLD_SOLID_BREP"));
                assert!(text.contains("MultiCAD analytic solid"));
                assert!(!text.contains("Open CASCADE STEP processor"));
                let readback =
                    multicad_kernel::rust_kernel::step_read::read_analytic_solid(&text, 1e-7)
                        .unwrap();
                assert_eq!(readback.solid_count, 1);
                assert_eq!(readback.bounds_mm, [[5., -7., 2.], [8., 5., 10.]]);
                assert!((readback.volume_mm3 - 288.).abs() < 1e-7);
            } else {
                assert!(provider.starts_with("step-files.1;"));
                assert!(text.contains("Open CASCADE STEP processor"));
            }
            #[cfg(feature = "occt")]
            {
                let solid = multicad_kernel::Solid::read_step(
                    &step,
                    multicad_kernel::GeometryPolicy::default(),
                )
                .unwrap();
                let bounds = solid.bounds().unwrap();
                assert_eq!(solid.solid_count().unwrap(), 1);
                for (actual, expected) in bounds
                    .min
                    .into_iter()
                    .chain(bounds.max)
                    .zip([5., -7., 2., 8., 5., 10.])
                {
                    assert!((actual - expected).abs() < 1e-7);
                }
                assert!((solid.volume_mm3().unwrap() - 288.).abs() < 1e-7);
            }
        }
        assert_eq!(rust_outputs, 1);
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
    }
    #[test]
    fn rust_workspace_exports_a_cylinder_with_analytic_faces() {
        let mut session = Session::new(Document::new("Cylindrical Rust STEP").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let cylinder = PrimitiveRequest::new(
            "Cylinder",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(6.),
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&session, 0, actor, &cylinder).unwrap();
        session.commit(prepared.prepared).unwrap();
        let mut workspace = ExportWorkspace::default();
        workspace
            .set_backend(Box::new(Backend::with_geometry_backend(
                GeometryBackend::Rust,
            )))
            .unwrap();
        create_step_set(&mut workspace, &mut session);
        let before = session.document().clone();
        preview_step_set(&mut workspace, &mut session);
        let dir = tempfile::tempdir().unwrap();
        workspace
            .execute(
                &mut session,
                ExportTool::Run,
                Some(dir.path().to_str().unwrap()),
                Channel::Ui,
            )
            .unwrap();
        settle(&mut workspace, &session);
        let (notice, error) = workspace.take_notice().unwrap();
        assert!(!error, "{notice}");
        assert!(notice.contains("Exported 1 file"), "{notice}");
        let bundle = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.is_dir())
            .unwrap();
        let path = std::fs::read_dir(&bundle)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|s| s == "step"))
            .unwrap();
        let step = std::fs::read_to_string(&path).unwrap();
        assert_eq!(step.matches("=CYLINDRICAL_SURFACE(").count(), 1);
        assert_eq!(step.matches("=CIRCLE(").count(), 2);
        let readback =
            multicad_kernel::rust_kernel::step_read::read_analytic_solid(&step, 1e-7).unwrap();
        assert_eq!(readback.solid_count, 1);
        assert!((readback.volume_mm3 - 150. * std::f64::consts::PI).abs() < 1e-6);
        #[cfg(feature = "occt")]
        {
            let independent = multicad_kernel::Solid::read_step(&path, Default::default()).unwrap();
            assert!((independent.volume_mm3().unwrap() - readback.volume_mm3).abs() < 1e-6);
        }
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn rust_workspace_publishes_exact_round_and_rounded_pockets() {
        use multicad_kernel::{
            CircularPocketTool, GeometryPolicy, PlaneFrame, RoundedSlotTool, Solid,
            rust_kernel::{Rational, step_read},
        };

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("machined-panel.step");
        let policy = GeometryPolicy::default();
        let feature = EntityId::new();
        let origin = |role: &str| multicad_core::references::Origin {
            source: feature,
            role: role.into(),
        };
        let frame = PlaneFrame {
            origin: [0., 0., 13.],
            ..Default::default()
        };
        let stock = Solid::rust_box(
            feature.uuid_bits(),
            [20, 16, 12].map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame::default(),
            policy,
        )
        .unwrap()
        .rust_drill_rounded_slots(
            &[RoundedSlotTool {
                frame,
                centers_uv: [[5., 6.], [9., 6.]],
                radius_mm: 2.,
                displacement_mm: [0., 0., -5.],
                origin: origin("mortise"),
            }],
            policy,
        )
        .unwrap()
        .rust_drill_circular_pockets(
            &[CircularPocketTool {
                frame,
                center_uv: [14., 10.],
                radius_mm: 1.,
                displacement_mm: [0., 0., -4.],
                origin: origin("drill"),
            }],
            policy,
        )
        .unwrap();
        stock.write_step(&source).unwrap();
        let file =
            multicad_model::imports::read_step_file_with_backend(&source, GeometryBackend::Rust)
                .unwrap();
        let mut session = Session::new(Document::new("Rust STEP publication").unwrap()).unwrap();
        let import = model::imports::prepare_step_import(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &file,
        )
        .unwrap();
        session.commit(import.model.prepared).unwrap();
        let mut workspace = ExportWorkspace::default();
        workspace
            .set_backend(Box::new(Backend::with_geometry_backend(
                GeometryBackend::Rust,
            )))
            .unwrap();
        create_step_set(&mut workspace, &mut session);
        preview_step_set(&mut workspace, &mut session);
        let output = tempfile::tempdir().unwrap();
        workspace
            .execute(
                &mut session,
                ExportTool::Run,
                output.path().to_str(),
                Channel::Ui,
            )
            .unwrap();
        settle(&mut workspace, &session);
        let (notice, error) = workspace.take_notice().unwrap();
        assert!(!error && notice.contains("Exported 1 file"), "{notice}");
        let bundle = std::fs::read_dir(output.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir())
            .unwrap();
        let published = std::fs::read_dir(bundle)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "step"))
            .unwrap();
        let text = std::fs::read_to_string(&published).unwrap();
        let readback = step_read::read_analytic_solid(&text, policy.linear_tolerance_mm).unwrap();
        assert_eq!(readback.semicircles.len(), 4);
        assert_eq!(readback.circles.len(), 2);
        assert!((readback.volume_mm3 - stock.volume_mm3().unwrap()).abs() < 1e-6);
        #[cfg(feature = "occt")]
        {
            let independent = Solid::read_step(&published, policy).unwrap();
            assert!((independent.volume_mm3().unwrap() - readback.volume_mm3).abs() < 1e-6);
        }
    }

    #[test]
    fn rust_workspace_requests_exact_planar_faces_through_the_export_backend() {
        use multicad_ui::exchange::ExportBackend;
        let mut session = Session::new(Document::new("Rust planar catalog").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let cylinder = PrimitiveRequest::new(
            "Cylinder",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(6.),
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&session, 0, actor.clone(), &cylinder).unwrap();
        session.commit(prepared.prepared).unwrap();
        let before = session.document().clone();
        let revision = session.revision();
        let mut backend = Backend::with_geometry_backend(GeometryBackend::Rust);
        let request = backend
            .planar_catalog(session.document(), revision, &Scope::default(), actor)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let reply = loop {
            if let Some(reply) = backend.poll() {
                break reply;
            }
            assert!(
                Instant::now() < deadline,
                "Planar face catalog did not arrive"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(reply.id, request);
        let ExportResult::PlanarCatalog(catalog) = reply.result else {
            panic!("expected the planar face catalog");
        };
        let catalog = catalog.unwrap();
        let [part] = catalog.parts.as_slice() else {
            panic!("expected the current cylinder");
        };
        assert_eq!(part.subject.part, cylinder.body);
        assert_eq!(part.faces.len(), 2);
        assert!(part.faces.iter().all(|face| {
            face.source == cylinder.body
                && face
                    .intent
                    .origins
                    .iter()
                    .any(|origin| origin.source == cylinder.feature)
        }));
        assert!(
            part.faces
                .iter()
                .any(|face| { face.plane.origin_mm[2] == 6. && face.plane.normal()[2] > 0. })
        );
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
    }
    #[cfg(feature = "occt")]
    #[test]
    fn native_schedule_csv_writes_exactly_the_model_bom_off_the_ui_thread() {
        schedule_csv_writes_exactly_the_model_bom_off_the_ui_thread(GeometryBackend::OpenCascade);
    }
    #[test]
    fn rust_schedule_csv_writes_exactly_the_model_bom_off_the_ui_thread() {
        schedule_csv_writes_exactly_the_model_bom_off_the_ui_thread(GeometryBackend::Rust);
    }
    fn schedule_csv_writes_exactly_the_model_bom_off_the_ui_thread(backend: GeometryBackend) {
        let mut session = Session::new(Document::new("Shop CSV").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        for name in ["Leg", "Top"] {
            let primitive = PrimitiveRequest::new(
                name,
                PrimitiveParameters::Box {
                    size: [length(12.), length(8.), length(3.)],
                    frame: Default::default(),
                },
            );
            let p =
                model::prepare_primitive(&session, session.revision(), actor.clone(), &primitive)
                    .unwrap();
            session.commit(p.prepared).unwrap();
        }
        let mut workspace = ExportWorkspace::default();
        workspace
            .set_backend(Box::new(Backend::with_geometry_backend(backend)))
            .unwrap();
        workspace
            .execute(&mut session, ExportTool::ScheduleCsv, None, Channel::Ui)
            .unwrap();
        settle(&mut workspace, &session);
        let preview = workspace.schedule_preview(&session).unwrap().clone();
        let model = multicad_model::Compiler::default()
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap();
        let expected = multicad_model::schedules::build(
            &model,
            session.document(),
            session.revision(),
            workspace.schedule_spec(),
        )
        .unwrap()
        .csv();
        assert_eq!(preview.csv, expected);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shop_bom.csv");
        let before = session.document().clone();
        workspace
            .write_schedule(&session, &path, Channel::Ui)
            .unwrap();
        assert!(workspace.writing());
        settle(&mut workspace, &session);
        let notice = workspace.take_notice().unwrap();
        assert!(!notice.1, "{}", notice.0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert!(expected.contains("\n1,Leg,1,") && expected.contains("\n2,Top,1,"));
        assert_eq!(session.document(), &before, "the source is unchanged");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
