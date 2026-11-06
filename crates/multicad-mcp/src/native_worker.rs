//! Bounded native tool execution with a cache on its owning thread.
use crate::{
    ToolError,
    host::Host,
    tools::{self, Call, ToolResult},
};
use serde_json::Value;
use std::sync::{
    Arc,
    mpsc::{self, SyncSender, TrySendError},
};
use tokio::sync::oneshot;

const MAX_QUEUED: usize = 8;
type Task = Box<dyn FnOnce(&dyn Host) -> ToolResult + Send>;
type WorkerResult = Result<ToolResult, ()>;
type Reply = oneshot::Receiver<WorkerResult>;
struct Job {
    run: Task,
    reply: oneshot::Sender<WorkerResult>,
}

pub(crate) struct NativeWorker {
    host: Arc<dyn Host>,
    sender: Result<SyncSender<Job>, String>,
}
impl NativeWorker {
    pub fn new(host: Arc<dyn Host>) -> Self {
        let owner = Arc::clone(&host);
        let pair_proofs = host.pair_cycle_proofs().unwrap_or_default();
        let (sender, receiver) = mpsc::sync_channel::<Job>(MAX_QUEUED);
        let started = std::thread::Builder::new()
            .name("multicad-mcp-native".into())
            .spawn(move || {
                multicad_model::with_pair_cycle_proofs(pair_proofs, || {
                    multicad_model::with_evaluation_cache(|| {
                        while let Ok(job) = receiver.recv() {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    (job.run)(owner.as_ref())
                                }));
                            let failed = result.is_err();
                            let _ = job.reply.send(result.map_err(|_| ()));
                            if failed {
                                // Drop every native cache on this owner; queued callers
                                // receive disconnection instead of reusing partial state.
                                break;
                            }
                        }
                    })
                });
            });
        Self {
            host,
            sender: started.map(|_| sender).map_err(|e| e.to_string()),
        }
    }
    pub async fn dispatch(&self, call: Call, name: String, arguments: Value) -> ToolResult {
        let (native_call, native_name, native_arguments) =
            (call.clone(), name.clone(), arguments.clone());
        let queued = self.enqueue(Box::new(move |host| {
            tools::dispatch(host, &native_call, &native_name, native_arguments)
        }));
        let error = match queued {
            Ok(reply) => match reply.await {
                Ok(Ok(result)) => return result,
                _ => unavailable(),
            },
            Err(error) => error,
        };
        let host = Arc::clone(&self.host);
        tokio::task::spawn_blocking(move || {
            tools::reject(host.as_ref(), &call, &name, arguments, error)
        })
        .await
        .unwrap_or_else(|_| Err(unavailable()))
    }
    #[cfg(test)]
    async fn submit(&self, run: Task) -> ToolResult {
        self.enqueue(run)?
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?
    }
    fn enqueue(&self, run: Task) -> Result<Reply, ToolError> {
        let sender = self.sender.as_ref().map_err(|_| unavailable())?;
        let (reply, receive) = oneshot::channel();
        match sender.try_send(Job { run, reply }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(ToolError::new(
                    "busy",
                    "The native tool queue is full; retry after pending work finishes",
                ));
            }
            Err(TrySendError::Disconnected(_)) => return Err(unavailable()),
        }
        Ok(receive)
    }
}
fn unavailable() -> ToolError {
    ToolError::new(
        "unavailable",
        "The native tool worker is no longer available",
    )
}

