//! Native STEP and KiCad reading for the window's import Inspector. The UI
//! calls these on its own worker threads; the live Session is never touched.
use multicad_core::{
    document::Document,
    model::imports::{KicadImport, StepFile},
};
use multicad_model::{
    Compiler, GeometryBackend,
    imports::{KicadOptions, read_kicad_file, read_step_file_with_backend},
};
use multicad_ui::mechanical_import::MechanicalImportBackend;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub struct Backend {
    geometry_backend: GeometryBackend,
}

impl Backend {
    pub fn with_geometry_backend(geometry_backend: GeometryBackend) -> Self {
        Self { geometry_backend }
    }
}

impl MechanicalImportBackend for Backend {
    fn read_step(&self, path: &Path) -> Result<StepFile, String> {
        read_step_file_with_backend(path, self.geometry_backend).map_err(|e| e.to_string())
    }
    /// The user chose the board in the native dialog, so its footprint models
    /// are looked up wherever KiCad would find them (`${KIPRJMOD}`, the
    /// environment), not only in one folder as for MCP.
    fn read_kicad(&self, path: &Path, min_hole_mm: f64) -> Result<KicadImport, String> {
        if self.geometry_backend == GeometryBackend::Rust {
            return Err("KiCad import is unavailable with the Rust geometry backend".into());
        }
        let options = KicadOptions {
            min_hole_mm,
            ..KicadOptions::default()
        };
        read_kicad_file(path, &options).map_err(|e| e.to_string())
    }
    fn verify(
        &self,
        document: &Document,
        revision: u64,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let interrupt = || cancel.load(Ordering::Relaxed);
        Compiler::default()
            .compile_with_backend(document, revision, self.geometry_backend, Some(&interrupt))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::Session,
        model::imports::{StepEncoding, read_step_source, step_bodies, step_sources},
        project_file::ProjectFile,
    };
    use multicad_kernel::rust_kernel::{Rational, step::write_box};
    use multicad_ui::{
        mechanical_import::{MechanicalImport, Outcome, Report, SourceKind, Target},
        storage::{DocumentStorage, SaveMode},
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    fn step_box(length: i64) -> String {
        write_box(
            [length, 8, 3].map(|mm| Rational::new(mm, 1).unwrap()),
            [5., -2., 4.],
            [0., 1., 0.],
            [0., 0., 1.],
        )
        .unwrap()
    }

    fn check_import(session: &Session, path: &Path, target: Target) -> MechanicalImport {
        let mut import = MechanicalImport::default();
        import
            .begin(
                Arc::new(Backend::with_geometry_backend(GeometryBackend::Rust)),
                SourceKind::Step,
                path.to_owned(),
                target,
            )
            .unwrap();
        let end = Instant::now() + Duration::from_secs(15);
        while import.pending() {
            assert!(Instant::now() < end, "STEP import timed out");
            import.poll(session);
            std::thread::sleep(Duration::from_millis(2));
        }
        import
    }

    #[test]
    fn rust_step_import_and_update_survive_project_reopen_without_the_source_file() {
        let scratch = tempfile::tempdir().unwrap();
        let step = scratch.path().join("stock.step");
        std::fs::write(&step, step_box(12)).unwrap();
        let mut session = Session::new(Document::new("STEP import").unwrap()).unwrap();

        let mut import = check_import(&session, &step, Target::New);
        assert_eq!(import.report(), Some(&Report::StepNew { parts: 1 }));
        assert!(import.can_apply(&session), "{:?}", import.checked());
        assert_eq!(session.revision(), 0);
        assert!(matches!(
            import.apply(&mut session),
            Ok(Outcome::Applied { .. })
        ));
        let source = step_sources(session.document()).next().unwrap().id;
        let body = step_bodies(session.document()).next().unwrap().id;
        assert_eq!(
            read_step_source(session.document(), source)
                .unwrap()
                .encoding,
            StepEncoding::RustStep
        );

        std::fs::write(&step, step_box(16)).unwrap();
        let mut update = check_import(&session, &step, Target::Update(source));
        let Some(Report::StepUpdate {
            matched,
            added,
            missing,
        }) = update.report()
        else {
            panic!("{:?}", update.checked());
        };
        assert_eq!(matched.len(), 1);
        assert!(matched[0].2);
        assert!(added.is_empty() && missing.is_empty());
        assert_eq!(session.revision(), 1);
        assert!(matches!(
            update.apply(&mut session),
            Ok(Outcome::Applied { .. })
        ));
        assert_eq!(session.revision(), 2);

        let mut store = crate::files::ProjectStore::new(scratch.path().join("recovery"));
        let project = scratch.path().join("stock.tcd");
        let file = ProjectFile::new(session.document().clone()).unwrap();
        store
            .save_project(&project, &file, SaveMode::Create)
            .unwrap();
        std::fs::remove_file(&step).unwrap();
        let reopened = store.open_project(&project).unwrap().file.document;
        assert_eq!(
            step_bodies(&reopened)
                .map(|entity| entity.id)
                .collect::<Vec<_>>(),
            [body]
        );
        let evaluated = Compiler::default()
            .compile_with_backend(&reopened, 0, GeometryBackend::Rust, None)
            .unwrap();
        let solid = evaluated.solid(body).unwrap();
        assert!(solid.uses_rust_geometry());
        assert!((solid.volume_mm3().unwrap() - 384.).abs() < 1e-7);
        let bounds = solid.bounds().unwrap();
        assert_eq!(bounds.min, [5., -2., 4.]);
        assert_eq!(bounds.max, [8., 14., 12.]);
    }

    #[test]
    fn rust_step_tenon_reopens_with_exact_caps_after_the_source_is_deleted() {
        use multicad_core::{
            document::EntityId, manufacturing::PlanarEdge, references::TopologyKind,
        };
        use multicad_kernel::{
            GeometryPolicy, MeshOptions, PlaneFrame,
            rust_kernel::{stadium::Stadium, step::write_stadium_prism},
        };
        use std::f64::consts::PI;

        let scratch = tempfile::tempdir().unwrap();
        let source_path = scratch.path().join("tenon.step");
        let frame = PlaneFrame {
            origin: [5., -2., 4.],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        };
        let profile = Stadium::new([[0., 0.], [4., 0.]], 2.).unwrap();
        let source =
            write_stadium_prism(&profile, 8., frame.origin, frame.u, frame.v, false).unwrap();
        std::fs::write(&source_path, source).unwrap();
        let mut session = Session::new(Document::new("Rounded STEP import").unwrap()).unwrap();
        let mut import = check_import(&session, &source_path, Target::New);
        assert_eq!(
            import.report(),
            Some(&Report::StepNew { parts: 1 }),
            "{:?}",
            import.checked()
        );
        assert!(matches!(
            import.apply(&mut session),
            Ok(Outcome::Applied { .. })
        ));
        let body = step_bodies(session.document()).next().unwrap().id;
        let mut store = crate::files::ProjectStore::new(scratch.path().join("recovery"));
        let project_path = scratch.path().join("tenon.tcd");
        store
            .save_project(
                &project_path,
                &ProjectFile::new(session.document().clone()).unwrap(),
                SaveMode::Create,
            )
            .unwrap();
        std::fs::remove_file(source_path).unwrap();
        let document = store.open_project(&project_path).unwrap().file.document;
        let model = Compiler::default()
            .compile_with_backend(&document, 0, GeometryBackend::Rust, None)
            .unwrap();
        let solid = model.solid(body).unwrap();
        assert!(solid.uses_rust_geometry());
        let tolerance = GeometryPolicy::default().linear_tolerance_mm;
        assert!((solid.volume_mm3().unwrap() - profile.area_mm2() * 8.).abs() <= tolerance);
        solid
            .manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap();
        let cap = solid
            .topology_snapshot(EntityId::new(), 0)
            .unwrap()
            .elements
            .into_iter()
            .find(|item| {
                item.key.kind == TopologyKind::Face
                    && item.frames.iter().any(|cap| {
                        (cap.frame.origin_mm[0] - frame.origin[0]).abs() <= tolerance
                            && cap.frame.normal()[0].abs() > 0.99
                    })
            })
            .expect("Imported cap retained its analytic frame");
        let contours = solid.planar_face_contours(cap.key.ordinal, frame).unwrap();
        assert!((contours.area_mm2() - profile.area_mm2()).abs() <= tolerance);
        let arcs = contours.loops[0]
            .edges
            .iter()
            .filter(|edge| matches!(edge, PlanarEdge::Arc { sweep_rad, .. } if (sweep_rad.abs() - PI).abs() <= tolerance));
        assert_eq!(arcs.count(), 2);
    }

    #[test]
    fn unsupported_step_units_do_not_fall_back_to_native_or_change_the_project() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("centimetres.step");
        let step = step_box(12).replace("SI_UNIT(.MILLI.,.METRE.)", "SI_UNIT(.CENTI.,.METRE.)");
        std::fs::write(&path, step).unwrap();
        assert!(
            Backend::with_geometry_backend(GeometryBackend::OpenCascade)
                .read_step(&path)
                .is_ok()
        );

        let session = Session::new(Document::new("Unsupported STEP").unwrap()).unwrap();
        let import = check_import(&session, &path, Target::New);
        assert!(
            import
                .error()
                .is_some_and(|error| error.contains("Rust STEP")),
            "{:?}",
            import.error()
        );
        assert!(!import.can_apply(&session));
        assert_eq!(session.revision(), 0);
        assert!(step_sources(session.document()).next().is_none());

        let backend = Backend::with_geometry_backend(GeometryBackend::Rust);
        assert_eq!(
            backend
                .read_kicad(&scratch.path().join("board.kicad_pcb"), 2.)
                .unwrap_err(),
            "KiCad import is unavailable with the Rust geometry backend",
        );
    }
}
