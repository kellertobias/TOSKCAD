//! Print packaging intent: which finished Parts go into a 3MF print package,
//! whether their removable FDM aids go with them, and what a native provider
//! reports back. Nothing here slices, controls a printer or asserts that a
//! package is printable; the native provider only packages exact geometry.
use super::*;
use crate::{materials::TagQuery, model, project_file::ContentHash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Upper bound on Parts in one print package, including aids.
pub const MAX_PRINT_PARTS: usize = 128;
/// The tag Print proposes when choosing Parts by tag.
pub const DEFAULT_PRINT_TAG: &str = "printed";

/// How Print chooses finished Parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrintSource {
    /// Explicit Parts, usually the selection captured on entering Export.
    Parts { parts: Vec<EntityId> },
    /// Every world Part carrying this exact, case-sensitive tag.
    Tag { tag: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintRequest {
    pub source: PrintSource,
    /// `IncludeAll` packages the removable aids serving the chosen Parts;
    /// `ExcludeRemovable` leaves every aid Part out.
    pub aids: AidPolicy,
    pub quality: MeshQuality,
    /// Also write one binary STL per packaged Part beside the 3MF.
    pub stl_per_part: bool,
}
impl Default for PrintRequest {
    fn default() -> Self {
        Self {
            source: PrintSource::Parts { parts: vec![] },
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality::default(),
            stl_per_part: false,
        }
    }
}
fn invalid(message: impl Into<String>) -> ExportError {
    ExportError::new(ErrorKind::Invalid, None, message)
}
impl PrintRequest {
    pub fn validate(&self) -> Result<(), ExportError> {
        match &self.source {
            PrintSource::Parts { parts } => {
                if parts.len() > MAX_PRINT_PARTS {
                    return Err(ExportError::new(
                        ErrorKind::Budget,
                        None,
                        format!("A print package holds at most {MAX_PRINT_PARTS} Parts"),
                    ));
                }
                if parts.iter().collect::<BTreeSet<_>>().len() != parts.len() {
                    return Err(invalid("Choose each printable Part once"));
                }
            }
            PrintSource::Tag { tag } => {
                crate::materials::checked_tags(&BTreeSet::from([tag.clone()]))
                    .map_err(|e| invalid(e.to_string()))?;
            }
        }
        let q = &self.quality;
        if !(0.001..=10.).contains(&q.chord_deviation_mm)
            || !(0.6..=57.).contains(&q.angular_deviation_deg)
        {
            return Err(invalid(
                "Mesh quality needs a chord deviation of 0.001–10 mm and an angular deviation of 0.6–57°",
            ));
        }
        Ok(())
    }
}
/// Why a Part is in the package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrintRole {
    Finished,
    /// A removable FDM aid, classified by its producing `fdm.aid` feature.
    Aid {
        feature: EntityId,
        target: EntityId,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintPart {
    pub part: EntityId,
    pub name: String,
    pub role: PrintRole,
}
/// A chosen or related entity that the package leaves out, with the reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintSkip {
    pub entity: EntityId,
    pub name: String,
    pub reason: String,
}
/// The resolved package source: finished Parts first, then included aids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintSelection {
    pub stamp: ExportStamp,
    pub parts: Vec<PrintPart>,
    pub skipped: Vec<PrintSkip>,
    /// FDM process records named by aids serving the chosen Parts.
    pub processes: Vec<EntityId>,
}
fn name_of(document: &Document, id: EntityId) -> String {
    document
        .get(id)
        .map_or_else(|| id.to_string(), |e| e.name.clone())
}
/// Resolve a print request against one immutable document snapshot.
/// Explicit identities that no longer exist fail instead of broadening the
/// package; placed occurrences and component definitions are listed as
/// skipped, never silently substituted.
pub fn resolve_print(
    document: &Document,
    revision: u64,
    request: &PrintRequest,
) -> Result<PrintSelection, ExportError> {
    request.validate()?;
    let stamp = ExportStamp::capture(document, revision)?;
    let candidates: Vec<EntityId> = match &request.source {
        PrintSource::Parts { parts } => parts.clone(),
        PrintSource::Tag { tag } => TagQuery {
            any: BTreeSet::from([tag.clone()]),
            ..Default::default()
        }
        .select(document)?,
    };
    let definitions = model::assembly_definition_parts(document)?;
    let aid_parts = model::aid_parts(document);
    let mut skipped = vec![];
    let mut finished = vec![];
    let mut explicit_aids = vec![];
    for id in candidates {
        let entity = document.get(id).ok_or_else(|| {
            ExportError::new(
                ErrorKind::Missing,
                Some(id),
                format!("Missing printable Part {id}; the package was not broadened"),
            )
        })?;
        let skip = |reason: &str| PrintSkip {
            entity: id,
            name: entity.name.clone(),
            reason: reason.into(),
        };
        match &entity.data {
            EntityData::Part { .. } => {
                if model::scale_component_owner(document, id)?.is_some() {
                    skipped.push(skip(
                        "Owned scale geometry belongs to its Instance; Print packages world Parts only",
                    ));
                } else if definitions.contains(&id) {
                    skipped.push(skip(
                        "Assembly component definition; Print packages world Parts, not placed occurrences",
                    ));
                } else if aid_parts.contains(&id) {
                    explicit_aids.push(id);
                } else {
                    finished.push(id);
                }
            }
            EntityData::Instance { .. } | EntityData::Assembly { .. } => skipped.push(skip(
                "Placed occurrences are not packaged by Print; choose the world Parts",
            )),
            _ => skipped.push(skip("Not a Part")),
        }
    }
    let finished_set: BTreeSet<_> = finished.iter().copied().collect();
    let mut parts: Vec<PrintPart> = finished
        .iter()
        .map(|id| PrintPart {
            part: *id,
            name: name_of(document, *id),
            role: PrintRole::Finished,
        })
        .collect();
    let mut processes = vec![];
    let mut included = BTreeSet::new();
    for entity in model::aids(document) {
        let aid = model::read_aid(document, entity.id)?;
        let part = aid.output.body;
        let chosen = explicit_aids.contains(&part);
        if !finished_set.contains(&aid.target) && !chosen {
            continue;
        }
        if let Some(process) = aid.process
            && !processes.contains(&process)
        {
            processes.push(process);
        }
        if request.aids == AidPolicy::ExcludeRemovable {
            skipped.push(PrintSkip {
                entity: part,
                name: name_of(document, part),
                reason: "Removable FDM aid left out by the aid policy".into(),
            });
        } else if included.insert(part) {
            parts.push(PrintPart {
                part,
                name: name_of(document, part),
                role: PrintRole::Aid {
                    feature: entity.id,
                    target: aid.target,
                },
            });
        }
    }
    if parts.len() > MAX_PRINT_PARTS {
        return Err(ExportError::new(
            ErrorKind::Budget,
            None,
            format!("A print package holds at most {MAX_PRINT_PARTS} Parts, including aids"),
        ));
    }
    Ok(PrintSelection {
        stamp,
        parts,
        skipped,
        processes,
    })
}
/// Binds a print preview to its source snapshot and the exact package plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintKey {
    pub source: ExportStamp,
    pub plan_hash: ContentHash,
}
/// One packaged Part as tessellated from its current exact solid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintObject {
    pub part: EntityId,
    pub name: String,
    pub role: PrintRole,
    /// World bounds of the exact solid, in millimeters.
    pub bounds_mm: [[f64; 3]; 2],
    pub volume_mm3: f64,
    pub triangles: usize,
    pub mesh_volume_mm3: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintFileKind {
    ThreeMf,
    Stl,
}
/// One file the package writes. Its name is the chosen file's stem plus
/// `suffix`, in the chosen file's directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintFile {
    pub kind: PrintFileKind,
    pub part: Option<EntityId>,
    pub suffix: String,
    pub objects: usize,
    pub triangles: usize,
}
impl PrintFile {
    pub fn filename(&self, stem: &str) -> String {
        format!("{stem}{}", self.suffix)
    }
}
/// An FDM process record as resolved for this snapshot, or why it failed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintProcess {
    pub id: EntityId,
    pub name: String,
    pub layer_height_mm: f64,
    pub line_width_mm: f64,
    pub min_layers: u32,
    pub min_lines: u32,
    pub support_gap_mm: f64,
    pub fit_clearance_mm: f64,
}
/// A review note. Findings never block writing and never establish that a
/// package prints; they repeat the FDM aid inspection and plain geometry facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintFinding {
    pub part: Option<EntityId>,
    pub subject: String,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintPreview {
    pub requested_by: Actor,
    pub key: PrintKey,
    pub provider: String,
    pub request: PrintRequest,
    pub selection: PrintSelection,
    pub objects: Vec<PrintObject>,
    pub files: Vec<PrintFile>,
    pub processes: Vec<PrintProcess>,
    pub findings: Vec<PrintFinding>,
    /// Why nothing can be written from this preview, if so.
    pub blocked: Option<String>,
}
impl PrintPreview {
    pub fn can_write(&self) -> bool {
        self.blocked.is_none() && !self.objects.is_empty()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintWritten {
    pub path: String,
    pub kind: PrintFileKind,
    pub bytes: u64,
    pub content_hash: ContentHash,
    pub objects: usize,
    pub triangles: usize,
    pub mesh_volume_mm3: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintReceipt {
    pub actor: Actor,
    pub key: PrintKey,
    pub files: Vec<PrintWritten>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PrintResult {
    Preview(Result<Box<PrintPreview>, ExportError>),
    Write(Result<PrintReceipt, ExportError>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintReply {
    pub id: EntityId,
    pub document: EntityId,
    pub revision: u64,
    pub actor: Actor,
    pub result: PrintResult,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        commands::{Actor, Channel, Session},
        materials::{TagEdit, prepare_tags},
        model::{
            AidDefinition, AidRequest, AidShape, BodyOutput, PrimitiveParameters, PrimitiveRequest,
            ProcessDefinition, angle, length,
        },
    };
    fn actor() -> Actor {
        Actor::local(Channel::Ui)
    }
    fn cube(s: &mut Session, name: &str) -> EntityId {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(10.)],
                frame: Default::default(),
            },
        );
        let p = model::prepare_primitive(s, s.revision(), actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        request.body
    }
    struct Fixture {
        s: Session,
        block: EntityId,
        other: EntityId,
        tab: EntityId,
        process: EntityId,
    }
    fn fixture() -> Fixture {
        let mut s = Session::new(Document::new("Print").unwrap()).unwrap();
        let block = cube(&mut s, "Block");
        let other = cube(&mut s, "Other");
        let (process, prepared) = model::prepare_process(
            &s,
            s.revision(),
            actor(),
            None,
            "PLA",
            &ProcessDefinition::default(),
        )
        .unwrap();
        s.commit(prepared).unwrap();
        let request = AidRequest::new(
            "Tab",
            AidDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                target: block,
                shape: AidShape::Tab {
                    origin: [length(10.), length(5.), length(0.)],
                    angle: angle(0.),
                    length: length(8.),
                    width: length(6.),
                    thickness: length(0.4),
                    overlap: length(2.),
                },
                process: Some(process),
            },
        );
        let p = model::prepare_aid(&s, s.revision(), actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        Fixture {
            s,
            block,
            other,
            tab: request.body,
            process,
        }
    }
    fn request(source: PrintSource, aids: AidPolicy) -> PrintRequest {
        PrintRequest {
            source,
            aids,
            ..Default::default()
        }
    }
    #[test]
    fn explicit_parts_bring_their_aids_unless_the_policy_excludes_them() {
        let f = fixture();
        let parts = PrintSource::Parts {
            parts: vec![f.block],
        };
        let with = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(parts.clone(), AidPolicy::IncludeAll),
        )
        .unwrap();
        assert_eq!(
            with.parts.iter().map(|p| p.part).collect::<Vec<_>>(),
            [f.block, f.tab]
        );
        assert!(matches!(with.parts[1].role, PrintRole::Aid { target, .. } if target == f.block));
        assert_eq!(with.processes, [f.process]);
        let without = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(parts, AidPolicy::ExcludeRemovable),
        )
        .unwrap();
        assert_eq!(
            without.parts.iter().map(|p| p.part).collect::<Vec<_>>(),
            [f.block]
        );
        assert_eq!(without.skipped[0].entity, f.tab);
        assert_eq!(without.processes, [f.process], "process still shown");
        // A Part without aids brings none.
        let other = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(
                PrintSource::Parts {
                    parts: vec![f.other],
                },
                AidPolicy::IncludeAll,
            ),
        )
        .unwrap();
        assert_eq!(other.parts.len(), 1);
        assert!(other.processes.is_empty());
    }
    #[test]
    fn tags_choose_parts_exactly_and_missing_identities_fail() {
        let mut f = fixture();
        let p = prepare_tags(
            &f.s,
            f.s.revision(),
            actor(),
            &[f.other],
            &TagEdit::Add(BTreeSet::from(["printed".to_string()])),
        )
        .unwrap();
        f.s.commit(p).unwrap();
        let tagged = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(
                PrintSource::Tag {
                    tag: "printed".into(),
                },
                AidPolicy::IncludeAll,
            ),
        )
        .unwrap();
        assert_eq!(
            tagged.parts.iter().map(|p| p.part).collect::<Vec<_>>(),
            [f.other]
        );
        let none = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(
                PrintSource::Tag {
                    tag: "Printed".into(),
                },
                AidPolicy::IncludeAll,
            ),
        )
        .unwrap();
        assert!(none.parts.is_empty(), "tags are case-sensitive");
        let missing = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(
                PrintSource::Parts {
                    parts: vec![EntityId::new()],
                },
                AidPolicy::IncludeAll,
            ),
        )
        .unwrap_err();
        assert_eq!(missing.kind, ErrorKind::Missing);
        let duplicate = request(
            PrintSource::Parts {
                parts: vec![f.block, f.block],
            },
            AidPolicy::IncludeAll,
        );
        assert!(duplicate.validate().is_err());
        let mut coarse = request(PrintSource::Parts { parts: vec![] }, AidPolicy::IncludeAll);
        coarse.quality.chord_deviation_mm = 20.;
        assert!(coarse.validate().is_err());
    }
}
