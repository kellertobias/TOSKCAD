//! Native owner-thread measurement provider for the UI's data-only Inspector.
use multicad_core::{document::Document, inspection::*};
use multicad_model::{Compiler, GeometryBackend, inspection::MeasurementWorker};
pub struct Backend {
    worker: Result<MeasurementWorker, String>,
}
impl Backend {
    pub fn with_geometry_backend(geometry_backend: GeometryBackend) -> Self {
        Self {
            worker: MeasurementWorker::with_geometry_backend(
                Compiler {
                    omit_display: true,
                    ..Compiler::default()
                },
                geometry_backend,
            ),
        }
    }
}
impl multicad_ui::inspection::MeasurementBackend for Backend {
    fn request(
        &mut self,
        document: &Document,
        stamp: MeasurementStamp,
        query: Option<&MeasurementQuery>,
    ) -> Result<u64, String> {
        self.worker
            .as_mut()
            .map_err(|e| e.clone())?
            .request(document, stamp, query)
    }
    fn poll(&mut self) -> Option<MeasurementReply> {
        self.worker.as_mut().ok().and_then(MeasurementWorker::poll)
    }
    fn cancel(&mut self) {
        if let Ok(worker) = &mut self.worker {
            worker.cancel();
        }
    }
}
