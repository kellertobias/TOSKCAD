//! One bounded source-hash lane per sketch feed. Old snapshots can populate
//! only their own immutable cache epoch, never a newer live source revision.
use multicad_core::commands::{PreparationSnapshot, Session};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

pub(super) struct HashWorker {
    requests: SyncSender<PreparationSnapshot>,
    replies: Receiver<Result<(), String>>,
    busy: bool,
}
impl HashWorker {
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn new() -> Result<Self, String> {
        Self::spawn(|snapshot| snapshot.session().source_hash().map(|_| ()))
    }
    pub(super) fn spawn(
        calculate: impl Fn(PreparationSnapshot) -> Result<(), String> + Send + 'static,
    ) -> Result<Self, String> {
        let (requests, input) = mpsc::sync_channel::<PreparationSnapshot>(1);
        let (output, replies) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("sketch-source-hash".into())
            .spawn(move || {
                while let Ok(snapshot) = input.recv() {
                    let result = calculate(snapshot);
                    if output.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            requests,
            replies,
            busy: false,
        })
    }
    pub fn poll(&mut self) -> Result<(), String> {
        if !self.busy {
            return Ok(());
        }
        match self.replies.try_recv() {
            Ok(result) => {
                self.busy = false;
                result
            }
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => {
                self.busy = false;
                Err("Sketch source inspection worker stopped".into())
            }
        }
    }
    pub fn request(&mut self, session: &Session) -> Result<(), String> {
        if !self.busy {
            self.requests
                .try_send(session.preparation_snapshot())
                .map_err(|error| error.to_string())?;
            self.busy = true;
        }
        Ok(())
    }
}
