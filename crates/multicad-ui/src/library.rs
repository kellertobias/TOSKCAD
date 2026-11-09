//! The single installation library and project cards. Add to Library is an
//! explicit user action that lists a saved project file at its original
//! location. Included definitions share this browser without entering the
//! writable index. The library never copies or edits an original, never runs on
//! Save and never changes definitions already embedded (pinned) in consuming
//! projects. File-system access belongs to the application's `LibraryStore`;
//! this module owns the nonblocking worker and preview textures.
pub(crate) mod facets;

use crate::storage::FileStamp;
use eframe::egui;
use multicad_core::{
    document::EntityId,
    project_file::{ContentHash, ProjectCard, info::PreviewImage},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
};

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    pub origin: LibraryOrigin,
    /// Stable entry identity within this installation's library.
    pub id: EntityId,
    /// Document identity of the listed project; one entry per document.
    pub document: EntityId,
    /// Included project or listed original's absolute location; last known
    /// location when unavailable.
    pub path: PathBuf,
    /// Read live from the file; the last known values when it is missing.
    pub name: String,
    pub description: String,
    pub preview: Option<PreviewImage>,
    pub registered_unix_ms: u64,
    pub symbol_definitions: usize,
    pub tags: Vec<String>,
    pub mechanical_parts: Vec<multicad_core::model::MechanicalPartCard>,
    /// Set when the file is missing, unreadable or now holds another project.
    pub problem: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryOrigin {
    ListedOriginal,
    Bundled,
}
/// Validate local Library tags independently from the project file.
pub fn library_tags(tags: impl IntoIterator<Item = String>) -> Result<Vec<String>, String> {
    let mut unique = BTreeSet::new();
    let mut result = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > 32 || tag.contains(',') || tag.chars().any(char::is_control) {
            return Err("Use tags of up to 32 characters, separated by commas.".into());
        }
        if unique.insert(tag.to_lowercase()) {
            result.push(tag.to_owned());
        }
        if result.len() > 12 {
            return Err("A Library project can have up to 12 tags.".into());
        }
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct RegisterRequest {
    pub path: PathBuf,
    /// The saved file this window last wrote or opened; registration refuses
    /// a file that changed on disk since then.
    pub stamp: FileStamp,
    pub document: EntityId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Registration {
    Added,
    /// This document is already listed at this location; nothing changed.
    AlreadyListed,
    /// This document was listed at another location (moved file or Save As
    /// copy); the entry now points at the new location.
    Relocated {
        previous: PathBuf,
    },
}
#[derive(Clone, Debug)]
pub struct RegisterReceipt {
    pub outcome: Registration,
    pub entry: LibraryEntry,
}
impl RegisterReceipt {
    pub fn message(&self) -> String {
        let name = &self.entry.name;
        let path = self.entry.path.display();
        match &self.outcome {
            Registration::Added => format!("Listed “{name}” in the library · {path}"),
            Registration::AlreadyListed => {
                format!("“{name}” is already listed in the library at {path} · nothing changed")
            }
            Registration::Relocated { previous } => format!(
                "The library now lists “{name}” at {path} instead of {}",
                previous.display()
            ),
        }
    }
}
pub trait LibraryStore: Send + 'static {
    fn load(&mut self) -> Result<Vec<LibraryEntry>, String>;
    fn register(
        &mut self,
        request: &RegisterRequest,
    ) -> Result<(RegisterReceipt, Vec<LibraryEntry>), String>;
    /// Remove one listing; the file itself is never touched.
    fn remove(&mut self, entry: EntityId) -> Result<Vec<LibraryEntry>, String>;
    fn set_tags(&mut self, _entry: EntityId, _tags: Vec<String>) -> Result<Vec<String>, String> {
        Err("Editing Library tags is unavailable in this host".into())
    }
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
    Remove(EntityId),
    SetTags { entry: EntityId, tags: Vec<String> },
    Register(RegisterRequest),
    Card(PathBuf),
    Preview { local: EntityId, path: PathBuf },
}
pub enum LibraryEvent {
    Loaded(Result<Vec<LibraryEntry>, String>),
    Removed(Result<Vec<LibraryEntry>, String>),
    Tags {
        entry: EntityId,
        result: Result<Vec<String>, String>,
    },
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
                        Job::Remove(id) => LibraryEvent::Removed(store.remove(id)),
                        Job::SetTags { entry, tags } => LibraryEvent::Tags {
                            entry,
                            result: store.set_tags(entry, tags),
                        },
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
    pub fn remove(&mut self, entry: EntityId) -> Result<(), String> {
        self.submit(Job::Remove(entry))
    }
    pub fn set_tags(&mut self, entry: EntityId, tags: Vec<String>) -> Result<(), String> {
        self.submit(Job::SetTags { entry, tags })
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
pub(crate) const PROJECT_CARD_GAP: f32 = 12.;
pub(crate) const PROJECT_CARD_PADDING: f32 = 10.;

pub(crate) fn project_card_columns(width: f32) -> (usize, f32) {
    let gap = PROJECT_CARD_GAP;
    let columns = (((width + gap) / (240. + gap)).floor() as usize).max(1);
    (
        columns,
        (width - gap * (columns - 1) as f32) / columns as f32,
    )
}

pub(crate) fn project_card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(33, 44, 60))
        .inner_margin(PROJECT_CARD_PADDING)
        .corner_radius(5)
}

pub(crate) fn project_card_preview(width: f32) -> egui::Vec2 {
    let width = (width - 2. * PROJECT_CARD_PADDING).max(1.);
    egui::vec2(width, width * 9. / 16.)
}

pub(crate) fn paint_preview_placeholder(ui: &mut egui::Ui, size: egui::Vec2, label: &str) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        preview_placeholder(ui, rect, label);
    }
}

