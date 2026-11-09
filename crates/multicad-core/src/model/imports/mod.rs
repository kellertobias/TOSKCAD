//! Mechanical imports. A STEP source keeps each imported solid's exact
//! geometry with a source identity and signature; one `import.step_body`
//! feature per Part rebuilds it. A KiCad board becomes an `import.board`
//! Part (outline, cutouts and drilled holes) and one `import.component` Part
//! per footprint: its linked STEP model, or an envelope from its courtyard
//! when the model is missing. Imported features are marked as imports, not
//! editable modeling history. Reimports match source identities and keep the
//! Parts of everything that still matches.
pub mod kicad;

use super::{
    BodyOutput, CreatedBody, ModelPrepared,
    common::{error, revision},
    source::invalid,
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    project_file::ContentHash,
};
pub use kicad::{BoardHole, Footprint, KicadBoard, ModelReference, OutlineEdge};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const STEP_SOURCE_KIND: &str = "import.step_source";
pub const STEP_SOURCE_CAPABILITY: &str = "import.step_source.1";
pub const RUST_STEP_SOURCE_CAPABILITY: &str = "import.step_source.2";
pub const STEP_BODY_TYPE: &str = "import.step_body";
pub const STEP_BODY_CAPABILITY: &str = "import.step_body.1";
pub const BOARD_TYPE: &str = "import.board";
pub const BOARD_CAPABILITY: &str = "import.board.1";
pub const COMPONENT_TYPE: &str = "import.component";
pub const COMPONENT_CAPABILITY: &str = "import.component.1";
/// Total geometry text one STEP source may store in the document.
pub const MAX_STEP_BREP_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STEP_SOLIDS: usize = 512;
pub const MAX_COMPONENTS: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepEncoding {
    #[default]
    OpenCascadeBrep,
    RustStep,
}

impl StepEncoding {
    fn is_native(&self) -> bool {
        *self == Self::OpenCascadeBrep
    }

    fn schema_version(self) -> u32 {
        match self {
            Self::OpenCascadeBrep => 1,
            Self::RustStep => 2,
        }
    }
}

/// Volume and bounds of a solid in millimeters, used to match solids on
/// reimport.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolidSignature {
    pub volume_mm3: f64,
    pub min: [f64; 3],
    pub max: [f64; 3],
}
impl SolidSignature {
    fn same_shape(&self, other: &Self) -> bool {
        let close = |a: f64, b: f64, t: f64| (a - b).abs() <= t;
        close(
            self.volume_mm3,
            other.volume_mm3,
            1e-6 * self.volume_mm3.abs().max(1.),
        ) && (0..3).all(|i| {
            close(self.min[i], other.min[i], 1e-3) && close(self.max[i], other.max[i], 1e-3)
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedSolid {
    /// Stable within the source across reimports.
    pub id: String,
    pub name: String,
    pub signature: SolidSignature,
    /// Exact embedded geometry in the source's declared encoding. The field
    /// name is retained for compatibility with the native source schema.
    pub brep: String,
    /// The solid was not found in the latest reimport; its last geometry is
    /// kept so its Part and links survive.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSource {
    pub file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub content: ContentHash,
    #[serde(default, skip_serializing_if = "StepEncoding::is_native")]
    pub encoding: StepEncoding,
    pub solids: Vec<ImportedSolid>,
}
/// A STEP file with verified embedded geometry, before it enters a document.
#[derive(Clone, Debug, PartialEq)]
pub struct StepFile {
    pub file_name: String,
    pub path: Option<String>,
    pub content: ContentHash,
    pub encoding: StepEncoding,
    /// (name, embedded geometry text, signature) per solid.
    pub solids: Vec<(String, String, SolidSignature)>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepBodyDefinition {
    pub output: BodyOutput,
    pub source: EntityId,
    pub solid: String,
}
/// The board outline in board coordinates (KiCad millimeters, Y up), bottom
/// at z = 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardDefinition {
    pub output: BodyOutput,
    pub file_name: String,
    pub content: ContentHash,
    pub thickness_mm: f64,
    pub outer: Vec<OutlineEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<Vec<OutlineEdge>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<BoardHole>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentDefinition {
    pub output: BodyOutput,
    pub board: EntityId,
    pub footprint: Footprint,
    /// Envelope height above (or below, on the back) the board, used when no
    /// model is linked.
    pub height_mm: f64,
    /// The STEP source of the footprint's 3D model, when it was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<EntityId>,
    /// The footprint is gone from the latest board reimport.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}

fn definition<T: Serialize>(type_id: &str, data: &T) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: type_id.into(),
        schema_version: 1,
        data: serde_json::to_value(data).map_err(|e| invalid(e.to_string()))?,
    })
}
fn parse<T: for<'a> Deserialize<'a>>(d: &Definition, type_id: &str) -> Result<T, DocumentError> {
    if d.type_id != type_id || d.schema_version != 1 {
        return Err(invalid(format!("Expected {type_id} schema 1")));
    }
    serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))
}
fn feature_definition(
    document: &Document,
    id: EntityId,
) -> Result<(&Definition, &[Link]), DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Feature {
            definition, inputs, ..
        }) => Ok((definition, inputs)),
        _ => Err(invalid("Choose an import feature")),
    }
}
fn check_links(inputs: &[Link], expected: &[Link], what: &str) -> Result<(), DocumentError> {
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid(format!(
            "{what} source and canonical input links disagree"
        )));
    }
    Ok(())
}
fn finite(v: f64) -> bool {
    v.is_finite() && v.abs() <= 1e9
}

