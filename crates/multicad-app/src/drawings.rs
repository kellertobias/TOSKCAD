//! Native drawing composition, sharing the bounded geometry worker while keeping
//! native solids and projection operations off the UI thread.
use multicad_core::{
    document::Document,
    drawings::{DrawingEvaluation, DrawingStamp},
};
use multicad_model::{Compiler, DrawingProviders, Runtime};
use multicad_ui::drawings::{DrawingBackend, DrawingReply};
use std::sync::Arc;
pub struct Backend {
    runtime: Result<Runtime<Arc<DrawingEvaluation>>, String>,
    expected: Option<(u64, DrawingStamp)>,
}
impl Backend {
    pub fn new() -> Self {
        Self {
            runtime: Runtime::with_model_publisher(Compiler::default(), |document, model, _| {
                model
                    .drawings(
                        document,
                        model.snapshot().stamp.revision,
                        &DrawingProviders::default(),
                    )
                    .map(Arc::new)
            }),
            expected: None,
        }
    }
}
impl DrawingBackend for Backend {
    fn cancel(&mut self) {
        self.expected = None;
        if let Ok(runtime) = self.runtime.as_mut() {
            runtime.cancel();
        }
    }
    fn request(&mut self, document: &Document, stamp: DrawingStamp) -> Result<(), String> {
        stamp.check(document, stamp.revision)?;
        let runtime = self.runtime.as_mut().map_err(|e| e.clone())?;
        let generation = runtime.request(document, stamp.revision)?;
        self.expected = Some((generation, stamp));
        Ok(())
    }
    fn poll(&mut self) -> Option<DrawingReply> {
        let (generation, stamp) = self.expected?;
        let reply = self.runtime.as_mut().ok()?.poll()?;
        if reply.generation != generation {
            return None;
        }
        self.expected = None;
        let result = reply.result.map_err(|e| e.to_string()).and_then(|result| {
            if result.stamp == stamp {
                Ok(result)
            } else {
                Err("Native drawing result does not match the requested source".into())
            }
        });
        Some(DrawingReply { stamp, result })
    }
}
