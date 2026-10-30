//! Compiled native file dialogs, parented to the application window. The UI
//! receives only a chosen path or cancellation, never platform window handles.
use multicad_ui::{
    CreationContext,
    file_picker::{FilePicker, Kind, Reply, Request},
};
use std::sync::mpsc::{self, Receiver, TryRecvError};
pub struct NativePicker {
    parent: rfd::AsyncFileDialog,
    pending: Option<(u64, Receiver<Reply>)>,
}
impl NativePicker {
    pub fn new(context: &CreationContext<'_>) -> Self {
        Self {
            parent: rfd::AsyncFileDialog::new().set_parent(context),
            pending: None,
        }
    }
}
impl FilePicker for NativePicker {
    fn start(&mut self, request: Request) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("A native file dialog is already open".into());
        }
        let mut dialog = self.parent.clone().set_title(match request.kind {
            Kind::OpenProject => "Open ToskCAD project",
            Kind::SaveProject => "Save ToskCAD project",
            Kind::ImportSource => "Import source snapshot",
            Kind::ImportArtwork => "Import DXF or SVG artwork",
            Kind::ImportStep => "Import or update a STEP file",
            Kind::ImportKicad => "Import or update a KiCad board",
            Kind::PreviewImage => "Choose a PNG preview image",
        });
        dialog = match request.kind {
            Kind::ImportSource => dialog.add_filter("Source snapshot", &["json"]),
            Kind::ImportArtwork => dialog.add_filter("DXF or SVG artwork", &["dxf", "svg"]),
            Kind::ImportStep => dialog.add_filter("STEP file", &["step", "stp", "STEP", "STP"]),
            Kind::ImportKicad => dialog.add_filter("KiCad board", &["kicad_pcb"]),
            Kind::PreviewImage => dialog.add_filter("PNG image", &["png"]),
            _ => dialog.add_filter("ToskCAD project", &["multicad"]),
        };
        if let Some(path) = &request.directory {
            dialog = dialog.set_directory(path);
        }
        if let Some(name) = &request.suggested_name {
            dialog = dialog.set_file_name(name);
        }
        // Start on the window thread: macOS creates an attached native sheet.
        // Poll its future on a bounded worker; AppKit dispatch stays in rfd.
        let future: std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>,
        > = if request.kind == Kind::SaveProject {
            Box::pin(dialog.save_file())
        } else {
            Box::pin(dialog.pick_file())
        };
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("multicad-file-picker".into())
            .spawn(move || {
                let picked = pollster::block_on(future).map(|file| {
                    let mut path = file.path().to_path_buf();
                    if request.kind == Kind::SaveProject && path.extension().is_none() {
                        path.set_extension("multicad");
                    }
                    path
                });
                let _ = tx.send(Reply {
                    id: request.id,
                    result: Ok(picked),
                });
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some((request.id, rx));
        Ok(())
    }
    fn poll(&mut self) -> Option<Reply> {
        let (id, rx) = self.pending.as_ref()?;
        let reply = match rx.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Reply {
                id: *id,
                result: Err("The native file dialog closed without a result".into()),
            },
        };
        self.pending = None;
        Some(reply)
    }
}