impl StepSource {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.solids.is_empty() || self.solids.len() > MAX_STEP_SOLIDS {
            return Err(invalid(format!(
                "A STEP import needs 1–{MAX_STEP_SOLIDS} solids"
            )));
        }
        let bytes: usize = self.solids.iter().map(|s| s.brep.len()).sum();
        if bytes > MAX_STEP_BREP_BYTES {
            return Err(invalid(format!(
                "The STEP geometry is {:.1} MiB; imports are limited to {} MiB per file",
                bytes as f64 / 1048576.,
                MAX_STEP_BREP_BYTES / 1048576
            )));
        }
        let mut ids = BTreeSet::new();
        if self
            .solids
            .iter()
            .any(|s| s.id.is_empty() || !ids.insert(&s.id))
        {
            return Err(invalid("STEP solids need distinct identities"));
        }
        if self.encoding == StepEncoding::RustStep
            && self.solids.iter().any(|solid| {
                let sig = solid.signature;
                solid.brep.is_empty()
                    || !sig.volume_mm3.is_finite()
                    || sig.volume_mm3 <= 0.
                    || sig
                        .min
                        .into_iter()
                        .zip(sig.max)
                        .any(|(lo, hi)| !lo.is_finite() || !hi.is_finite() || lo >= hi)
            })
        {
            return Err(invalid(
                "Rust STEP solids need geometry and a finite, positive signature",
            ));
        }
        Ok(())
    }
    pub fn solid(&self, id: &str) -> Option<&ImportedSolid> {
        self.solids.iter().find(|s| s.id == id)
    }
    fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        let mut definition = definition(STEP_SOURCE_KIND, self)?;
        definition.schema_version = self.encoding.schema_version();
        Ok(EntityData::Extension {
            definition,
            inputs: vec![],
        })
    }
}
impl StepBodyDefinition {
    fn inputs(&self) -> Vec<Link> {
        vec![Link::new(self.source, "source")]
    }
}
impl BoardDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if !finite(self.thickness_mm) || self.thickness_mm <= 0. || self.outer.is_empty() {
            return Err(invalid("A board needs an outline and a positive thickness"));
        }
        if self
            .holes
            .iter()
            .any(|h| !finite(h.diameter) || h.diameter <= 0.)
        {
            return Err(invalid("Board holes need positive diameters"));
        }
        Ok(())
    }
}
impl ComponentDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let [lo, hi] = self.footprint.envelope;
        if !finite(self.height_mm) || self.height_mm <= 0. || hi[0] <= lo[0] || hi[1] <= lo[1] {
            return Err(invalid(
                "A component envelope needs positive size and height",
            ));
        }
        Ok(())
    }
    fn inputs(&self) -> Vec<Link> {
        let mut links = vec![Link::new(self.board, "board")];
        links.extend(self.model.map(|m| Link::new(m, "model")));
        links
    }
}

