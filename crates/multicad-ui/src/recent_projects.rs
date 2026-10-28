//! Local launch history is metadata; it never replaces a canonical project.
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};
pub const MAX_RECENTS: usize = 20;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentProject {
    pub path: PathBuf,
    pub title: String,
    pub last_opened_unix_ms: u64,
}
impl RecentProject {
    pub fn validate(&self) -> Result<(), String> {
        if !self.path.is_absolute()
            || self.path.as_os_str().len() > 8192
            || self.title.trim().is_empty()
            || self.title.chars().count() > 256
            || self.title.chars().any(char::is_control)
        {
            return Err("Invalid recent-project metadata".into());
        }
        Ok(())
    }
}
pub fn normalize(mut entries: Vec<RecentProject>) -> Result<Vec<RecentProject>, String> {
    if entries.len() > 1024 {
        return Err("Recent-project list exceeds its budget".into());
    }
    for entry in &entries {
        entry.validate()?;
    }
    entries.sort_by(|a, b| {
        b.last_opened_unix_ms
            .cmp(&a.last_opened_unix_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut seen = std::collections::BTreeSet::new();
    entries.retain(|e| seen.insert(e.path.clone()));
    entries.truncate(MAX_RECENTS);
    Ok(entries)
}
pub trait RecentStore: Send + 'static {
    fn load(&mut self) -> Result<Vec<RecentProject>, String>;
    /// Merge atomically with other application instances, preserving unrelated entries.
    fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String>;
}
enum Job {
    Load,
    Record(RecentProject),
}
pub struct RecentWorker {
    tx: SyncSender<Job>,
    rx: Receiver<(bool, Result<Vec<RecentProject>, String>)>,
    pending: usize,
    pending_records: usize,
}
impl RecentWorker {
    pub fn new(mut store: Box<dyn RecentStore>) -> Result<Self, String> {
        let (tx, jobs) = mpsc::sync_channel(16);
        let (results, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("multicad-recents".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let record = matches!(job, Job::Record(_));
                    let result = match job {
                        Job::Load => store.load(),
                        Job::Record(entry) => store.record(&entry),
                    }
                    .and_then(normalize);
                    if results.send((record, result)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        tx.send(Job::Load).map_err(|e| e.to_string())?;
        Ok(Self {
            tx,
            rx,
            pending: 1,
            pending_records: 0,
        })
    }
    pub fn record(&mut self, entry: RecentProject) -> Result<(), String> {
        entry.validate()?;
        self.tx
            .try_send(Job::Record(entry))
            .map_err(|e| format!("Recent-project metadata queue: {e}"))?;
        self.pending += 1;
        self.pending_records += 1;
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Result<Vec<RecentProject>, String>> {
        match self.rx.try_recv() {
            Ok((record, result)) => {
                self.pending_records = self.pending_records.saturating_sub(usize::from(record));
                self.pending = self.pending.saturating_sub(1);
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) if self.pending > 0 => {
                self.pending = 0;
                self.pending_records = 0;
                Some(Err("Recent-project metadata worker stopped".into()))
            }
            Err(TryRecvError::Disconnected) => None,
        }
    }
    pub fn writing(&self) -> bool {
        self.pending_records > 0
    }
    pub fn pending(&self) -> bool {
        self.pending > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    #[test]
    fn writes_remain_pending_until_the_storage_receipt_and_queue_drain() {
        struct Delayed {
            entered: mpsc::Sender<()>,
            finish: Receiver<()>,
        }
        impl RecentStore for Delayed {
            fn load(&mut self) -> Result<Vec<RecentProject>, String> {
                Ok(vec![])
            }
            fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String> {
                self.entered.send(()).unwrap();
                self.finish.recv().unwrap();
                Ok(vec![entry.clone()])
            }
        }
        let (entered, signal) = mpsc::channel();
        let (finish, gate) = mpsc::channel();
        let mut worker = RecentWorker::new(Box::new(Delayed {
            entered,
            finish: gate,
        }))
        .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while worker.pending() {
            let _ = worker.poll();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(!worker.writing());
        let entry = RecentProject {
            path: PathBuf::from("/private/tmp/confirmed.multicad"),
            title: "Confirmed".into(),
            last_opened_unix_ms: 1,
        };
        worker.record(entry.clone()).unwrap();
        signal.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(worker.writing() && worker.pending());
        assert!(worker.poll().is_none());
        finish.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = worker.poll() {
                assert_eq!(result.unwrap(), vec![entry]);
                break;
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(!worker.writing() && !worker.pending());
    }
}
