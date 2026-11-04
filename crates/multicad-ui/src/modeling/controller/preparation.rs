//! One running preparation and one replaceable pending intent. Cancellation
//! revokes publication without joining an uninterruptible validation call.
use super::*;
use multicad_core::commands::PreparationSnapshot;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct Job {
    pub source: PreparationSnapshot,
    pub request: ModelRequest,
    pub channel: Channel,
    pub form: FormStamp,
    pub generation: u64,
}
pub(super) struct Reply {
    pub request: ModelRequest,
    pub form: FormStamp,
    generation: u64,
    pub result: Result<(model::ModelPrepared, ContentHash), String>,
}
struct Running {
    receive: Receiver<Reply>,
    request: ModelRequest,
    form: FormStamp,
    generation: u64,
}
#[derive(Default)]
pub(super) struct Lane {
    running: Option<Running>,
    queued: Option<Job>,
    desired: Option<u64>,
}
impl Lane {
    pub fn pending(&self) -> bool {
        self.desired.is_some()
    }
    pub fn cancel(&mut self) {
        self.desired = None;
        self.queued = None;
    }
    pub fn submit(&mut self, job: Job) -> Result<(), String> {
        self.desired = Some(job.generation);
        self.queued = Some(job);
        self.start()
    }
    fn start(&mut self) -> Result<(), String> {
        if self.running.is_some() {
            return Ok(());
        }
        let Some(job) = self.queued.take() else {
            return Ok(());
        };
        let (send, receive) = mpsc::sync_channel(1);
        let running = Running {
            receive,
            request: job.request.clone(),
            form: job.form,
            generation: job.generation,
        };
        let spawned = std::thread::Builder::new()
            .name("model-preparation".into())
            .spawn(move || {
                let result = (|| {
                    let prepared = job
                        .request
                        .prepare(job.source.session(), Actor::local(job.channel))?;
                    let content = ContentHash::document(prepared.prepared.document())?;
                    Ok((prepared, content))
                })();
                let _ = send.send(Reply {
                    request: job.request,
                    form: job.form,
                    generation: job.generation,
                    result,
                });
            });
        if let Err(error) = spawned {
            self.desired = None;
            return Err(format!("Cannot start model preparation: {error}"));
        }
        self.running = Some(running);
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Reply> {
        let running = self.running.as_ref()?;
        let reply = match running.receive.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Reply {
                request: running.request.clone(),
                form: running.form,
                generation: running.generation,
                result: Err("Model preparation worker stopped before returning a result".into()),
            },
        };
        self.running = None;
        let current = self.desired == Some(reply.generation);
        if current {
            self.desired = None;
        }
        if let Err(error) = self.start() {
            return Some(Reply {
                result: Err(error),
                ..reply
            });
        }
        current.then_some(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn job(session: &Session, generation: u64) -> Job {
        Job {
            source: session.preparation_snapshot(),
            request: ModelRequest::Thread(Box::new(model::ThreadRequest::new(
                format!("Thread {generation}"),
                Default::default(),
            ))),
            channel: Channel::Ui,
            form: FormStamp::new(),
            generation,
        }
    }
    fn blocked(lane: &mut Lane, session: &Session) -> mpsc::SyncSender<Reply> {
        let (send, receive) = mpsc::sync_channel(1);
        let job = job(session, 0);
        lane.running = Some(Running {
            receive,
            request: job.request,
            form: job.form,
            generation: 0,
        });
        send
    }
    fn release(send: mpsc::SyncSender<Reply>, session: &Session) {
        let job = job(session, 0);
        send.send(Reply {
            request: job.request,
            form: job.form,
            generation: 0,
            result: Err("obsolete".into()),
        })
        .unwrap();
    }
    #[test]
    fn busy_preparation_coalesces_and_only_latest_source_transaction_returns() {
        let session = Session::new(Document::new("Preparation").unwrap()).unwrap();
        let mut lane = Lane::default();
        let send = blocked(&mut lane, &session);
        lane.submit(job(&session, 1)).unwrap();
        lane.submit(job(&session, 2)).unwrap();
        assert!(lane.pending());
        assert_eq!(lane.queued.as_ref().unwrap().generation, 2);
        assert!(lane.poll().is_none());
        release(send, &session);
        assert!(lane.poll().is_none());
        let deadline = Instant::now() + Duration::from_secs(10);
        let reply = loop {
            if let Some(reply) = lane.poll() {
                break reply;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(reply.generation, 2);
        let (prepared, content) = reply.result.unwrap();
        session.validate_preview(&prepared.prepared).unwrap();
        assert_eq!(
            content,
            ContentHash::document(prepared.prepared.document()).unwrap()
        );
        assert!(!lane.pending());
        assert!(lane.running.is_none());
        assert_eq!(session.revision(), 0);
    }
    #[test]
    fn cancel_revokes_running_and_queued_results_without_waiting() {
        let session = Session::new(Document::new("Cancelled").unwrap()).unwrap();
        let mut lane = Lane::default();
        let send = blocked(&mut lane, &session);
        lane.submit(job(&session, 1)).unwrap();
        lane.cancel();
        assert!(!lane.pending());
        assert!(lane.queued.is_none());
        assert!(lane.running.is_some());
        release(send, &session);
        assert!(lane.poll().is_none());
        assert!(lane.running.is_none());
        assert_eq!(session.revision(), 0);
    }
    #[test]
    fn failed_worker_cannot_leave_current_preparation_pending() {
        let session = Session::new(Document::new("Failed").unwrap()).unwrap();
        let mut lane = Lane::default();
        let send = blocked(&mut lane, &session);
        lane.desired = Some(0);
        drop(send);
        let reply = lane.poll().unwrap();
        assert!(reply.result.unwrap_err().contains("worker stopped"));
        assert!(!lane.pending());
    }

    fn blocked_controller(session: &Session) -> (ModelToolController, mpsc::SyncSender<Reply>) {
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(
            super::super::super::native_fixture::NativeBackend::new(),
        ));
        controller
            .begin_thread(
                session,
                &Default::default(),
                None,
                &Default::default(),
                Channel::Ui,
            )
            .unwrap();
        let send = blocked(&mut controller.preparation, session);
        controller
            .preview(session, &Default::default(), Channel::Ui)
            .unwrap();
        assert!(controller.pending());
        assert!(controller.candidate.is_none());
        (controller, send)
    }
    #[test]
    fn controller_cancel_discards_preparation_before_native_publication() {
        let session =
            Session::new(Document::new("Cancel before native evaluation").unwrap()).unwrap();
        let before = session.document().clone();
        let (mut controller, send) = blocked_controller(&session);
        controller.cancel();
        assert!(!controller.pending());
        release(send, &session);
        controller.poll(&session, &Default::default());
        assert!(controller.candidate.is_none());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert_eq!(session.document(), &before);
        assert!(!session.can_undo());
    }
    #[test]
    fn foreign_session_with_identical_document_and_revision_revokes_preparation() {
        let session = Session::new(Document::new("Original owner").unwrap()).unwrap();
        let foreign = Session::new(session.document().clone()).unwrap();
        let (mut controller, send) = blocked_controller(&session);
        controller.poll(&foreign, &Default::default());
        assert!(!controller.pending());
        assert!(controller.error().unwrap().contains("session changed"));
        release(send, &session);
        controller.poll(&foreign, &Default::default());
        assert!(controller.candidate.is_none());
        assert!(!controller.can_apply(&foreign, &Default::default()));
        assert_eq!(foreign.revision(), 0);
        assert!(!foreign.can_undo());
    }
}
