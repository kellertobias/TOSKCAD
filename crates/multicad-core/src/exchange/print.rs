//! Print packaging intent: which finished Parts and placed occurrences go
//! into a 3MF print package, whether their removable FDM aids go with them,
//! and what a native provider reports back. Nothing here slices, controls a printer or asserts that a
//! package is printable; the native provider only packages exact geometry.
use super::*;
use crate::{materials::TagQuery, model, project_file::ContentHash};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Upper bound on packaged objects (world Parts and placed occurrences,
/// including aids) in one print package.
pub const MAX_PRINT_PARTS: usize = 128;
/// The tag Print proposes when choosing Parts by tag.
pub const DEFAULT_PRINT_TAG: &str = "printed";

/// How Print chooses finished Parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrintSource {
    /// Explicit Parts, Instances or Assemblies, usually the selection
    /// captured on entering Export. A placed occurrence is packaged at its
    /// world placement.
    Parts { parts: Vec<EntityId> },
    /// Every world Part and every placed occurrence of a Part carrying this
    /// exact, case-sensitive tag.
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
    /// Also write one binary STL per packaged object (world Part or placed
    /// occurrence) beside the 3MF.
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
    /// The placed occurrence, outermost Instance first; empty for a world
    /// Part. An aid riding on a placed target shares its target's path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrence_path: Vec<EntityId>,
    /// The Part name, followed by the occurrence names for a placed
    /// occurrence; repeated labels are numbered.
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
/// The resolved package source: finished objects first, then included aids.
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
/// One leaf Part occurrence chosen for the package: a world Part when the
/// path is empty.
#[derive(Clone)]
struct Chosen {
    part: EntityId,
    path: Vec<EntityId>,
    names: Vec<String>,
}
fn label(document: &Document, chosen: &Chosen) -> String {
    let name = name_of(document, chosen.part);
    if chosen.names.is_empty() {
        name
    } else {
        format!("{name} ({})", chosen.names.join(" / "))
    }
}
/// Resolve a print request against one immutable document snapshot.
/// Explicit identities that no longer exist fail instead of broadening the
/// package. A chosen Instance or Assembly is walked to its leaf Part
/// occurrences, each packaged at its own world placement; component
/// definitions are listed as skipped, never silently substituted.
pub fn resolve_print(
    document: &Document,
    revision: u64,
    request: &PrintRequest,
) -> Result<PrintSelection, ExportError> {
    request.validate()?;
    let stamp = ExportStamp::capture(document, revision)?;
    let by_tag = matches!(request.source, PrintSource::Tag { .. });
    let candidates: Vec<EntityId> = match &request.source {
        PrintSource::Parts { parts } => parts.clone(),
        PrintSource::Tag { tag } => TagQuery {
            any: BTreeSet::from([tag.clone()]),
            ..Default::default()
        }
        .select(document)?,
    };
    let definitions = model::assembly_definition_parts(document)?;
    let reusable = model::reusable_assemblies(document)?;
    let mut components = BTreeSet::new();
    let mut members = BTreeMap::new();
    for entity in document.entities() {
        match &entity.data {
            EntityData::Instance { component, .. } => {
                components.insert(*component);
            }
            EntityData::Assembly { instances } => {
                for member in instances {
                    members.insert(*member, entity.id);
                }
            }
            _ => {}
        }
    }
    // An Assembly that is reusable or itself placed has no world occurrences
    // of its own; its placed Instances do.
    let definition_assembly = |id: &EntityId| reusable.contains(id) || components.contains(id);
    let aid_parts = model::aid_parts(document);
    let mut skipped = vec![];
    let mut finished: Vec<Chosen> = vec![];
    let mut explicit_aids: Vec<Chosen> = vec![];
    let mut seen = BTreeSet::new();
    let mut choose = |chosen: Chosen, finished: &mut Vec<Chosen>, aids: &mut Vec<Chosen>| {
        if seen.insert((chosen.part, chosen.path.clone())) {
            if aid_parts.contains(&chosen.part) {
                aids.push(chosen);
            } else {
                finished.push(chosen);
            }
        }
    };
    let mut occurrences = vec![];
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
                        "Owned scale geometry belongs to its Instance; choose the Instance",
                    ));
                } else if definitions.contains(&id) {
                    skipped.push(skip(if by_tag {
                        "Assembly component definition; its placed occurrences carrying the tag are packaged instead"
                    } else {
                        "Assembly component definition; choose its placed Instances"
                    }));
                } else {
                    choose(
                        Chosen {
                            part: id,
                            path: vec![],
                            names: vec![],
                        },
                        &mut finished,
                        &mut explicit_aids,
                    );
                }
            }
            EntityData::Instance { .. }
                if members.get(&id).is_some_and(definition_assembly) =>
            {
                skipped.push(skip(
                    "Member of a reusable Assembly definition; choose a placed Instance of the Assembly",
                ))
            }
            EntityData::Assembly { .. } if definition_assembly(&id) => skipped.push(skip(
                "Reusable Assembly definition; choose a placed Instance of the Assembly",
            )),
            EntityData::Instance { .. } | EntityData::Assembly { .. } => {
                occurrences.extend(resolve_occurrences(document, id)?)
            }
            _ => skipped.push(skip("Not a Part, Instance or Assembly")),
        }
    }
    if let PrintSource::Tag { tag } = &request.source {
        // Placed occurrences whose leaf Part carries the tag, walked as a
        // project export scope walks them; world Parts were handled above.
        let scope = Scope {
            root: ScopeRoot::Project,
            tags: Some(TagQuery {
                any: BTreeSet::from([tag.clone()]),
                ..Default::default()
            }),
        };
        occurrences.extend(
            resolve_parts(document, &scope)?
                .into_iter()
                .filter(|p| !p.reference.occurrence_path.is_empty()),
        );
    }
    for occurrence in occurrences {
        choose(
            Chosen {
                part: occurrence.reference.part,
                path: occurrence.reference.occurrence_path,
                names: occurrence.occurrence_names,
            },
            &mut finished,
            &mut explicit_aids,
        );
    }
    let mut parts: Vec<PrintPart> = finished
        .iter()
        .map(|c| PrintPart {
            part: c.part,
            occurrence_path: c.path.clone(),
            name: label(document, c),
            role: PrintRole::Finished,
        })
        .collect();
    let mut processes = vec![];
    let mut included = BTreeSet::new();
    for entity in model::aids(document) {
        let aid = model::read_aid(document, entity.id)?;
        let part = aid.output.body;
        // The aid rides on every chosen occurrence of its target, and is
        // packaged wherever the aid Part itself was chosen.
        let placed: Vec<Chosen> = finished
            .iter()
            .filter(|c| c.part == aid.target)
            .map(|c| Chosen { part, ..c.clone() })
            .chain(explicit_aids.iter().filter(|c| c.part == part).cloned())
            .collect();
        if placed.is_empty() {
            continue;
        }
        if let Some(process) = aid.process
            && !processes.contains(&process)
        {
            processes.push(process);
        }
        for chosen in placed {
            if !included.insert((part, chosen.path.clone())) {
                continue;
            }
            if request.aids == AidPolicy::ExcludeRemovable {
                skipped.push(PrintSkip {
                    entity: part,
                    name: label(document, &chosen),
                    reason: "Removable FDM aid left out by the aid policy".into(),
                });
            } else {
                parts.push(PrintPart {
                    part,
                    name: label(document, &chosen),
                    occurrence_path: chosen.path,
                    role: PrintRole::Aid {
                        feature: entity.id,
                        target: aid.target,
                    },
                });
            }
        }
    }
    if parts.len() > MAX_PRINT_PARTS {
        return Err(ExportError::new(
            ErrorKind::Budget,
            None,
            format!(
                "A print package holds at most {MAX_PRINT_PARTS} objects, including placed occurrences and aids"
            ),
        ));
    }
    // Number repeated labels so every packaged object is told apart.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for part in &parts {
        *counts.entry(part.name.clone()).or_default() += 1;
    }
    let mut ordinal: BTreeMap<String, usize> = BTreeMap::new();
    for part in &mut parts {
        if counts[&part.name] > 1 {
            let n = ordinal.entry(part.name.clone()).or_default();
            *n += 1;
            part.name = format!("{} #{n}", part.name);
        }
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
/// One packaged world Part or placed occurrence as tessellated from its
/// current exact solid. Bounds and volumes are in world placement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintObject {
    pub part: EntityId,
    /// The placed occurrence; empty for a world Part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrence_path: Vec<EntityId>,
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
    /// For a per-object STL: the placed occurrence of `part`, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrence_path: Vec<EntityId>,
    pub suffix: String,
    /// Mesh resource objects; placed occurrences of one Part share one.
    pub objects: usize,
    /// Build items: one per packaged world Part or placed occurrence.
    pub items: usize,
    /// Placed triangles over all build items.
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
    fn placed_occurrences_are_chosen_per_path_with_their_aids() {
        let mut f = fixture();
        let mut placed = model::ComponentPlacementRequest::new(f.block, Default::default());
        placed.name = "Spare".into();
        let p = model::prepare_place_component(&f.s, f.s.revision(), actor(), &placed).unwrap();
        f.s.commit(p.prepared).unwrap();
        let chosen = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(
                PrintSource::Parts {
                    parts: vec![placed.instance, f.block],
                },
                AidPolicy::IncludeAll,
            ),
        )
        .unwrap();
        assert_eq!(
            chosen
                .parts
                .iter()
                .map(|p| (p.part, p.occurrence_path.clone(), p.name.as_str()))
                .collect::<Vec<_>>(),
            [
                (f.block, vec![], "Block"),
                (f.block, vec![placed.instance], "Block (Spare)"),
                (f.tab, vec![], "Tab"),
                (f.tab, vec![placed.instance], "Tab (Spare)"),
            ]
        );
        assert!(chosen.skipped.is_empty());
        assert_eq!(chosen.processes, [f.process], "listed once");
        let without = resolve_print(
            f.s.document(),
            f.s.revision(),
            &request(
                PrintSource::Parts {
                    parts: vec![placed.instance],
                },
                AidPolicy::ExcludeRemovable,
            ),
        )
        .unwrap();
        assert_eq!(without.parts.len(), 1);
        assert_eq!(without.skipped[0].name, "Tab (Spare)");
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