fn preview_placeholder(ui: &egui::Ui, rect: egui::Rect, label: &str) {
    ui.painter()
        .rect_filled(rect, 4, egui::Color32::from_rgb(36, 46, 60));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.),
        egui::Color32::GRAY,
    );
}

type ThumbnailResult = (ContentHash, Result<egui::ColorImage, String>);

/// Small, bounded worker for thumbnails requested by visible Library tiles.
struct ThumbnailWorker {
    tx: SyncSender<(ContentHash, PreviewImage)>,
    rx: Receiver<ThumbnailResult>,
    pending: BTreeSet<ContentHash>,
}

impl ThumbnailWorker {
    fn start(ctx: egui::Context) -> Result<Self, String> {
        let (tx, jobs) = mpsc::sync_channel::<(ContentHash, PreviewImage)>(6);
        let (results, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("multicad-library-thumbnails".into())
            .spawn(move || {
                while let Ok((key, image)) = jobs.recv() {
                    if results
                        .send((key, decode_preview(&image).map(thumbnail)))
                        .is_err()
                    {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            tx,
            rx,
            pending: BTreeSet::new(),
        })
    }
}

fn thumbnail(image: egui::ColorImage) -> egui::ColorImage {
    const MAX_EDGE: usize = 480;
    let [width, height] = image.size;
    let edge = width.max(height);
    if edge <= MAX_EDGE {
        return image;
    }
    let w = (width * MAX_EDGE / edge).max(1);
    let h = (height * MAX_EDGE / edge).max(1);
    let pixels = (0..h)
        .flat_map(|y| {
            let row = &image.pixels[(y * height / h) * width..][..width];
            (0..w).map(move |x| row[x * width / w])
        })
        .collect();
    egui::ColorImage::new([w, h], pixels)
}

#[derive(Default)]
pub(crate) struct LibraryThumbnails {
    worker: Option<Result<ThumbnailWorker, String>>,
    textures: BTreeMap<ContentHash, Result<egui::TextureHandle, String>>,
    order: VecDeque<ContentHash>,
}

impl LibraryThumbnails {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        image: Option<&PreviewImage>,
        size: egui::Vec2,
        name: &str,
    ) {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let Some(image) = image else {
            preview_placeholder(ui, rect, "No preview image");
            return;
        };
        let worker = self
            .worker
            .get_or_insert_with(|| ThumbnailWorker::start(ui.ctx().clone()));
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                preview_placeholder(ui, rect, "Preview unavailable");
                response.on_hover_text(error.as_str());
                return;
            }
        };
        while let Ok((key, decoded)) = worker.rx.try_recv() {
            worker.pending.remove(&key);
            if self.order.len() >= 64
                && let Some(old) = self.order.pop_front()
            {
                self.textures.remove(&old);
            }
            self.order.push_back(key);
            let texture = decoded.map(|image| {
                ui.ctx()
                    .load_texture(format!("library-preview-{key}"), image, Default::default())
            });
            self.textures.insert(key, texture);
        }
        let key = image.content();
        match self.textures.get(&key) {
            Some(Ok(texture)) => {
                let dimensions = egui::vec2(image.width() as f32, image.height() as f32);
                let fitted = dimensions * (size.x / dimensions.x).min(size.y / dimensions.y);
                let image_rect = egui::Rect::from_center_size(rect.center(), fitted);
                ui.painter().rect_filled(rect, 0, crate::theme::PANEL);
                ui.new_child(egui::UiBuilder::new().max_rect(image_rect))
                    .put(
                        image_rect,
                        egui::Image::new((texture.id(), fitted)).alt_text(name),
                    );
            }
            Some(Err(error)) => {
                preview_placeholder(ui, rect, "Preview unavailable");
                response.on_hover_text(error);
            }
            None => {
                if !worker.pending.contains(&key) {
                    match worker.tx.try_send((key, image.clone())) {
                        Ok(()) => {
                            worker.pending.insert(key);
                        }
                        Err(mpsc::TrySendError::Full(_)) => {}
                        Err(mpsc::TrySendError::Disconnected(_)) => {
                            preview_placeholder(ui, rect, "Preview unavailable");
                            return;
                        }
                    }
                }
                preview_placeholder(ui, rect, "Loading preview…");
            }
        }
    }
}