/// These inspect canonical source only. They remain available while an exact
/// native operation is running; all geometry consumers use the owner queue.
pub(crate) fn source_inspection(name: &str) -> bool {
    matches!(
        name,
        "document_info"
            | "list_entities"
            | "get_entity"
            | "list_parameters"
            | "activity"
            | "review_status"
            | "history"
            | "thread_profiles"
            | "inspect_thread"
            | "mechanical_part_profiles"
            | "inspect_mechanical_part"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Headless, Workspace};
    use multicad_core::document::Document;
    use serde_json::json;

    fn worker() -> NativeWorker {
        NativeWorker::new(Arc::new(Headless::new(
            Workspace::new(Document::new("Native owner").unwrap()).unwrap(),
            None,
            false,
        )))
    }
    #[test]
    fn successive_requests_run_on_one_owner_and_failure_closes_it() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let worker = worker();
            let caller = std::thread::current().id();
            let (send, receive) = std::sync::mpsc::channel();
            worker
                .submit(Box::new(move |_| {
                    send.send(std::thread::current().id()).unwrap();
                    Ok(json!({}))
                }))
                .await
                .unwrap();
            let owner = receive.recv().unwrap();
            assert_ne!(owner, caller);
            worker
                .submit(Box::new(move |_| {
                    assert_eq!(std::thread::current().id(), owner);
                    Ok(json!({}))
                }))
                .await
                .unwrap();
            assert!(
                worker
                    .submit(Box::new(|_| panic!("native owner failure")))
                    .await
                    .is_err()
            );
            assert!(worker.submit(Box::new(|_| Ok(json!({})))).await.is_err());
        });
    }
    #[test]
    fn queue_is_bounded_fifo_and_cancelled_mutation_keeps_source_unchanged() {
        use multicad_core::commands::{Actor, Channel};
        use std::sync::atomic::{AtomicBool, Ordering};
        let host = Arc::new(Headless::new(
            Workspace::new(Document::new("Queue").unwrap()).unwrap(),
            None,
            false,
        ));
        let before = host.document();
        let worker = NativeWorker::new(host.clone());
        let (started, active) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let first = worker
            .enqueue(Box::new(move |_| {
                started.send(()).unwrap();
                released
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                Ok(json!({}))
            }))
            .unwrap();
        active
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let queued = worker.enqueue(Box::new({
            let cancel = Arc::clone(&cancel);
            move |host| tools::dispatch(host, &Call { actor: Actor::local(Channel::Mcp), cancel }, "set_mechanical_part", json!({"expected_revision":0,"name":"Cancelled washer","recipe":{"kind":"washer"}}))
        })).unwrap();
        let (order, observed) = std::sync::mpsc::channel();
        let mut replies = Vec::new();
        for index in 1..MAX_QUEUED {
            let order = order.clone();
            replies.push(
                worker
                    .enqueue(Box::new(move |_| {
                        order.send(index).unwrap();
                        Ok(json!({}))
                    }))
                    .unwrap(),
            );
        }
        assert_eq!(
            worker
                .enqueue(Box::new(|_| Ok(json!({}))))
                .unwrap_err()
                .code,
            "busy"
        );
        let call = Call {
            actor: Actor::local(Channel::Mcp),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let rejected = runtime
            .block_on(worker.dispatch(
                call.clone(),
                "set_mechanical_part".into(),
                json!({"expected_revision":0,"recipe":{"kind":"washer"}}),
            ))
            .unwrap_err();
        assert_eq!(rejected.code, "busy");
        let activity = tools::dispatch(host.as_ref(), &call, "activity", json!({})).unwrap();
        assert!(activity["entries"].as_array().unwrap().iter().any(|entry| {
            entry["tool"] == "set_mechanical_part"
                && entry["message"]
                    .as_str()
                    .is_some_and(|message| message.starts_with("busy:"))
        }));
        // Source-only inspection must remain available while native work waits.
        assert!(source_inspection("document_info"));
        tools::dispatch(
            host.as_ref(),
            &Call {
                actor: Actor::local(Channel::Mcp),
                cancel: Arc::new(AtomicBool::new(false)),
            },
            "document_info",
            json!({}),
        )
        .unwrap();
        cancel.store(true, Ordering::Relaxed);
        release.send(()).unwrap();
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                first.await.unwrap().unwrap().unwrap();
                assert_eq!(
                    queued.await.unwrap().unwrap().unwrap_err().code,
                    "cancelled"
                );
                for reply in replies {
                    reply.await.unwrap().unwrap().unwrap();
                }
            });
        assert_eq!(
            observed.try_iter().collect::<Vec<_>>(),
            (1..MAX_QUEUED).collect::<Vec<_>>()
        );
        assert_eq!(host.document(), before);
    }
}
