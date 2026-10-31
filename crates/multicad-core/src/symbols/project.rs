//! Pinned references to other native projects (CAD-185).
//!
//! A project reference places the physical content of another `.tcd` file
//! as an ordinary pinned symbol (ADR-0024): the referenced project's Parts
//! and their dependency closure are embedded as one immutable
//! `symbols.definition`, and each placement materializes its own private
//! copy. A `symbols.project_source` record next to the definition keeps
//! where that content came from: the referenced document identity, a
//! portable relative path and the hash of the referenced canonical source.
//!
//! The referencing project never reads the source to open or evaluate: the
//! pin is the content. Finding the source file again only tells whether it
//! is current, changed (an explicit update is available) or missing. See
//! `docs/architecture/project-references.md`.
use super::{
    materialize::{generated, record_entity},
    prepare::{finish, stale, update_edits},
    source::{author, decode, encode, entity, invalid},
    *,
};
use crate::{
    commands::{Actor, CommandError, Session},
    document::{Document, DocumentError, EntityData, EntityId, Link, Mutation, Placement},
    model::{self, AssemblyDefinition},
    project_file::ContentHash,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const PROJECT_SOURCE_TYPE: &str = "symbols.project_source";
pub const PROJECT_CAPABILITY: &str = "symbols.project_source.1";
/// Longest stored relative path, in UTF-8 bytes.
pub const MAX_PROJECT_PATH: usize = 1024;

/// Where an embedded definition came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSource {
    /// The embedded `symbols.definition` holding the pinned content.
    pub definition: EntityId,
    /// Canonical identity of the referenced project document.
    pub document: EntityId,
    /// Portable path of the referenced file relative to the folder of the
    /// referencing file, with `/` separators. Never absolute.
    pub path: String,
    /// SHA-256 of the referenced project's canonical source when it was
    /// pinned. Informational: the definition pin decides whether the
    /// referenced content changed.
    pub source: ContentHash,
}
impl ProjectSource {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.definition.is_nil() || self.document.is_nil() {
            return Err(invalid(
                "A project reference needs its definition and source document identities",
            ));
        }
        validate_path(&self.path)
    }
    fn links(&self) -> Vec<Link> {
        vec![Link::new(self.definition, "definition")]
    }
    /// The file name the stored path ends in.
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// A stored reference path is relative, `/`-separated and names a native
/// project file. Absolute paths, drive letters, backslashes and empty
/// segments are refused so that no developer path is ever written.
pub fn validate_path(path: &str) -> Result<(), DocumentError> {
    let bad = path.trim() != path
        || path.is_empty()
        || path.len() > MAX_PROJECT_PATH
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.contains(':')
        || path.split('/').any(|s| s.is_empty() || s == ".")
        || !crate::project_path::is_project_path(Path::new(path));
    if bad {
        return Err(invalid(format!(
            "Project reference path {path:?} must be a relative, '/'-separated path to a .tcd or .multicad file"
        )));
    }
    Ok(())
}

