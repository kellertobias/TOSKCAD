//! Exact assembly queries stay on their native owner worker.
use multicad_core::{assembly_inspection::*, document::Document};
use multicad_model::{Compiler, assembly_inspection::AssemblyInspectionWorker};
pub struct Backend {
    worker: Result<AssemblyInspectionWorker, String>,
}
impl Backend {
    pub fn new() -> Self {
        Self {
            worker: AssemblyInspectionWorker::new(Compiler::default()),
        }
    }
}
impl multicad_ui::assembly::AssemblyInspectionBackend for Backend {
    fn request(
        &mut self,
        document: &Document,
        query: &AssemblyInspectionQuery,
    ) -> Result<u64, String> {
        self.worker
            .as_mut()
            .map_err(|error| error.clone())?
            .submit(document, query.clone())
    }
    fn poll(&mut self) -> Option<AssemblyInspectionReply> {
        self.worker
            .as_mut()
            .ok()
            .and_then(AssemblyInspectionWorker::poll)
    }
    fn cancel(&mut self) {
        if let Ok(worker) = &mut self.worker {
            worker.cancel();
        }
    }
}