pub fn step_sources(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == STEP_SOURCE_KIND))
}
fn features_of<'a>(
    document: &'a Document,
    type_id: &'a str,
) -> impl Iterator<Item = &'a Entity> + 'a {
    document.entities().filter(move |e| matches!(&e.data, EntityData::Feature { definition, .. } if definition.type_id == type_id))
}
pub fn step_bodies(document: &Document) -> impl Iterator<Item = &Entity> {
    features_of(document, STEP_BODY_TYPE)
}
pub fn boards(document: &Document) -> impl Iterator<Item = &Entity> {
    features_of(document, BOARD_TYPE)
}
pub fn components(document: &Document) -> impl Iterator<Item = &Entity> {
    features_of(document, COMPONENT_TYPE)
}
pub fn read_step_source(document: &Document, id: EntityId) -> Result<StepSource, DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, .. }) => {
            if definition.type_id != STEP_SOURCE_KIND || !matches!(definition.schema_version, 1 | 2)
            {
                return Err(invalid("Expected a supported STEP source schema"));
            }
            let value: StepSource = serde_json::from_value(definition.data.clone())
                .map_err(|e| invalid(e.to_string()))?;
            if value.encoding.schema_version() != definition.schema_version {
                return Err(invalid("STEP source encoding does not match its schema"));
            }
            value.validate()?;
            Ok(value)
        }
        Some(_) => Err(invalid(format!("{id} is not a STEP import"))),
        None => Err(DocumentError::Missing(id)),
    }
}
pub fn read_step_body(
    document: &Document,
    id: EntityId,
) -> Result<StepBodyDefinition, DocumentError> {
    let (d, inputs) = feature_definition(document, id)?;
    let value: StepBodyDefinition = parse(d, STEP_BODY_TYPE)?;
    check_links(inputs, &value.inputs(), "STEP body")?;
    Ok(value)
}
pub fn read_board(document: &Document, id: EntityId) -> Result<BoardDefinition, DocumentError> {
    let (d, inputs) = feature_definition(document, id)?;
    let value: BoardDefinition = parse(d, BOARD_TYPE)?;
    value.validate()?;
    check_links(inputs, &[], "Board")?;
    Ok(value)
}
pub fn read_component(
    document: &Document,
    id: EntityId,
) -> Result<ComponentDefinition, DocumentError> {
    let (d, inputs) = feature_definition(document, id)?;
    let value: ComponentDefinition = parse(d, COMPONENT_TYPE)?;
    value.validate()?;
    check_links(inputs, &value.inputs(), "Component")?;
    Ok(value)
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for e in step_sources(document) {
        read_step_source(document, e.id)?;
    }
    for e in step_bodies(document) {
        read_step_body(document, e.id)?;
    }
    for e in boards(document) {
        read_board(document, e.id)?;
    }
    for e in components(document) {
        read_component(document, e.id)?;
    }
    Ok(())
}