/// A deterministic identity for synthesized source entities, so that
/// authoring the same referenced project twice yields the same definition
/// and a compatible update keeps every placement's identity map.
pub fn derived_id(namespace: EntityId, label: &str) -> EntityId {
    let mut hash = Sha256::new();
    hash.update(b"multicad.symbols.project/");
    hash.update(namespace.to_string().as_bytes());
    hash.update(b"/");
    hash.update(label.as_bytes());
    let mut bytes: [u8; 16] = hash.finalize()[..16].try_into().expect("16 bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x80; // version 8: custom
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    EntityId::parse(&format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
    .expect("formatted UUID")
}

/// Pin the physical content of a whole project as one static definition.
///
/// The referenced project's physical occurrences are its Parts that are not
/// definition-only members of a reusable Assembly and not in `consumed`
/// (tool bodies an unsuppressed Boolean consumed; the native compiler knows
/// them), plus its top-level Instances. A synthesized root Assembly holds
/// one identity-placed Instance per such Part and every top-level Instance;
/// its identities derive from the source identities. Drawings, export sets,
/// file information and weld/sequence records stay in the source file.
pub fn project_definition(
    source: &Document,
    consumed: &BTreeSet<EntityId>,
) -> Result<SymbolDefinition, DocumentError> {
    let definition_parts = model::assembly_definition_parts(source)?;
    let members = model::assembly_member_instances(source)?;
    let parts = source
        .entities()
        .filter(|e| {
            matches!(e.data, EntityData::Part { .. })
                && !definition_parts.contains(&e.id)
                && !consumed.contains(&e.id)
        })
        .map(|e| e.id)
        .collect::<Vec<_>>();
    let instances = source
        .entities()
        .filter(|e| matches!(e.data, EntityData::Instance { .. }) && !members.contains(&e.id))
        .map(|e| e.id)
        .collect::<Vec<_>>();
    if parts.is_empty() && instances.is_empty() {
        return Err(invalid(format!(
            "{} has no physical Parts or Instances to reference",
            source.name()
        )));
    }
    let root = derived_id(source.id(), "root");
    let ownership = derived_id(source.id(), "root-ownership");
    let mut edits = vec![];
    let mut occurrences = vec![];
    for part in &parts {
        let occurrence = derived_id(*part, "occurrence");
        occurrences.push(occurrence);
        let name = source
            .get(*part)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        edits.push(Mutation::Insert(entity(
            occurrence,
            name,
            EntityData::Instance {
                component: *part,
                placement: Placement::default(),
                placement_source: None,
            },
        )));
    }
    occurrences.extend(instances);
    edits.push(Mutation::Insert(entity(
        root,
        source.name(),
        EntityData::Assembly {
            instances: occurrences,
        },
    )));
    let assembly = AssemblyDefinition {
        assembly: root,
        definition_parts: parts,
    };
    edits.push(Mutation::Insert(entity(
        ownership,
        "Referenced project content",
        EntityData::Extension {
            definition: assembly.definition()?,
            inputs: assembly.inputs()?,
        },
    )));
    let candidate = source.edited(&edits)?;
    let request = SymbolAuthorRequest {
        definition: EntityId::new(),
        symbol: derived_id(source.id(), "symbol"),
        revision: "pending".into(),
        name: source.name().to_string(),
        kind: SymbolKind::Static,
        root,
        exposed: vec![],
        plan: None,
    };
    let mut value = author(&candidate, &request)?;
    let template = serde_json::to_vec(&value.template).map_err(|e| invalid(e.to_string()))?;
    let hash = ContentHash::of(&template).to_string();
    value.revision = format!("source {}", &hash[..12]);
    value.validate()?;
    Ok(value)
}

pub fn read_project_source(doc: &Document, id: EntityId) -> Result<ProjectSource, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = doc.get(id).map(|e| &e.data) else {
        return Err(invalid("Select a project reference source record"));
    };
    let value: ProjectSource = decode(definition, PROJECT_SOURCE_TYPE)?;
    value.validate()?;
    if *inputs != value.links() {
        return Err(invalid(
            "Project reference inputs disagree with its source record",
        ));
    }
    Ok(value)
}
pub fn project_sources(doc: &Document) -> Result<Vec<(EntityId, ProjectSource)>, DocumentError> {
    doc.entities()
        .filter(|e| is_project_source(e))
        .map(|e| read_project_source(doc, e.id).map(|v| (e.id, v)))
        .collect()
}
fn is_project_source(e: &crate::document::Entity) -> bool {
    matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == PROJECT_SOURCE_TYPE)
}
/// The source record of an embedded definition, if it came from a project.
pub fn project_source_of(
    doc: &Document,
    definition: EntityId,
) -> Result<Option<(EntityId, ProjectSource)>, DocumentError> {
    Ok(project_sources(doc)?
        .into_iter()
        .find(|(_, v)| v.definition == definition))
}
pub fn required_capability(doc: &Document) -> bool {
    doc.entities().any(is_project_source)
}
pub(super) fn validate_sources(doc: &Document) -> Result<(), DocumentError> {
    let mut seen = BTreeSet::new();
    for (_, source) in project_sources(doc)? {
        if !seen.insert(source.definition) {
            return Err(invalid(
                "An embedded definition has more than one project reference source",
            ));
        }
        let definition = read_definition(doc, source.definition)?;
        if definition.template.id() != source.document {
            return Err(invalid(format!(
                "Project reference {} does not hold content of its recorded source project",
                definition.name
            )));
        }
    }
    Ok(())
}
fn source_entity(
    record: EntityId,
    value: &ProjectSource,
) -> Result<crate::document::Entity, DocumentError> {
    value.validate()?;
    Ok(entity(
        record,
        "Project reference source",
        EntityData::Extension {
            definition: encode(PROJECT_SOURCE_TYPE, value)?,
            inputs: value.links(),
        },
    ))
}

