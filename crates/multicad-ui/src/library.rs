//! The single installation library and project cards. Registration is an
//! explicit user action that stores an immutable managed snapshot of a saved
//! project; it never edits the source project, never runs on Save and never
//! changes definitions already embedded (pinned) in consuming projects.
//! File-system access belongs to the application's `LibraryStore`; this
//! module owns the nonblocking worker and preview textures.
use crate::storage::FileStamp;
use eframe::egui;
use multicad_core::{
    document::EntityId,
    project_file::{ContentHash, ProjectCard, info::PreviewImage},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
};

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    /// Stable entry identity within this installation's library.
    pub id: EntityId,
    /// Document identity of the registered project; one entry per document.
    pub document: EntityId,
    pub name: String,
    pub description: String,
    pub preview: Option<PreviewImage>,
    /// Managed immutable copy inside the library directory.
    pub snapshot: PathBuf,
    /// Where the file was registered from. Informational only: the entry
    /// keeps working when this file moves, changes or disappears.
    pub source_path: Option<PathBuf>,
    /// Hash of the snapshot bytes.
    pub content: ContentHash,
    pub registered_unix_ms: u64,
    pub symbol_definitions: usize,
    /// Set when the managed copy is missing or unreadable.
    pub problem: Option<String>,
}
#[derive(Clone, Debug)]
pub struct RegisterRequest {
    pub path: PathBuf,
    /// The saved file this window last wrote or opened; registration refuses
    /// a file that changed on disk since then.
    pub stamp: FileStamp,
    pub document: EntityId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    Added,
    /// The same document was registered before with different content; its
    /// one entry now holds the new snapshot.
    Updated,
    /// The identical saved content is already registered; nothing changed.
    Unchanged,
}
#[derive(Clone, Debug)]
pub struct RegisterReceipt {
    pub outcome: Registration,
    pub entry: LibraryEntry,
    /// The previous registration's source when it was a different file.
    pub previous_source: Option<PathBuf>,
}
impl RegisterReceipt {
    pub fn message(&self) -> String {
        let name = &self.entry.name;
        match self.outcome {
            Registration::Added => format!("Added “{name}” to the library"),
            Registration::Unchanged => {
                format!(
                    "“{name}” is already in the library with this saved content · nothing changed"
                )
            }
            Registration::Updated => {
                let from = self
                    .previous_source
                    .as_ref()
                    .map(|p| format!(" (previously registered from {})", p.display()))
                    .unwrap_or_default();
                format!(
                    "Updated the library entry “{name}”{from} · projects that already use it keep their pinned copy until you review an update"
                )
            }
        }
    }
}
pub trait LibraryStore: Send + 'static {
    fn load(&mut self) -> Result<Vec<LibraryEntry>, String>;
    fn register(
        &mut self,
        request: &RegisterRequest,
    ) -> Result<(RegisterReceipt, Vec<LibraryEntry>), String>;
    /// Name, description and preview of a project file for the home screen.
    fn card(&mut self, path: &Path) -> Result<ProjectCard, String>;
    /// Read a user-chosen preview image file.
    fn read_preview(&mut self, path: &Path) -> Result<PreviewImage, String>;
}
/// Decode a checked preview for display; failure means the image is unavailable.
pub fn decode_preview(image: &PreviewImage) -> Result<egui::ColorImage, String> {
    let icon = eframe::icon_data::from_png_bytes(image.png())
        .map_err(|e| format!("The preview image cannot be decoded: {e}"))?;
    if icon.width != image.width() || icon.height != image.height() {
        return Err("The preview image dimensions do not match its header".into());
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    ))
}
enum Job {
    Load,
    Register(RegisterRequest),
    Card(PathBuf),
    Preview { local: EntityId, path: PathBuf },
}
pub enum LibraryEvent {
    Loaded(Result<Vec<LibraryEntry>, String>),
    Registered {
        document: EntityId,
        result: Result<(RegisterReceipt, Vec<LibraryEntry>), String>,
    },
    Card {
        path: PathBuf,
        result: Result<ProjectCard, String>,
    },
    Preview {
        local: EntityId,
        result: Result<PreviewImage, String>,
    },
}
pub struct LibraryWorker {
    tx: Sender<Job>,
    rx: Receiver<LibraryEvent>,
    pending: usize,
    registering: bool,
}
impl LibraryWorker {
    pub fn new(mut store: Box<dyn LibraryStore>) -> Result<Self, String> {
        let (tx, jobs) = mpsc::channel();
        let (results, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("multicad-library".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let event = match job {
                        Job::Load => LibraryEvent::Loaded(store.load()),
                        Job::Register(request) => LibraryEvent::Registered {
                            document: request.document,
                            result: store.register(&request),
                        },
                        Job::Card(path) => {
                            let result = store.card(&path);
                            LibraryEvent::Card { path, result }
                        }
                        Job::Preview { local, path } => LibraryEvent::Preview {
                            local,
                            result: store.read_preview(&path).and_then(|image| {
                                decode_preview(&image)?;
                                Ok(image)
                            }),
                        },
                    };
                    if results.send(event).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut worker = Self {
            tx,
            rx,
            pending: 0,
            registering: false,
        };
        worker.submit(Job::Load)?;
        Ok(worker)
    }
    fn submit(&mut self, job: Job) -> Result<(), String> {
        self.tx
            .send(job)
            .map_err(|_| "The library worker stopped".to_string())?;
        self.pending += 1;
        Ok(())
    }
    pub fn reload(&mut self) -> Result<(), String> {
        self.submit(Job::Load)
    }
    pub fn register(&mut self, request: RegisterRequest) -> Result<(), String> {
        if self.registering {
            return Err("A library registration is already running".into());
        }
        self.submit(Job::Register(request))?;
        self.registering = true;
        Ok(())
    }
    pub fn card(&mut self, path: PathBuf) -> Result<(), String> {
        self.submit(Job::Card(path))
    }
    pub fn read_preview(&mut self, local: EntityId, path: PathBuf) -> Result<(), String> {
        self.submit(Job::Preview { local, path })
    }
    pub fn poll(&mut self) -> Option<LibraryEvent> {
        match self.rx.try_recv() {
            Ok(event) => {
                self.pending = self.pending.saturating_sub(1);
                if matches!(event, LibraryEvent::Registered { .. }) {
                    self.registering = false;
                }
                Some(event)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) if self.pending > 0 => {
                self.pending = 0;
                self.registering = false;
                Some(LibraryEvent::Loaded(Err(
                    "The library worker stopped".into()
                )))
            }
            Err(TryRecvError::Disconnected) => None,
        }
    }
    pub fn pending(&self) -> bool {
        self.pending > 0
    }
    pub fn registering(&self) -> bool {
        self.registering
    }
}
/// Decoded preview textures keyed by image content.
#[derive(Default)]
pub struct PreviewTextures {
    textures: BTreeMap<ContentHash, Result<egui::TextureHandle, String>>,
}
impl PreviewTextures {
    /// Show `image` fitted into `size`, or a labelled placeholder.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        image: Option<&PreviewImage>,
        size: egui::Vec2,
        alt: &str,
    ) {
        let texture = image.map(|image| {
            let ctx = ui.ctx().clone();
            self.textures
                .entry(image.content())
                .or_insert_with(|| {
                    decode_preview(image).map(|decoded| {
                        ctx.load_texture(
                            format!("file-preview-{}", image.content()),
                            decoded,
                            Default::default(),
                        )
                    })
                })
                .clone()
        });
        match texture {
            Some(Ok(texture)) => {
                let [w, h] = texture.size().map(|v| v as f32);
                let scale = (size.x / w).min(size.y / h);
                ui.add(
                    egui::Image::new((texture.id(), egui::vec2(w * scale, h * scale)))
                        .alt_text(alt),
                );
            }
            other => {
                let text = match other {
                    Some(Err(_)) => "Preview unavailable",
                    _ => "No preview image",
                };
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 4, egui::Color32::from_rgb(36, 46, 60));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(11.),
                    egui::Color32::GRAY,
                );
                if let Some(Err(error)) = other {
                    response.on_hover_text(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    struct Fake;
    impl LibraryStore for Fake {
        fn load(&mut self) -> Result<Vec<LibraryEntry>, String> {
            Ok(vec![])
        }
        fn register(
            &mut self,
            _: &RegisterRequest,
        ) -> Result<(RegisterReceipt, Vec<LibraryEntry>), String> {
            Err("Library directory is read-only".into())
        }
        fn card(&mut self, path: &Path) -> Result<ProjectCard, String> {
            Err(format!("missing {}", path.display()))
        }
        fn read_preview(&mut self, _: &Path) -> Result<PreviewImage, String> {
            // A valid header whose pixels cannot be decoded.
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            png.extend_from_slice(&[0, 0, 0, 4, 0, 0, 0, 4, 8, 6, 0, 0, 0, 0, 0, 0, 0]);
            PreviewImage::from_png(png)
        }
    }
    #[test]
    fn failures_are_reported_and_undecodable_previews_are_rejected() {
        let mut worker = LibraryWorker::new(Box::new(Fake)).unwrap();
        let local = EntityId::new();
        worker
            .register(RegisterRequest {
                path: "/private/tmp/a.multicad".into(),
                stamp: FileStamp {
                    manifest: ContentHash::of(b"a"),
                    length: 1,
                },
                document: local,
            })
            .unwrap();
        assert!(worker.registering());
        assert!(
            worker
                .register(RegisterRequest {
                    path: "/private/tmp/a.multicad".into(),
                    stamp: FileStamp {
                        manifest: ContentHash::of(b"a"),
                        length: 1,
                    },
                    document: local,
                })
                .is_err(),
            "one registration at a time"
        );
        worker
            .read_preview(local, "/private/tmp/p.png".into())
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        let (mut registered, mut preview) = (false, false);
        while worker.pending() {
            match worker.poll() {
                Some(LibraryEvent::Registered { result, .. }) => {
                    assert!(result.unwrap_err().contains("read-only"));
                    registered = true;
                }
                Some(LibraryEvent::Preview { result, .. }) => {
                    assert!(result.unwrap_err().contains("cannot be decoded"));
                    preview = true;
                }
                _ => {}
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(registered && preview && !worker.registering());
    }
}