fn insert_part(
    edits: &mut Vec<Mutation>,
    name: &str,
    type_id: &str,
    data: &impl Serialize,
    inputs: Vec<Link>,
    output: &CreatedBody,
) -> Result<(), CommandError> {
    let mut feature = Entity::new(
        format!("{name} import"),
        EntityData::Feature {
            definition: definition(type_id, data)?,
            inputs,
            suppressed: false,
        },
    );
    feature.id = output.feature;
    let mut part = Entity::new(
        name,
        EntityData::Part {
            features: vec![output.feature],
            material: None,
            tags: BTreeSet::new(),
        },
    );
    part.id = output.body;
    edits.extend([Mutation::Insert(feature), Mutation::Insert(part)]);
    Ok(())
}
fn new_output() -> CreatedBody {
    CreatedBody {
        feature: EntityId::new(),
        body: EntityId::new(),
    }
}
fn solid_name(file: &str, name: &str, index: usize) -> String {
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    if name.trim().is_empty() {
        format!("{stem} solid {}", index + 1)
    } else {
        format!("{stem} {name}")
    }
}

#[derive(Clone, Debug)]
pub struct StepImportPrepared {
    pub source: EntityId,
    pub model: ModelPrepared,
}
fn step_source(file: &StepFile, ids: impl Iterator<Item = String>) -> StepSource {
    StepSource {
        file_name: file.file_name.clone(),
        path: file.path.clone(),
        content: file.content,
        encoding: file.encoding,
        solids: file
            .solids
            .iter()
            .zip(ids)
            .map(|((name, brep, signature), id)| ImportedSolid {
                id,
                name: name.clone(),
                signature: *signature,
                brep: brep.clone(),
                missing: false,
            })
            .collect(),
    }
}
/// Import every solid of a STEP file as its own Part.
pub fn prepare_step_import(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    file: &StepFile,
) -> Result<StepImportPrepared, CommandError> {
    revision(session, expected_revision)?;
    let source = step_source(file, (1..).map(|i| format!("solid-{i}")));
    let entity = Entity::new(file.file_name.clone(), source.entity_data()?);
    let id = entity.id;
    let mut edits = vec![Mutation::Insert(entity)];
    let mut outputs = vec![];
    for (i, solid) in source.solids.iter().enumerate() {
        let output = new_output();
        let body = StepBodyDefinition {
            output: BodyOutput { body: output.body },
            source: id,
            solid: solid.id.clone(),
        };
        let inputs = body.inputs();
        insert_part(
            &mut edits,
            &solid_name(&file.file_name, &solid.name, i),
            STEP_BODY_TYPE,
            &body,
            inputs,
            &output,
        )?;
        outputs.push(output);
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: format!("Import {}", file.file_name),
        edits,
    })?;
    Ok(StepImportPrepared {
        source: id,
        model: ModelPrepared { prepared, outputs },
    })
}