/// Place one reference to another project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectReferenceRequest {
    /// Content authored by [`project_definition`] from the referenced file.
    pub content: SymbolDefinition,
    /// Identity and canonical source hash of the referenced file.
    pub document: EntityId,
    pub source: ContentHash,
    /// Relative path recorded when the reference is new.
    pub path: String,
    /// Used only when the definition is not embedded yet.
    pub definition: EntityId,
    pub source_record: EntityId,
    /// The placement's identities, as for [`SymbolPlaceRequest`].
    pub record: EntityId,
    pub instance: EntityId,
    pub wrapper: EntityId,
    pub wrapper_metadata: EntityId,
    pub member: EntityId,
    pub ids: BTreeMap<EntityId, EntityId>,
    pub name: String,
    pub placement: Placement,
}
impl ProjectReferenceRequest {
    pub fn new(
        content: SymbolDefinition,
        source: &Document,
        path: impl Into<String>,
        name: impl Into<String>,
        placement: Placement,
    ) -> Result<Self, DocumentError> {
        let ids = super::remap::identities(&content)?
            .into_iter()
            .map(|id| (id, EntityId::new()))
            .collect();
        Ok(Self {
            document: source.id(),
            source: ContentHash::document(source).map_err(invalid)?,
            content,
            path: path.into(),
            definition: EntityId::new(),
            source_record: EntityId::new(),
            record: EntityId::new(),
            instance: EntityId::new(),
            wrapper: EntityId::new(),
            wrapper_metadata: EntityId::new(),
            member: EntityId::new(),
            ids,
            name: name.into(),
            placement,
        })
    }
}

/// Embed the referenced content if this project does not hold it yet,
/// record its source, and place one copy, as one transaction. A second
/// placement of the same content reuses the embedded definition. A
/// different revision of an already referenced project is refused: update
/// the existing reference explicitly first.
pub fn prepare_project_reference(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &ProjectReferenceRequest,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    let doc = session.document();
    super::validate(doc)?;
    validate_path(&request.path)?;
    request.content.validate()?;
    if request.content.template.id() != request.document {
        return Err(invalid("Referenced content was not authored from this source project").into());
    }
    model::RigidPose::from_placement(request.placement)?;
    let pin = request.content.pin()?;
    let mut edits = vec![];
    let mut definition = None;
    for (id, old) in definitions(doc)? {
        if old.symbol != request.content.symbol {
            continue;
        }
        if old.pin()? != pin {
            return Err(invalid(format!(
                "This project already references another version of {} ({}); update that reference before placing {}",
                old.name, old.revision, request.content.revision
            ))
            .into());
        }
        definition = Some(id);
    }
    let definition = match definition {
        Some(id) => {
            if project_source_of(doc, id)?.is_none() {
                edits.push(Mutation::Insert(source_entity(
                    request.source_record,
                    &ProjectSource {
                        definition: id,
                        document: request.document,
                        path: request.path.clone(),
                        source: request.source,
                    },
                )?));
            }
            id
        }
        None => {
            edits.push(Mutation::Insert(entity(
                request.definition,
                request.content.name.clone(),
                EntityData::Extension {
                    definition: request.content.definition()?,
                    inputs: vec![],
                },
            )));
            edits.push(Mutation::Insert(source_entity(
                request.source_record,
                &ProjectSource {
                    definition: request.definition,
                    document: request.document,
                    path: request.path.clone(),
                    source: request.source,
                },
            )?));
            request.definition
        }
    };
    let value = SymbolInstance {
        definition,
        pin,
        instance: request.instance,
        wrapper: request.wrapper,
        wrapper_metadata: request.wrapper_metadata,
        member: request.member,
        ids: request.ids.clone(),
        overrides: BTreeMap::new(),
    };
    edits.extend(
        generated(&request.content, &value)?
            .into_values()
            .map(Mutation::Insert),
    );
    edits.push(Mutation::Insert(entity(
        request.instance,
        request.name.clone(),
        EntityData::Instance {
            component: request.wrapper,
            placement: request.placement,
            placement_source: None,
        },
    )));
    edits.push(Mutation::Insert(record_entity(request.record, &value)?));
    finish(
        session,
        revision,
        actor,
        &format!("Place reference to {}", request.content.name),
        edits,
        definition,
        &[value],
    )
}