pub(crate) const PROJECT_PREVIEW_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(12, 17, 24);

/// Recognize the exact scanline palette of the older example rasterizer after
/// its 2:1 preview resize. Arbitrary photos and viewport captures stay intact.
fn darken_example_background(image: &mut egui::ColorImage) {
    const SIZE: [usize; 2] = [640, 480];
    if image.size != SIZE {
        return;
    }
    let legacy_gray = |row: usize| (246. - 22. * row as f64 / 960.) as u8;
    let legacy = |row: usize| {
        let gray = (u16::from(legacy_gray(2 * row)) + u16::from(legacy_gray(2 * row + 1))) / 2;
        egui::Color32::from_rgb(gray as u8, gray as u8, (gray + 4) as u8)
    };
    let (rows, remainder) = image.pixels.as_chunks_mut::<{ SIZE[0] }>();
    if !remainder.is_empty()
        || rows.len() != SIZE[1]
        || !rows.iter().enumerate().all(|(y, row)| {
            let background = legacy(y);
            row[0] == background && row[SIZE[0] - 1] == background
        })
    {
        return;
    }
    for (y, row) in rows.iter_mut().enumerate() {
        let background = legacy(y);
        for pixel in row {
            if *pixel == background {
                *pixel = PROJECT_PREVIEW_BACKGROUND;
            }
        }
    }
}

