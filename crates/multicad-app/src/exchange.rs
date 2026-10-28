//! Exact manufacturing outputs run on a bounded native owner thread.
use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::*,
};
use multicad_model::export::{ExportWorker, Exporter};
use std::path::Path;
pub struct Backend {
    worker: Result<ExportWorker, String>,
}
impl Backend {
    pub fn new() -> Self {
        Self {
            worker: ExportWorker::new(Exporter::default()),
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
        for tool in [ExportTool::NewSet, ExportTool::SaveSet] {
            workspace
                .execute(&mut session, tool, None, Channel::Ui)
                .unwrap();
        }
        let before = session.document().clone();
        let revision = session.revision();
        workspace
            .execute(&mut session, ExportTool::Preview, None, Channel::Ui)
            .unwrap();
        settle(&mut workspace, &session);
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
}
