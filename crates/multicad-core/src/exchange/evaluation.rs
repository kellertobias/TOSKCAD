use super::*;
use crate::{commands::Actor, document::EntityId, project_file::ContentHash};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewKey {
    pub source: ExportStamp,
    pub set: EntityId,
    pub plan_hash: ContentHash,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "reason",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RecipeStatus {
    Ready,
    Empty,
    Unsupported(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeReport {
    pub recipe: EntityId,
    pub status: RecipeStatus,
    pub files: usize,
    /// Selected Parts this recipe left out because they are removable FDM
    /// aids and the recipe excludes removable aids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_aids: Vec<PartRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    ConsumedProducer,
    NoCurrentSolid,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludedPart {
    pub reference: PartRef,
    pub reason: ExclusionReason,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportArtifact {
    pub output: NamedOutput,
    pub producer: EntityId,
    pub solid: EntityId,
    pub bounds_mm: [[f64; 3]; 2],
    pub volume_mm3: f64,
    pub solid_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planar: Option<PlanarEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawing: Option<DrawingEvidence>,
}
/// What a drawing output contains, in page order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingEvidence {
    pub pages: Vec<PageRef>,
    pub page_sizes_mm: Vec<[f64; 2]>,
    pub vectors: usize,
    pub texts: usize,
}
/// What a mesh output contains: resource objects, placed build items and the
/// tessellation actually written, in millimeters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshEvidence {
    pub units: super::LengthUnit,
    pub objects: usize,
    pub build_items: usize,
    pub triangles: usize,
    pub mesh_volume_mm3: f64,
    pub chord_deviation_mm: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarEvidence {
    pub frame: crate::references::ReferenceFrame,
    pub bounds_mm: [[f64; 2]; 2],
    pub area_mm2: f64,
    pub outer_loops: usize,
    pub inner_loops: usize,
    pub geometry_hash: ContentHash,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPreview {
    pub requested_by: Actor,
    pub key: PreviewKey,
    pub provider: String,
    pub selection: ResolvedSet,
    pub recipes: Vec<RecipeReport>,
    pub artifacts: Vec<ExportArtifact>,
    pub excluded: Vec<ExcludedPart>,
}
impl ExportPreview {
    pub fn can_run(&self) -> bool {
        !self.artifacts.is_empty() && self.recipes.iter().all(|r| r.status == RecipeStatus::Ready)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WrittenFile {
    pub artifact: ExportArtifact,
    pub content_hash: ContentHash,
    pub bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReceipt {
    pub actor: Actor,
    pub key: PreviewKey,
    pub directory: String,
    pub files: Vec<WrittenFile>,
    pub reused: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExportResult {
    Preview(Result<Box<ExportPreview>, ExportError>),
    Run(Result<RunReceipt, ExportError>),
    PlanarCatalog(Result<Box<PlanarCatalog>, ExportError>),
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportReply {
    pub id: EntityId,
    pub document: EntityId,
    pub revision: u64,
    pub actor: Actor,
    pub result: ExportResult,
}