/// Decoded preview textures keyed by image content.
#[derive(Default)]
pub struct PreviewTextures {
    textures: BTreeMap<(ContentHash, bool), Result<egui::TextureHandle, String>>,
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
        self.show_preview(ui, image, size, alt, false, false);
    }

    /// Reserve the full frame and center the image at its original aspect ratio.
    pub(crate) fn show_framed(
        &mut self,
        ui: &mut egui::Ui,
        image: Option<&PreviewImage>,
        size: egui::Vec2,
        alt: &str,
    ) {
        self.show_preview(ui, image, size, alt, true, false);
    }

    pub(crate) fn show_project_preview(
        &mut self,
        ui: &mut egui::Ui,
        image: Option<&PreviewImage>,
        size: egui::Vec2,
        automatic: bool,
    ) {
        self.show_preview(ui, image, size, "Project preview image", false, automatic);
    }

    fn show_preview(
        &mut self,
        ui: &mut egui::Ui,
        image: Option<&PreviewImage>,
        size: egui::Vec2,
        alt: &str,
        framed: bool,
        automatic: bool,
    ) {
        let fitted = image.map_or(size, |image| {
            let dimensions = egui::vec2(image.width() as f32, image.height() as f32);
            dimensions * (size.x / dimensions.x).min(size.y / dimensions.y)
        });
        let (rect, response) =
            ui.allocate_exact_size(if framed { size } else { fitted }, egui::Sense::hover());
        // Scroll layout still needs the image's extent, but offscreen entries
        // need neither a content hash, PNG decode nor GPU texture allocation.
        if !ui.is_rect_visible(rect) {
            return;
        }
        let texture = image.map(|image| {
            let ctx = ui.ctx().clone();
            self.textures
                .entry((image.content(), automatic))
                .or_insert_with(|| {
                    decode_preview(image).map(|mut decoded| {
                        if automatic {
                            darken_example_background(&mut decoded);
                        }
                        ctx.load_texture(
                            format!("file-preview-{}-{automatic}", image.content()),
                            decoded,
                            Default::default(),
                        )
                    })
                })
                .clone()
        });
        match texture {
            Some(Ok(texture)) => {
                let preview = egui::Image::new((texture.id(), fitted)).alt_text(alt);
                if framed {
                    ui.painter().rect_filled(rect, 0, crate::theme::PANEL);
                    let image_rect = egui::Rect::from_center_size(rect.center(), fitted);
                    // The frame already advanced the layout. A short image
                    // must not pull the following title into that frame.
                    ui.new_child(egui::UiBuilder::new().max_rect(image_rect))
                        .put(image_rect, preview);
                } else {
                    ui.put(rect, preview);
                }
            }
            other => {
                let text = match other {
                    Some(Err(_)) => "Preview unavailable",
                    _ => "No preview image",
                };
                preview_placeholder(ui, rect, text);
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
    #[test]
    fn legacy_project_preview_gets_a_dark_matte_without_recoloring_ordinary_images() {
        use multicad_core::project_file;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/toolcase.tcd");
        let mut file = std::fs::File::open(path).unwrap();
        let card = project_file::card(
            &mut file,
            project_file::FileLimits::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let preview = card.info.preview.unwrap();
        let original = decode_preview(&preview).unwrap();
        assert_eq!(original.pixels[0], PROJECT_PREVIEW_BACKGROUND);
        let mut display = egui::ColorImage::filled([640, 480], egui::Color32::WHITE);
        let (rows, _) = display.pixels.as_chunks_mut::<640>();
        let gray = |row: usize| (246. - 22. * row as f64 / 960.) as u16;
        for (row, pixels) in rows.iter_mut().enumerate() {
            let g = ((gray(row * 2) + gray(row * 2 + 1)) / 2) as u8;
            pixels.fill(egui::Color32::from_rgb(g, g, g + 4));
        }
        let model_pixel = original.pixels[240 * 640 + 320];
        display.pixels[240 * 640 + 320] = model_pixel;
        darken_example_background(&mut display);
        assert_eq!(display.pixels[0], PROJECT_PREVIEW_BACKGROUND);
        assert_eq!(display.pixels[240 * 640 + 320], model_pixel);
        assert_eq!(decode_preview(&preview).unwrap(), original);

        let mut custom = egui::ColorImage::filled([640, 480], egui::Color32::WHITE);
        darken_example_background(&mut custom);
        assert!(
            custom
                .pixels
                .iter()
                .all(|&pixel| pixel == egui::Color32::WHITE)
        );
    }

    #[test]
    fn offscreen_previews_keep_layout_without_decoding_and_load_when_visible() {
        let image = PreviewImage::from_png(
            include_bytes!("../../../assets/branding/app/AppIcon.png").as_slice(),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut textures = PreviewTextures::default();
        let mut cursors = vec![];
        for visible in [false, true, true] {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                if !visible {
                    ui.set_clip_rect(egui::Rect::NOTHING);
                }
                textures.show(ui, Some(&image), egui::vec2(64., 48.), "Preview");
                cursors.push(ui.cursor());
            });
            output.drop_without_applying_deltas();
            assert_eq!(textures.textures.len(), usize::from(visible));
        }
        assert_eq!(cursors[0], cursors[1]);
        assert_eq!(cursors[1], cursors[2]);
        assert!(textures.textures.values().all(Result::is_ok));
    }
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
        fn remove(&mut self, _: EntityId) -> Result<Vec<LibraryEntry>, String> {
            Ok(vec![])
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
