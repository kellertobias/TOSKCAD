//! Native picker seam. Dialogs select destinations; the session-bound file
//! worker still owns reading, validation and atomic publication.
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    OpenProject,
    SaveProject,
    ImportSource,
    ImportArtwork,
    ImportStep,
    ImportKicad,
    PreviewImage,
    ImportHeightMap,
    /// Save a 3MF print package from Export → Print.
    SavePrintPackage,
    /// A schedule CSV destination.
    SaveCsv,
    /// An existing folder that receives an export bundle.
    ChooseFolder,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub id: u64,
    pub kind: Kind,
    pub directory: Option<PathBuf>,
    pub suggested_name: Option<String>,
}
pub struct Reply {
    pub id: u64,
    pub result: Result<Option<PathBuf>, String>,
}
pub trait FilePicker {
    /// Called on the window thread, allowing the adapter to attach its parent.
    /// The native dialog and its completion must not block an application frame.
    fn start(&mut self, request: Request) -> Result<(), String>;
    fn poll(&mut self) -> Option<Reply>;
}