/// How a reimported file's solids relate to the stored ones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepUpdatePlan {
    /// (stored solid id, new solid index, geometry changed).
    pub matched: Vec<(String, usize, bool)>,
    pub added: Vec<usize>,
    /// Stored solids not found again; their Parts are kept.
    pub missing: Vec<String>,
}
/// Match a reimported file against a stored source: identical shapes first
/// (volume and bounds), then equal non-empty names with changed geometry.
pub fn plan_step_update(source: &StepSource, file: &StepFile) -> StepUpdatePlan {
    let mut plan = StepUpdatePlan::default();
    let mut used = BTreeSet::new();
    let mut matched = BTreeSet::new();
    for old in &source.solids {
        if let Some(i) = file
            .solids
            .iter()
            .enumerate()
            .position(|(i, (_, _, sig))| !used.contains(&i) && old.signature.same_shape(sig))
        {
            used.insert(i);
            matched.insert(old.id.clone());
            // The volume and bounds can stay the same while a pocket moves.
            let changed = source.encoding == StepEncoding::RustStep && old.brep != file.solids[i].1;
            plan.matched.push((old.id.clone(), i, changed));
        }
    }
    for old in &source.solids {
        if matched.contains(&old.id) || old.name.trim().is_empty() {
            continue;
        }
        if let Some(i) = file
            .solids
            .iter()
            .enumerate()
            .position(|(i, (name, _, _))| !used.contains(&i) && *name == old.name)
        {
            used.insert(i);
            matched.insert(old.id.clone());
            plan.matched.push((old.id.clone(), i, true));
        }
    }
    plan.missing = source
        .solids
        .iter()
        .filter(|s| !matched.contains(&s.id))
        .map(|s| s.id.clone())
        .collect();
    plan.added = (0..file.solids.len())
        .filter(|i| !used.contains(i))
        .collect();
    plan
}
/// Apply a reimport: matched solids keep their identities and Parts, new
/// solids get new Parts, and missing ones keep their last geometry.
pub fn prepare_step_update(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    source_id: EntityId,
    file: &StepFile,
) -> Result<(StepUpdatePlan, StepImportPrepared), CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let old = read_step_source(document, source_id)?;
    if old.encoding != file.encoding {
        return Err(error(
            "Reimport requires the same STEP geometry encoding as the original source",
        ));
    }
    let plan = plan_step_update(&old, file);
    let mut solids = vec![];
    for s in &old.solids {
        let found = plan.matched.iter().find(|(id, ..)| *id == s.id);
        solids.push(match found {
            Some((_, i, _)) => {
                let (name, brep, signature) = &file.solids[*i];
                ImportedSolid {
                    id: s.id.clone(),
                    name: name.clone(),
                    signature: *signature,
                    brep: brep.clone(),
                    missing: false,
                }
            }
            None => ImportedSolid {
                missing: true,
                ..s.clone()
            },
        });
    }
    let next = old
        .solids
        .iter()
        .filter_map(|s| s.id.strip_prefix("solid-")?.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    let mut edits = vec![];
    let mut outputs = vec![];
    for (k, &i) in plan.added.iter().enumerate() {
        let (name, brep, signature) = &file.solids[i];
        let id = format!("solid-{}", next + k + 1);
        solids.push(ImportedSolid {
            id: id.clone(),
            name: name.clone(),
            signature: *signature,
            brep: brep.clone(),
            missing: false,
        });
        let output = new_output();
        let body = StepBodyDefinition {
            output: BodyOutput { body: output.body },
            source: source_id,
            solid: id,
        };
        let inputs = body.inputs();
        insert_part(
            &mut edits,
            &solid_name(&file.file_name, name, i),
            STEP_BODY_TYPE,
            &body,
            inputs,
            &output,
        )?;
        outputs.push(output);
    }
    let source = StepSource {
        file_name: file.file_name.clone(),
        path: file.path.clone().or(old.path),
        content: file.content,
        encoding: file.encoding,
        solids,
    };
    let mut entity = document
        .get(source_id)
        .cloned()
        .ok_or_else(|| error("STEP import is missing"))?;
    entity.data = source.entity_data()?;
    edits.insert(0, Mutation::Replace(entity));
    for e in step_bodies(document) {
        if read_step_body(document, e.id)?.source == source_id {
            let part = read_step_body(document, e.id)?.output.body;
            outputs.push(CreatedBody {
                feature: e.id,
                body: part,
            });
        }
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: format!("Reimport {}", file.file_name),
        edits,
    })?;
    Ok((
        plan,
        StepImportPrepared {
            source: source_id,
            model: ModelPrepared { prepared, outputs },
        },
    ))
}

