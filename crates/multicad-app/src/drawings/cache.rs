//! Optional, disposable preview publications. All I/O stays on the drawing
//! worker. Canonical source and manufacturing export never consume this cache.
use multicad_core::{document::Document, drawings::DrawingEvaluation, project_file::ContentHash};
use multicad_model::GeometryBackend;
use std::{
    io::{Read, Write},
    path::PathBuf,
};

const ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const TOTAL_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Cache {
    root: PathBuf,
    build: ContentHash,
}
impl Cache {
    pub fn installed() -> Option<Self> {
        let base = if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Caches"))
        } else if cfg!(target_os = "windows") {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        }?;
        // Rebuilding/replacing the executable invalidates generated previews,
        // including local dev builds with an unchanged package version.
        let metadata = std::fs::metadata(std::env::current_exe().ok()?).ok()?;
        let build = ContentHash::of(
            format!(
                "drawing-preview-v1:{}:{}:{}:{}:{:?}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                metadata.len(),
                metadata.modified().ok()?
            )
            .as_bytes(),
        );
        Some(Self {
            root: base.join("MultiCAD/drawing-previews"),
            build,
        })
    }
    pub fn for_backend(mut self, backend: GeometryBackend) -> Self {
        if backend == GeometryBackend::Rust {
            self.build = ContentHash::of(format!("{}:rust", self.build).as_bytes());
        }
        self
    }
    fn path(&self, document: &Document, tag: u64) -> Option<PathBuf> {
        let hash = ContentHash::document(document).ok()?;
        Some(
            self.root
                .join(format!("{}-{hash}-{tag}.preview", self.build)),
        )
    }
    pub fn read(&self, document: &Document, revision: u64, tag: u64) -> Option<DrawingEvaluation> {
        let mut file = std::fs::File::open(self.path(document, tag)?).ok()?;
        if file.metadata().ok()?.len() > ENTRY_BYTES {
            return None;
        }
        let mut digest = [0; 32];
        file.read_exact(&mut digest).ok()?;
        let mut bytes = Vec::new();
        file.take(ENTRY_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > ENTRY_BYTES || ContentHash::of(&bytes).0 != digest {
            return None;
        }
        let mut result: DrawingEvaluation = serde_json::from_slice(&bytes).ok()?;
        if result.stamp.document != document.id()
            || result.stamp.source_hash != ContentHash::document(document).ok()?
        {
            return None;
        }
        result.stamp.revision = revision;
        Some(result)
    }
    pub fn write(&self, document: &Document, tag: u64, result: &DrawingEvaluation) {
        // Cache failures (permissions, space, corruption) must never fail a preview.
        let _ = self.try_write(document, tag, result);
    }
    fn try_write(&self, document: &Document, tag: u64, result: &DrawingEvaluation) -> Option<()> {
        // Native failures can be transient (for example, memory pressure).
        // Display the current failure, but allow the next open to retry it.
        if result
            .views
            .values()
            .any(|view| !matches!(view, multicad_core::drawings::ViewEvaluation::Ready { .. }))
        {
            return None;
        }
        let path = self.path(document, tag)?;
        let bytes = serde_json::to_vec(result).ok()?;
        if bytes.len() as u64 + 32 > ENTRY_BYTES {
            return None;
        }
        std::fs::create_dir_all(&self.root).ok()?;
        let mut file = tempfile::NamedTempFile::new_in(&self.root).ok()?;
        file.write_all(&ContentHash::of(&bytes).0).ok()?;
        file.write_all(&bytes).ok()?;
        file.persist(path).ok()?;
        // Bound all builds together. Evict the oldest completed entries.
        let mut entries: Vec<_> = std::fs::read_dir(&self.root)
            .ok()?
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "preview"))
            .filter_map(|e| {
                e.metadata()
                    .ok()
                    .map(|m| (m.modified().ok(), m.len(), e.path()))
            })
            .collect();
        entries.sort_by_key(|e| e.0);
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        let mut count = entries.len();
        for (_, size, path) in entries {
            if total <= TOTAL_BYTES && count <= 64 {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total = total.saturating_sub(size);
                count -= 1;
            }
        }
        Some(())
    }
    #[cfg(test)]
    pub fn isolated(root: PathBuf) -> Self {
        Self {
            root,
            build: ContentHash::of(b"test-build"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::drawings::DrawingStamp;
    #[test]
    fn cache_checks_source_build_page_and_integrity_and_restamps_reopened_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::isolated(dir.path().to_owned());
        let doc = Document::new("Cached drawing").unwrap();
        let result = DrawingEvaluation {
            stamp: DrawingStamp {
                document: doc.id(),
                revision: 42,
                source_hash: ContentHash::document(&doc).unwrap(),
            },
            views: Default::default(),
            annotations: Default::default(),
        };
        cache.write(&doc, 1, &result);
        assert_eq!(cache.read(&doc, 0, 1).unwrap().stamp.revision, 0);
        let rust_cache = cache.clone().for_backend(GeometryBackend::Rust);
        assert!(rust_cache.read(&doc, 0, 1).is_none());
        rust_cache.write(&doc, 2, &result);
        assert!(cache.read(&doc, 0, 2).is_none());
        assert_eq!(rust_cache.read(&doc, 42, 2).unwrap(), result);
        assert!(
            cache
                .read(&Document::new("Changed").unwrap(), 0, 1)
                .is_none()
        );
        let other = Cache {
            build: ContentHash::of(b"new build"),
            ..cache.clone()
        };
        assert!(other.read(&doc, 0, 1).is_none());
        std::fs::write(cache.path(&doc, 1).unwrap(), b"broken").unwrap();
        assert!(cache.read(&doc, 0, 1).is_none());
        cache.write(&doc, 1, &result);
        assert_eq!(cache.read(&doc, 42, 1).unwrap(), result);
        let mut failed = result.clone();
        failed.views.insert(
            multicad_core::document::EntityId::new(),
            multicad_core::drawings::ViewEvaluation::Unresolved {
                message: "Native preview failed".into(),
            },
        );
        cache.write(&doc, 2, &failed);
        assert!(cache.read(&doc, 42, 2).is_none());
    }
}