/// Explicitly bring a reference up to date with its source file, or point it
/// at the file's new location. Content that equals the pin only rewrites
/// the source record. Changed content goes through the ordinary symbol
/// update: every placement keeps its outer Instance, pose and the private
/// identities of unchanged source entities, all in one undoable step.
pub fn prepare_project_update(
    session: &Session,
    revision: u64,
    actor: Actor,
    source_record: EntityId,
    content: &SymbolDefinition,
    source: &Document,
    path: &str,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    let doc = session.document();
    let old = read_project_source(doc, source_record)?;
    validate_path(path)?;
    if source.id() != old.document || content.template.id() != old.document {
        return Err(invalid(format!(
            "{} is a different project ({}); a reference can only follow its own source project",
            path,
            source.name()
        ))
        .into());
    }
    let current = read_definition(doc, old.definition)?;
    let record = ProjectSource {
        definition: old.definition,
        document: old.document,
        path: path.into(),
        source: ContentHash::document(source).map_err(invalid)?,
    };
    if current.pin()? == content.pin()? {
        return finish(
            session,
            revision,
            actor,
            &format!("Relocate reference to {}", current.name),
            vec![Mutation::Replace(source_entity(source_record, &record)?)],
            old.definition,
            &[],
        );
    }
    let update = SymbolUpdateRequest::new(doc, old.definition, content.clone())?;
    let (mut edits, values) = update_edits(doc, &update)?;
    edits.push(Mutation::Replace(source_entity(source_record, &record)?));
    finish(
        session,
        revision,
        actor,
        &format!("Update reference to {}", current.name),
        edits,
        old.definition,
        &values,
    )
}

/// The state of a reference's source file. The referencing project always
/// shows and evaluates its pinned content, whatever the state.
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectSourceState {
    /// Found, and its referenced content equals the pin.
    Current,
    /// Found, same project, different content: an explicit update is
    /// available. Nothing changes until the user applies it.
    Updated { revision: String },
    /// A file was found at a candidate path, but it is another project.
    Different { document: EntityId, name: String },
    /// No candidate file exists.
    Missing,
    /// A candidate exists but could not be read or pinned.
    Unreadable { message: String },
}
/// The outcome of looking for one reference's source file.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectSourceCheck {
    pub record: EntityId,
    pub definition: EntityId,
    pub name: String,
    pub revision: String,
    pub path: String,
    pub instances: Vec<EntityId>,
    /// The file the state was decided from, if any.
    pub found: Option<PathBuf>,
    /// Every location looked at, in order.
    pub searched: Vec<PathBuf>,
    pub state: ProjectSourceState,
}
impl ProjectSourceCheck {
    /// One line for an Inspector, status bar or MCP summary.
    pub fn summary(&self) -> String {
        let place = |p: &Option<PathBuf>| {
            p.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| self.path.clone())
        };
        match &self.state {
            ProjectSourceState::Current => format!(
                "{} · revision {} · source current ({})",
                self.name,
                self.revision,
                place(&self.found)
            ),
            ProjectSourceState::Updated { revision } => format!(
                "{} · revision {} · source changed to revision {revision} ({}); the pinned revision stays in use until you update the reference",
                self.name,
                self.revision,
                place(&self.found)
            ),
            ProjectSourceState::Different { name, .. } => format!(
                "{} · revision {} · {} now holds another project ({name}); the pinned revision stays in use",
                self.name,
                self.revision,
                place(&self.found)
            ),
            ProjectSourceState::Missing => format!(
                "{} · revision {} · source {} not found; the pinned revision stays in use",
                self.name, self.revision, self.path
            ),
            ProjectSourceState::Unreadable { message } => format!(
                "{} · revision {} · source {} unreadable ({message}); the pinned revision stays in use",
                self.name,
                self.revision,
                place(&self.found)
            ),
        }
    }
}

