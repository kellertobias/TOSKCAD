//! Local launch history is metadata; it never replaces a canonical project.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
};
pub const MAX_RECENTS: usize = 20;
/// User-local navigation state. It is never part of CAD source or Undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentView {
    pub mode: crate::registry::Mode,
    pub workspace: String,
    pub architecture: Option<crate::architecture_view::Recall>,
}
impl Default for RecentView {
    fn default() -> Self {
        Self {
            mode: crate::registry::Mode::Parameters,
            workspace: "General".into(),
            architecture: None,
        }
    }
}
impl RecentView {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .architecture
            .is_some_and(|view| view.level.is_some_and(|id| id.is_nil()))
        {
            return Err("Invalid Architecture level identity".into());
        }
        if self.workspace.trim().is_empty()
            || self.workspace.chars().count() > 80
            || self.workspace.chars().any(char::is_control)
        {
            return Err("Invalid recent workspace".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentProject {
    pub path: PathBuf,
    pub title: String,
    pub last_opened_unix_ms: u64,
    pub view: Option<RecentView>,
}
impl RecentProject {
    /// Missing view/context means unvisited, while an explicit World context
    /// is `Some(Recall { level: None, .. })` and replaces the previous floor.
    pub fn inherit_view(&mut self, previous: Option<&RecentView>) {
        if let Some(previous) = previous {
            if let Some(current) = &mut self.view {
                if current.architecture.is_none() {
                    current.architecture = previous.architecture;
                }
            } else {
                self.view = Some(previous.clone());
            }
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(view) = &self.view {
            view.validate()?;
        }
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
    /// A visit with no view preserves that path's previously recorded navigation.
    /// An uninitialized Architecture context inherits the earlier context;
    /// an explicit World context replaces it. `RecentProject::inherit_view`
    /// supplies these merge semantics for file stores and queued visits.
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
    loading: bool,
    deferred: BTreeMap<PathBuf, RecentProject>,
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
            loading: true,
            deferred: BTreeMap::new(),
        })
    }
    pub fn record(&mut self, mut entry: RecentProject) -> Result<(), String> {
        entry.validate()?;
        entry.inherit_view(
            self.deferred
                .get(&entry.path)
                .and_then(|previous| previous.view.as_ref()),
        );
        self.deferred.insert(entry.path.clone(), entry);
        // Keep only the same bounded history the persistent store can retain.
        if self.deferred.len() > MAX_RECENTS {
            let oldest = self
                .deferred
                .values()
                .min_by_key(|entry| entry.last_opened_unix_ms)
                .map(|entry| entry.path.clone())
                .expect("nonempty");
            self.deferred.remove(&oldest);
        }
        self.flush_deferred()
    }
    fn flush_deferred(&mut self) -> Result<(), String> {
        while let Some((_, entry)) = self.deferred.pop_first() {
            match self.tx.try_send(Job::Record(entry)) {
                Ok(()) => {
                    self.pending += 1;
                    self.pending_records += 1;
                }
                Err(TrySendError::Full(Job::Record(entry))) => {
                    self.deferred.insert(entry.path.clone(), entry);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.deferred.clear();
                    return Err("Recent-project metadata worker stopped".into());
                }
                Err(TrySendError::Full(Job::Load)) => unreachable!("only records are flushed"),
            }
        }
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Result<Vec<RecentProject>, String>> {
        if let Err(error) = self.flush_deferred() {
            return Some(Err(error));
        }
        match self.rx.try_recv() {
            Ok((record, result)) => {
                if !record {
                    self.loading = false;
                }
                self.pending_records = self.pending_records.saturating_sub(usize::from(record));
                self.pending = self.pending.saturating_sub(1);
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) if self.pending > 0 => {
                self.pending = 0;
                self.pending_records = 0;
                self.loading = false;
                Some(Err("Recent-project metadata worker stopped".into()))
            }
            Err(TryRecvError::Disconnected) => None,
        }
    }
    pub fn loading(&self) -> bool {
        self.loading
    }
    pub fn writing(&self) -> bool {
        self.pending_records > 0 || !self.deferred.is_empty()
    }
    pub fn pending(&self) -> bool {
        self.pending > 0 || !self.deferred.is_empty()
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
            path: std::env::temp_dir().join("confirmed.multicad"),
            title: "Confirmed".into(),
            last_opened_unix_ms: 1,
            view: None,
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

#[cfg(test)]
mod backpressure_tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    #[test]
    fn a_full_queue_coalesces_and_eventually_persists_the_last_workspace() {
        struct Store {
            entered: mpsc::Sender<()>,
            gate: Receiver<()>,
            first: bool,
            last: Arc<Mutex<Option<RecentProject>>>,
        }
        impl RecentStore for Store {
            fn load(&mut self) -> Result<Vec<RecentProject>, String> {
                Ok(vec![])
            }
            fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String> {
                if self.first {
                    self.first = false;
                    self.entered.send(()).unwrap();
                    self.gate.recv().unwrap();
                }
                *self.last.lock().unwrap() = Some(entry.clone());
                Ok(vec![entry.clone()])
            }
        }
        let (entered, started) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let last = Arc::new(Mutex::new(None));
        let mut worker = RecentWorker::new(Box::new(Store {
            entered,
            gate,
            first: true,
            last: last.clone(),
        }))
        .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while worker.pending() {
            let _ = worker.poll();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        let mut entry = RecentProject {
            path: std::env::temp_dir().join("rapid-view.multicad"),
            title: "Rapid".into(),
            last_opened_unix_ms: 1,
            view: Some(RecentView::default()),
        };
        worker.record(entry.clone()).unwrap();
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let saved_context = crate::architecture_view::Recall {
            plan: true,
            level: Some(multicad_core::document::EntityId::new()),
        };
        for index in 2..102 {
            entry.last_opened_unix_ms = index;
            entry.view = Some(RecentView {
                mode: crate::registry::Mode::Model,
                workspace: format!("Workspace {index}"),
                architecture: (index == 50).then_some(saved_context),
            });
            worker.record(entry.clone()).unwrap();
        }
        assert!(!worker.deferred.is_empty());
        assert!(worker.deferred.len() <= MAX_RECENTS);
        assert!(worker.writing());
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while worker.pending() {
            if let Some(result) = worker.poll() {
                result.unwrap();
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        entry.view.as_mut().unwrap().architecture = Some(saved_context);
        assert_eq!(last.lock().unwrap().as_ref(), Some(&entry));
        assert!(!worker.writing());
    }
}
