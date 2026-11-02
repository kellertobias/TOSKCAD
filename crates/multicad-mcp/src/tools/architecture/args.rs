use super::*;
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectArgs {
    /// Omit to list embedded architectural definitions; otherwise inspect one definition or placed output.
    pub entity: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionTarget {
    WallHeight,
    WallLayerThickness { layer: String },
    FenceHeight,
    FenceSpacing,
    FencePostWidth,
    FencePostDepth,
    FencePostDiameter,
    FenceInfillBottom { recipe: String },
    FenceInfillHeight { recipe: String },
    FenceInfillOffset { recipe: String },
    FencePanelThickness { recipe: String },
    FencePurchasedPanelWidth { recipe: String },
    FenceRailDepth { recipe: String },
    FencePicketWidth { recipe: String },
    FencePicketDepth { recipe: String },
    FencePicketGap { recipe: String },
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterialTarget {
    WallLayer { layer: String },
    FencePost,
    FenceInfill { recipe: String },
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OptionTarget {
    WallReference,
    FenceLayout,
    FenceShortPanel { recipe: String },
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Outside,
    Inside,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reference {
    Outside,
    Center,
    Inside,
    Boundary { layer: String, side: Side },
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Equal,
    ShortLast,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShortPanel {
    Reject,
    Cut,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OptionValue {
    WallReference { reference: Reference },
    FenceLayout { layout: Layout },
    FenceShortPanel { policy: ShortPanel },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionSlot {
    pub id: Option<String>,
    pub name: String,
    pub target: DimensionTarget,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialSlot {
    pub id: Option<String>,
    pub name: String,
    pub target: MaterialTarget,
    pub choices: Vec<String>,
    pub allow_project_materials: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptionSlot {
    pub id: Option<String>,
    pub name: String,
    pub target: OptionTarget,
    pub choices: Vec<OptionValue>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthorArgs {
    pub expected_revision: u64,
    /// Live layered-wall or fence construction, never a sample wall/run or its generated parts.
    pub construction: String,
    /// Omit for a new interface. Retain the returned identity for a later authored revision.
    pub blueprint: Option<String>,
    /// Retain the returned symbol identity when publishing a new revision.
    pub symbol: Option<String>,
    pub name: String,
    pub revision: String,
    pub dimensions: Vec<DimensionSlot>,
    pub materials: Vec<MaterialSlot>,
    pub options: Vec<OptionSlot>,
    pub exposed_parameters: Vec<String>,
}
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
    /// Complete overrides, keyed by exposed source parameter ID. Omitted entries inherit the pin.
    #[serde(default)]
    pub parameters: BTreeMap<String, Numeric>,
    /// Complete overrides, keyed by public slot ID; formulas bind receiving-project parameters.
    #[serde(default)]
    pub dimensions: BTreeMap<String, Quantity>,
    #[serde(default)]
    pub materials: BTreeMap<String, MaterialValue>,
    #[serde(default)]
    pub options: BTreeMap<String, OptionValue>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Numeric {
    pub value: f64,
    pub unit: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterialValue {
    Preset { material: String },
    Project { material: String },
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum Context {
    Wall {
        path: Vec<[Quantity; 2]>,
        closed: bool,
        level: Option<String>,
        base_z: Quantity,
        flipped: bool,
    },
    Fence {
        vertices: Vec<super::fences::VertexArg>,
        closed: bool,
        level: Option<String>,
        base_z: Quantity,
        post_positions: Vec<super::fences::PositionArg>,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceArgs {
    pub expected_revision: u64,
    pub definition: String,
    pub name: String,
    pub inputs: Inputs,
    pub context: Context,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    pub expected_revision: u64,
    /// Architectural record or any output body/group/member reported by inspection.
    pub entity: String,
    pub inputs: Inputs,
    /// Omit to retain the canonical receiving path/context and all wall openings/joins.
    pub context: Option<Context>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibrarySource {
    /// Bare .tcd file name inside the server's configured exchange/export folder.
    pub file_name: String,
    /// Embedded source definition ID. Omit only when the file contains exactly one architectural definition.
    pub source_definition: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportArgs {
    pub expected_revision: u64,
    pub source: LibrarySource,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareArgs {
    pub definition: String,
    pub source: LibrarySource,
    /// Optional complete replacement public overrides for individual records. Other records retain their current values.
    #[serde(default)]
    pub inputs: BTreeMap<String, Inputs>,
    /// Optional receiving path/context repairs composed with the pin update.
    #[serde(default)]
    pub contexts: BTreeMap<String, Context>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    pub expected_revision: u64,
    pub update: CompareArgs,
}