/// Where to look for a reference's source, in order: the stored relative
/// path from the referencing file's folder; the same file name next to the
/// referencing file; then the file name in each bundled directory (for
/// example the installed examples). An unsaved project has no folder, so
/// only bundled directories are searched.
pub fn candidate_paths(
    stored: &str,
    referencing_dir: Option<&Path>,
    bundled: &[PathBuf],
) -> Vec<PathBuf> {
    let name = stored.rsplit('/').next().unwrap_or(stored);
    let mut out = vec![];
    if let Some(dir) = referencing_dir {
        out.push(stored.split('/').fold(dir.to_path_buf(), |p, s| p.join(s)));
        out.push(dir.join(name));
    }
    out.extend(bundled.iter().map(|d| d.join(name)));
    let mut seen = BTreeSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// A portable relative path from `from_dir` to `target`, with `/`
/// separators, or `None` when the two share no root (another drive).
pub fn relative_path(from_dir: &Path, target: &Path) -> Option<String> {
    use std::path::Component;
    let from = from_dir.components().collect::<Vec<_>>();
    let to = target.components().collect::<Vec<_>>();
    if from.first() != to.first()
        || !matches!(
            from.first(),
            Some(Component::RootDir | Component::Prefix(_))
        )
    {
        return None;
    }
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = vec!["..".to_string(); from.len() - common];
    for c in &to[common..] {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.to_string()),
            _ => return None,
        }
    }
    let joined = parts.join("/");
    validate_path(&joined).ok().map(|_| joined)
}

/// Decide a reference's state from the project found at a candidate path.
/// `content` is that project authored with [`project_definition`], or the
/// reason it could not be authored.
pub fn classify_source(
    doc: &Document,
    record: EntityId,
    found: &Document,
    content: Result<&SymbolDefinition, String>,
) -> Result<ProjectSourceState, DocumentError> {
    let source = read_project_source(doc, record)?;
    if found.id() != source.document {
        return Ok(ProjectSourceState::Different {
            document: found.id(),
            name: found.name().to_string(),
        });
    }
    let content = match content {
        Ok(c) => c,
        Err(message) => return Ok(ProjectSourceState::Unreadable { message }),
    };
    let pinned = read_definition(doc, source.definition)?;
    Ok(if content.pin()? == pinned.pin()? {
        ProjectSourceState::Current
    } else {
        ProjectSourceState::Updated {
            revision: content.revision.clone(),
        }
    })
}

/// A check with no file decided yet: name, revision, placements and the
/// locations to search.
pub fn pending_check(
    doc: &Document,
    record: EntityId,
    referencing_dir: Option<&Path>,
    bundled: &[PathBuf],
) -> Result<ProjectSourceCheck, DocumentError> {
    let source = read_project_source(doc, record)?;
    let definition = read_definition(doc, source.definition)?;
    Ok(ProjectSourceCheck {
        record,
        definition: source.definition,
        name: definition.name.clone(),
        revision: definition.revision.clone(),
        instances: instances(doc, source.definition)?
            .into_iter()
            .map(|(_, v)| v.instance)
            .collect(),
        searched: candidate_paths(&source.path, referencing_dir, bundled),
        path: source.path,
        found: None,
        state: ProjectSourceState::Missing,
    })
}