/// Options for a board import.
#[derive(Clone, Debug, PartialEq)]
pub struct KicadImport {
    pub file_name: String,
    pub content: ContentHash,
    pub board: KicadBoard,
    /// Footprint references to import as components; `None` imports all.
    pub components: Option<Vec<String>>,
    /// Envelope height for footprints without a linked model.
    pub default_height_mm: f64,
    /// Found 3D models by footprint reference, read natively.
    pub models: Vec<(String, StepFile)>,
}
#[derive(Clone, Debug)]
pub struct KicadPrepared {
    pub board: CreatedBody,
    /// (reference, component Part, linked model) per imported footprint.
    pub components: Vec<(String, CreatedBody, Option<EntityId>)>,
    /// Footprint models that were referenced but not found.
    pub missing_models: Vec<(String, String)>,
    pub unsupported: Vec<String>,
    pub model: ModelPrepared,
}
/// Import a KiCad board, its drilled holes and selected footprints.
pub fn prepare_kicad_import(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    import: &KicadImport,
) -> Result<KicadPrepared, CommandError> {
    revision(session, expected_revision)?;
    let stem = import
        .file_name
        .rsplit_once('.')
        .map_or(import.file_name.as_str(), |(s, _)| s)
        .to_owned();
    let board = BoardDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        file_name: import.file_name.clone(),
        content: import.content,
        thickness_mm: import.board.thickness_mm,
        outer: import.board.outer.clone(),
        cutouts: import.board.cutouts.clone(),
        holes: import.board.holes.clone(),
    };
    board.validate()?;
    let board_out = CreatedBody {
        feature: EntityId::new(),
        body: board.output.body,
    };
    let mut edits = vec![];
    insert_part(
        &mut edits,
        &format!("{stem} board"),
        BOARD_TYPE,
        &board,
        vec![],
        &board_out,
    )?;
    let mut outputs = vec![board_out.clone()];
    let mut components = vec![];
    let mut missing_models = vec![];
    let selected: Vec<_> = import
        .board
        .footprints
        .iter()
        .filter(|f| {
            import
                .components
                .as_ref()
                .is_none_or(|refs| refs.contains(&f.reference))
        })
        .collect();
    if let Some(refs) = &import.components
        && let Some(r) = refs
            .iter()
            .find(|r| !selected.iter().any(|f| f.reference == **r))
    {
        return Err(error(format!("The board has no footprint {r}")));
    }
    if selected.len() > MAX_COMPONENTS {
        return Err(error(format!("Import at most {MAX_COMPONENTS} components")));
    }
    for fp in selected {
        let model = match import.models.iter().find(|(r, _)| *r == fp.reference) {
            Some((_, file)) => {
                let source = step_source(file, (1..).map(|i| format!("solid-{i}")));
                let entity = Entity::new(file.file_name.clone(), source.entity_data()?);
                let id = entity.id;
                edits.push(Mutation::Insert(entity));
                Some(id)
            }
            None => {
                if let Some(m) = fp.models.first() {
                    missing_models.push((fp.reference.clone(), m.path.clone()));
                }
                None
            }
        };
        let output = new_output();
        let component = ComponentDefinition {
            output: BodyOutput { body: output.body },
            board: board_out.feature,
            footprint: fp.clone(),
            height_mm: import.default_height_mm,
            model,
            missing: false,
        };
        component.validate()?;
        let inputs = component.inputs();
        let name = if fp.reference.is_empty() {
            format!("{stem} {}", fp.library)
        } else {
            format!("{stem} {}", fp.reference)
        };
        insert_part(
            &mut edits,
            &name,
            COMPONENT_TYPE,
            &component,
            inputs,
            &output,
        )?;
        outputs.push(output.clone());
        components.push((fp.reference.clone(), output, model));
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: format!("Import {}", import.file_name),
        edits,
    })?;
    Ok(KicadPrepared {
        board: board_out,
        components,
        missing_models,
        unsupported: import.board.unsupported.clone(),
        model: ModelPrepared { prepared, outputs },
    })
}

