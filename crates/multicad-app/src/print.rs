//! Print packages run on their own bounded geometry owner thread.
use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::*,
};
use multicad_model::{
    GeometryBackend,
    export::{Exporter, PrintWorker},
};
use std::path::Path;
pub struct Backend {
    worker: Result<PrintWorker, String>,
}
impl Backend {
    pub fn new() -> Self {
        Self {
            worker: PrintWorker::new(Exporter::default()),
        }
    }
    pub fn with_geometry_backend(backend: GeometryBackend) -> Self {
        Self {
            worker: PrintWorker::with_geometry_backend(Exporter::default(), backend),
        }
    }
    fn worker(&mut self) -> Result<&mut PrintWorker, ExportError> {
        self.worker
            .as_mut()
            .map_err(|e| ExportError::new(ErrorKind::Unsupported, None, e.clone()))
    }
}
impl multicad_ui::print::PrintBackend for Backend {
    fn preview(
        &mut self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?.preview(document, revision, request, actor)
    }
    fn write(
        &mut self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?
            .write(document, revision, request, expected, path, actor)
    }
    fn preview_plate(
        &mut self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?
            .preview_plate(document, revision, plate, actor)
    }
    fn write_plate(
        &mut self,
        document: &Document,
        revision: u64,
        plate: &PrintPlateDefinition,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.worker()?
            .write_plate(document, revision, plate, expected, path, actor)
    }
    fn cancel(&mut self, id: EntityId) -> bool {
        self.worker.as_mut().is_ok_and(|w| w.cancel(id))
    }
    fn poll(&mut self) -> Option<PrintReply> {
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
        commands::{Channel, Session},
        model::{self, PrimitiveParameters, PrimitiveRequest, length},
    };
    use multicad_ui::print::{PrintTool, PrintWorkspace};
    use std::time::{Duration, Instant};
    fn settle(workspace: &mut PrintWorkspace, session: &Session) {
        let end = Instant::now() + Duration::from_secs(15);
        while workspace.pending() {
            assert!(Instant::now() < end, "Native print timeout");
            workspace.poll(session);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn native_workspace_writes_a_3mf_that_reads_back_without_mutating_source() {
        workspace_writes_a_3mf_that_reads_back_without_mutating_source(Backend::new());
    }

    #[test]
    fn rust_workspace_writes_a_3mf_that_reads_back_without_mutating_source() {
        workspace_writes_a_3mf_that_reads_back_without_mutating_source(
            Backend::with_geometry_backend(GeometryBackend::Rust),
        );
    }

    fn workspace_writes_a_3mf_that_reads_back_without_mutating_source(backend: Backend) {
        let mut session = Session::new(Document::new("Print fixture").unwrap()).unwrap();
        let primitive = PrimitiveRequest::new(
            "Stock",
            PrimitiveParameters::Box {
                size: [length(12.), length(8.), length(3.)],
                frame: Default::default(),
            },
        );
        let p =
            model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &primitive).unwrap();
        session.commit(p.prepared).unwrap();
        // A placed copy 20 mm along x is packaged as its own build item.
        let mut copy = model::ComponentPlacementRequest::new(
            primitive.body,
            model::NumericTransform {
                translation: [length(20.), length(0.), length(0.)],
                ..Default::default()
            },
        );
        copy.name = "Copy".into();
        let p = model::prepare_place_component(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &copy,
        )
        .unwrap();
        session.commit(p.prepared).unwrap();
        let mut workspace = PrintWorkspace::default();
        workspace.set_backend(Box::new(backend)).unwrap();
        workspace.capture(vec![primitive.body, copy.instance]);
        workspace.set_stl_per_part(true);
        let before = session.document().clone();
        workspace.request_preview(&session, Channel::Ui).unwrap();
        settle(&mut workspace, &session);
        assert!(
            workspace.enabled(PrintTool::Write, &session),
            "{:?}",
            workspace.error()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stock.3mf");
        workspace.write(&session, &path, Channel::Ui).unwrap();
        settle(&mut workspace, &session);
        let (notice, error) = workspace.take_notice().unwrap();
        assert!(!error, "{notice}");
        assert!(notice.contains("Wrote 3 file(s)"), "{notice}");
        let read =
            multicad_model::export::mesh::read_three_mf(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            (read.objects, read.items),
            (1, 2),
            "the copy shares the mesh"
        );
        assert!((read.volume_mm3() - 576.).abs() < 1e-6);
        assert!((read.bounds_mm().unwrap()[1][0] - 32.).abs() < 1e-6);
        let stl = multicad_model::export::mesh::read_stl(
            &std::fs::read(dir.path().join("stock-Stock.stl")).unwrap(),
        )
        .unwrap();
        assert!((stl.volume_mm3() - 288.).abs() < 1e-3);
        let copy_stl = multicad_model::export::mesh::read_stl(
            &std::fs::read(dir.path().join("stock-Stock (Copy).stl")).unwrap(),
        )
        .unwrap();
        assert!((copy_stl.bounds_mm().unwrap()[0][0] - 20.).abs() < 1e-4);
        assert_eq!(session.document(), &before);
    }
}