/// What a board reimport changes, by footprint reference.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KicadUpdatePlan {
    pub outline_changed: bool,
    pub holes_changed: bool,
    pub moved: Vec<String>,
    pub unchanged: Vec<String>,
    pub added: Vec<String>,
    /// Components kept with their last placement, marked missing.
    pub missing: Vec<String>,
}
/// Reimport a board: the board Part and matching components keep their
/// identities; new footprints among `references` (or all, if `None`) are
/// added; vanished ones are kept and marked missing.
pub fn prepare_kicad_update(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    board_feature: EntityId,
    import: &KicadImport,
) -> Result<(KicadUpdatePlan, ModelPrepared), CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let old = read_board(document, board_feature)?;
    let mut plan = KicadUpdatePlan {
        outline_changed: old.outer != import.board.outer || old.cutouts != import.board.cutouts,
        holes_changed: old.holes != import.board.holes,
        ..Default::default()
    };
    let board = BoardDefinition {
        output: old.output.clone(),
        file_name: import.file_name.clone(),
        content: import.content,
        thickness_mm: import.board.thickness_mm,
        outer: import.board.outer.clone(),
        cutouts: import.board.cutouts.clone(),
        holes: import.board.holes.clone(),
    };
    board.validate()?;
    let mut entity = document
        .get(board_feature)
        .cloned()
        .ok_or_else(|| error("Board is missing"))?;
    entity.data = EntityData::Feature {
        definition: definition(BOARD_TYPE, &board)?,
        inputs: vec![],
        suppressed: false,
    };
    let mut edits = vec![Mutation::Replace(entity)];
    let mut outputs = vec![CreatedBody {
        feature: board_feature,
        body: board.output.body,
    }];
    let existing: Vec<_> = components(document)
        .filter_map(|e| read_component(document, e.id).ok().map(|c| (e.id, c)))
        .filter(|(_, c)| c.board == board_feature)
        .collect();
    for (id, c) in &existing {
        let mut next = c.clone();
        match import
            .board
            .footprints
            .iter()
            .find(|f| f.reference == c.footprint.reference)
        {
            Some(fp) => {
                if fp.at != c.footprint.at
                    || fp.rotation_deg != c.footprint.rotation_deg
                    || fp.back != c.footprint.back
                    || fp.envelope != c.footprint.envelope
                {
                    plan.moved.push(fp.reference.clone());
                } else {
                    plan.unchanged.push(fp.reference.clone());
                }
                next.footprint = fp.clone();
                next.missing = false;
            }
            None => {
                plan.missing.push(c.footprint.reference.clone());
                next.missing = true;
            }
        }
        if next != *c {
            let mut e = document.get(*id).cloned().expect("component");
            e.data = EntityData::Feature {
                definition: definition(COMPONENT_TYPE, &next)?,
                inputs: next.inputs(),
                suppressed: false,
            };
            edits.push(Mutation::Replace(e));
        }
        outputs.push(CreatedBody {
            feature: *id,
            body: c.output.body,
        });
    }
    let stem = import
        .file_name
        .rsplit_once('.')
        .map_or(import.file_name.as_str(), |(s, _)| s)
        .to_owned();
    for fp in &import.board.footprints {
        let wanted = import
            .components
            .as_ref()
            .is_none_or(|r| r.contains(&fp.reference));
        if !wanted
            || existing
                .iter()
                .any(|(_, c)| c.footprint.reference == fp.reference)
        {
            continue;
        }
        plan.added.push(fp.reference.clone());
        let output = new_output();
        let component = ComponentDefinition {
            output: BodyOutput { body: output.body },
            board: board_feature,
            footprint: fp.clone(),
            height_mm: import.default_height_mm,
            model: None,
            missing: false,
        };
        component.validate()?;
        let inputs = component.inputs();
        insert_part(
            &mut edits,
            &format!("{stem} {}", fp.reference),
            COMPONENT_TYPE,
            &component,
            inputs,
            &output,
        )?;
        outputs.push(output);
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: format!("Reimport {}", import.file_name),
        edits,
    })?;
    Ok((plan, ModelPrepared { prepared, outputs }))
}
/// Link (or replace) a component's 3D model with a STEP import.
pub fn prepare_component_model(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    component: EntityId,
    model: Option<EntityId>,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let mut c = read_component(document, component)?;
    if let Some(m) = model {
        read_step_source(document, m)?;
    }
    c.model = model;
    let mut e = document
        .get(component)
        .cloned()
        .ok_or_else(|| error("Component is missing"))?;
    e.data = EntityData::Feature {
        definition: definition(COMPONENT_TYPE, &c)?,
        inputs: c.inputs(),
        suppressed: false,
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Link component model".into(),
        edits: vec![Mutation::Replace(e)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature: component,
            body: c.output.body,
        }],
    })
}
