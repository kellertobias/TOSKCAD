//! Staged repair of several sketch references as one transaction. Each step is
//! built against the cumulative uncommitted candidate, so a later replacement
//! may use geometry an earlier source edit creates. Nothing reaches canonical
//! source until a native adapter checks the complete candidate and commits it.
use super::{
    ExternalCatalog, ExternalQuery, OccurrenceCaptureRequest, bindings, is_occurrence_reference,
    occurrence_bindings, occurrence_capture_edits, read, read_occurrence, remove_edits,
    remove_occurrence_edits, source::invalid,
};
use crate::{
    constraint_sketch::{CurveKind, source::Source},
    document::{Document, DocumentError, EntityData, EntityId, Mutation},
    project_file::ContentHash,
    references::{self, ReferenceAddress, TopologyKey, TopologySnapshot},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Upper bound on staged rows; each row may expand to several mutations.
pub const MAX_REPAIR_STEPS: usize = 256;

/// One explicit staged change. No step retargets a reference it does not name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepairStep {
    /// Bind a feature-source reference to an explicitly chosen element of a
    /// source feature's current topology. The topology kind must match.
    Rebind {
        sketch: EntityId,
        reference: EntityId,
        source: EntityId,
        selected: TopologyKey,
    },
    /// Replace an occurrence reference's query and semantic selector. Output
    /// IDs, family and pole count must stay compatible.
    RebindOccurrence {
        request: Box<OccurrenceCaptureRequest>,
    },
    /// Remove one reference and its output geometry. Dependents must already
    /// be removed by an earlier row.
    Remove {
        sketch: EntityId,
        reference: EntityId,
    },
    /// Explicitly remove own sketch constraints, curves and points, for
    /// example before an incompatible replacement.
    RemoveSketchItems {
        sketch: EntityId,
        #[serde(default)]
        constraints: Vec<EntityId>,
        #[serde(default)]
        curves: Vec<EntityId>,
        #[serde(default)]
        points: Vec<EntityId>,
    },
    /// Source edits that make a replacement possible, such as creating or
    /// editing replacement geometry. Reference records are off limits here.
    SourceEdit { label: String, edits: Vec<Mutation> },
}
impl RepairStep {
    pub fn sketch(&self) -> Option<EntityId> {
        match self {
            Self::Rebind { sketch, .. }
            | Self::Remove { sketch, .. }
            | Self::RemoveSketchItems { sketch, .. } => Some(*sketch),
            Self::RebindOccurrence { request } => Some(request.query.sketch),
            Self::SourceEdit { .. } => None,
        }
    }
    pub fn reference(&self) -> Option<EntityId> {
        match self {
            Self::Rebind { reference, .. } | Self::Remove { reference, .. } => Some(*reference),
            Self::RebindOccurrence { request } => Some(request.query.reference),
            _ => None,
        }
    }
}

/// The base a set was staged against. Any change discards the whole set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
    /// Fingerprint of project resources held outside the document, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<String>,
}
impl RepairStamp {
    pub fn current(
        document: &Document,
        revision: u64,
        resources: Option<String>,
    ) -> Result<Self, DocumentError> {
        Ok(Self {
            document: document.id(),
            revision,
            source_hash: ContentHash::document(document).map_err(invalid)?,
            resources,
        })
    }
    /// Reject a set staged against another document, revision, source or
    /// resource state.
    pub fn check(
        &self,
        document: &Document,
        revision: u64,
        resources: Option<&str>,
    ) -> Result<(), DocumentError> {
        if self.document != document.id() {
            return Err(invalid("Repair set belongs to another document"));
        }
        if self.revision != revision
            || self.source_hash != ContentHash::document(document).map_err(invalid)?
        {
            return Err(invalid(
                "Source changed after the repair set was staged; stage it again",
            ));
        }
        if self.resources.as_deref() != resources {
            return Err(invalid(
                "Project resources changed after the repair set was staged; stage it again",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairSet {
    pub stamp: RepairStamp,
    pub steps: Vec<RepairStep>,
}

/// Native data a step needs, evaluated on the cumulative candidate.
pub trait RepairNative {
    fn topology(
        &self,
        candidate: &Document,
        source: EntityId,
    ) -> Result<TopologySnapshot, DocumentError>;
    fn catalog(
        &self,
        candidate: &Document,
        query: &ExternalQuery,
    ) -> Result<ExternalCatalog, DocumentError>;
}

/// What a row changes, for review before Apply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepairRow {
    pub index: usize,
    pub sketch: Option<EntityId>,
    pub reference: Option<EntityId>,
    /// The binding or items this row replaces or removes.
    pub old: String,
    /// The staged replacement, removal or edit.
    pub new: String,
    /// Entities whose evaluation the row changes.
    pub affected: Vec<EntityId>,
    pub error: Option<String>,
}

/// A staged candidate: every row's outcome, the cumulative mutations and the
/// resulting document. It is only commit-capable when no row failed and a
/// native adapter has checked the complete model.
#[derive(Clone, Debug)]
pub struct StagedRepair {
    pub rows: Vec<RepairRow>,
    pub edits: Vec<Mutation>,
    pub candidate: Document,
    pub sketches: BTreeSet<EntityId>,
}
impl StagedRepair {
    pub fn complete(&self) -> bool {
        self.rows.iter().all(|r| r.error.is_none())
    }
}

fn name(document: &Document, id: EntityId) -> String {
    document
        .get(id)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| format!("missing {id}"))
}
fn old_binding(document: &Document, reference: EntityId) -> String {
    if is_occurrence_reference(document, reference) {
        return read_occurrence(document, reference)
            .map(|r| {
                format!(
                    "{} occurrence reference",
                    name(document, r.query.source.source)
                )
            })
            .unwrap_or_else(|e| e.to_string());
    }
    read(document, reference)
        .map(|b| {
            format!(
                "{} · {}",
                name(document, b.source),
                format!("{:?}", b.intent.kind).to_lowercase()
            )
        })
        .unwrap_or_else(|e| e.to_string())
}
fn reference_records(document: &Document) -> BTreeSet<EntityId> {
    document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
        .flat_map(|e| {
            let id = e.id;
            bindings(document, id)
                .map(|b| b.into_iter().map(|b| b.reference).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .chain(
                    occurrence_bindings(document, id)
                        .map(|b| b.into_iter().map(|b| b.query.reference).collect::<Vec<_>>())
                        .unwrap_or_default(),
                )
        })
        .collect()
}
fn owned(document: &Document, sketch: EntityId, reference: EntityId) -> Result<(), DocumentError> {
    let feature = bindings(document, sketch)?
        .iter()
        .any(|b| b.reference == reference);
    let occurrence = occurrence_bindings(document, sketch)?
        .iter()
        .any(|b| b.query.reference == reference);
    if feature || occurrence {
        Ok(())
    } else {
        Err(invalid("Reference is not owned by this sketch"))
    }
}
fn curve_points(kind: &CurveKind) -> Vec<EntityId> {
    match kind {
        CurveKind::Line { start, end } => vec![*start, *end],
        CurveKind::Circle { center, .. } => vec![*center],
        CurveKind::Arc {
            center, start, end, ..
        } => vec![*center, *start, *end],
        CurveKind::BSpline { poles, .. } => poles.clone(),
    }
}
/// External output IDs are removed only with their reference.
fn protected(document: &Document, sketch: EntityId) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut ids = BTreeSet::new();
    for b in bindings(document, sketch)? {
        ids.extend(b.definition.output.all());
    }
    for b in occurrence_bindings(document, sketch)? {
        ids.extend(b.output.all());
    }
    Ok(ids)
}
fn remove_items(
    document: &Document,
    sketch: EntityId,
    constraints: &[EntityId],
    curves: &[EntityId],
    points: &[EntityId],
) -> Result<Vec<Mutation>, DocumentError> {
    if constraints.is_empty() && curves.is_empty() && points.is_empty() {
        return Err(invalid("Choose sketch items to remove"));
    }
    let protected = protected(document, sketch)?;
    if curves.iter().chain(points).any(|id| protected.contains(id)) {
        return Err(invalid(
            "Reference output geometry is removed with its reference, not as own sketch items",
        ));
    }
    let mut entity = document
        .get(sketch)
        .ok_or(DocumentError::Missing(sketch))?
        .clone();
    let EntityData::Sketch { definition, .. } = &mut entity.data else {
        return Err(invalid("Choose a sketch"));
    };
    let mut source = Source::from_definition(definition)?;
    let sketch_data = &mut source.sketch;
    for (ids, present) in [
        (
            constraints,
            sketch_data
                .constraints
                .iter()
                .map(|c| c.id)
                .collect::<BTreeSet<_>>(),
        ),
        (curves, sketch_data.curves.iter().map(|c| c.id).collect()),
        (points, sketch_data.points.iter().map(|p| p.id).collect()),
    ] {
        if let Some(missing) = ids.iter().find(|id| !present.contains(id)) {
            return Err(invalid(format!("Sketch item {missing} is missing")));
        }
    }
    let constraints: BTreeSet<_> = constraints.iter().copied().collect();
    let curves: BTreeSet<_> = curves.iter().copied().collect();
    let points: BTreeSet<_> = points.iter().copied().collect();
    sketch_data
        .constraints
        .retain(|c| !constraints.contains(&c.id));
    sketch_data.curves.retain(|c| !curves.contains(&c.id));
    sketch_data.points.retain(|p| !points.contains(&p.id));
    let removed: BTreeSet<_> = curves.union(&points).copied().collect();
    if let Some(c) = sketch_data
        .constraints
        .iter()
        .find(|c| c.kind.references().iter().any(|id| removed.contains(id)))
    {
        return Err(invalid(format!(
            "Constraint {} uses removed geometry; remove it explicitly in the same row",
            c.id
        )));
    }
    if let Some(c) = sketch_data
        .curves
        .iter()
        .find(|c| curve_points(&c.kind).iter().any(|id| points.contains(id)))
    {
        return Err(invalid(format!(
            "Curve {} uses a removed point; remove it explicitly in the same row",
            c.id
        )));
    }
    source.validate()?;
    let version = definition.schema_version;
    *definition = source.definition()?;
    definition.schema_version = version.max(4);
    Ok(vec![Mutation::Replace(entity)])
}
fn source_edit(document: &Document, edits: &[Mutation]) -> Result<Vec<Mutation>, DocumentError> {
    if edits.is_empty() {
        return Err(invalid("A source edit row needs at least one change"));
    }
    let records = reference_records(document);
    for edit in edits {
        let id = match edit {
            Mutation::Insert(e) | Mutation::Replace(e) => e.id,
            Mutation::Remove(id) => *id,
            Mutation::SetHierarchy(_) => continue,
            Mutation::RenameDocument(_) => {
                return Err(invalid("A repair set cannot rename the document"));
            }
        };
        if records.contains(&id) {
            return Err(invalid(
                "Reference records change only through rebind or remove rows",
            ));
        }
    }
    let mut edits = edits.to_vec();
    crate::hierarchy::scope_insertions(document, &mut edits).map_err(invalid)?;
    let after = document.edited(&edits)?;
    if !reference_records(&after).is_subset(&records) {
        return Err(invalid(
            "A source edit cannot create sketch references; capture them separately",
        ));
    }
    Ok(edits)
}

fn step_edits(
    document: &Document,
    revision: u64,
    step: &RepairStep,
    native: &dyn RepairNative,
) -> Result<(String, String, Vec<Mutation>), DocumentError> {
    match step {
        RepairStep::Rebind {
            sketch,
            reference,
            source,
            selected,
        } => {
            owned(document, *sketch, *reference)?;
            if is_occurrence_reference(document, *reference) {
                return Err(invalid(
                    "Occurrence references are rebound with an occurrence query",
                ));
            }
            if !matches!(
                document.get(*source).map(|e| &e.data),
                Some(EntityData::Feature { .. })
            ) {
                return Err(invalid("Replacement requires an exact source feature"));
            }
            let binding = read(document, *reference)?;
            let snapshot = native.topology(document, *source)?;
            let edit = references::repair_edit(
                document,
                revision,
                ReferenceAddress {
                    owner: *reference,
                    reference: binding.intent.id,
                },
                &snapshot,
                *selected,
                None,
            )
            .map_err(|e| invalid(e.message))?;
            Ok((
                old_binding(document, *reference),
                format!("{} · {:?}", name(document, *source), selected.kind),
                vec![edit],
            ))
        }
        RepairStep::RebindOccurrence { request } => {
            owned(document, request.query.sketch, request.query.reference)?;
            let catalog = native.catalog(document, &request.query)?;
            let edits = occurrence_capture_edits(document, revision, request, &catalog, true)?;
            Ok((
                old_binding(document, request.query.reference),
                format!(
                    "{} occurrence reference",
                    name(document, request.query.source.source)
                ),
                edits,
            ))
        }
        RepairStep::Remove { sketch, reference } => {
            owned(document, *sketch, *reference)?;
            let edits = if is_occurrence_reference(document, *reference) {
                remove_occurrence_edits(document, *sketch, *reference)?
            } else {
                remove_edits(document, *sketch, *reference)?
            };
            Ok((
                old_binding(document, *reference),
                "Remove reference and its geometry".into(),
                edits,
            ))
        }
        RepairStep::RemoveSketchItems {
            sketch,
            constraints,
            curves,
            points,
        } => Ok((
            format!(
                "{} constraints, {} curves, {} points",
                constraints.len(),
                curves.len(),
                points.len()
            ),
            "Remove explicitly".into(),
            remove_items(document, *sketch, constraints, curves, points)?,
        )),
        RepairStep::SourceEdit { label, edits } => {
            if label.trim().is_empty() || label.len() > 128 {
                return Err(invalid("A source edit row needs a short label"));
            }
            Ok((
                format!("{} source changes", edits.len()),
                label.clone(),
                source_edit(document, edits)?,
            ))
        }
    }
}

/// Stage every row against the cumulative candidate. A failed row leaves the
/// candidate unchanged, records its diagnostic and blocks Apply; later rows
/// are still staged so every problem is visible at once.
pub fn stage_repair_set(
    document: &Document,
    revision: u64,
    resources: Option<&str>,
    set: &RepairSet,
    native: &dyn RepairNative,
) -> Result<StagedRepair, DocumentError> {
    set.stamp.check(document, revision, resources)?;
    if set.steps.is_empty() {
        return Err(invalid("Stage at least one repair"));
    }
    if set.steps.len() > MAX_REPAIR_STEPS {
        return Err(invalid("Too many staged repairs"));
    }
    let mut candidate = document.clone();
    let mut edits = vec![];
    let mut rows = vec![];
    let mut sketches = BTreeSet::new();
    for (index, step) in set.steps.iter().enumerate() {
        let staged = step_edits(&candidate, revision, step, native).and_then(|(old, new, m)| {
            let next = candidate.edited(&m)?;
            Ok((old, new, m, next))
        });
        let mut row = RepairRow {
            index,
            sketch: step.sketch(),
            reference: step.reference(),
            old: String::new(),
            new: String::new(),
            affected: vec![],
            error: None,
        };
        match staged {
            Ok((old, new, m, next)) => {
                let roots: Vec<_> = m
                    .iter()
                    .filter_map(|m| match m {
                        Mutation::Insert(e) | Mutation::Replace(e) => Some(e.id),
                        _ => None,
                    })
                    .collect();
                row.affected = next.affected(roots).unwrap_or_default();
                for id in &row.affected {
                    if matches!(
                        next.get(*id).map(|e| &e.data),
                        Some(EntityData::Sketch { .. })
                    ) {
                        sketches.insert(*id);
                    }
                }
                row.old = old;
                row.new = new;
                edits.extend(m);
                candidate = next;
            }
            Err(e) => {
                row.old = step
                    .reference()
                    .map(|r| old_binding(&candidate, r))
                    .unwrap_or_default();
                row.error = Some(e.to_string());
            }
        }
        rows.push(row);
    }
    sketches.extend(set.steps.iter().filter_map(RepairStep::sketch));
    sketches.retain(|id| candidate.get(*id).is_some());
    Ok(StagedRepair {
        rows,
        edits,
        candidate,
        sketches,
    })
}

/// Mutations that turn `before` into `after`, for a source edit row made with
/// an ordinary command on a scratch Session over the candidate.
pub fn document_edits(before: &Document, after: &Document) -> Vec<Mutation> {
    let mut edits = vec![];
    for entity in after.entities() {
        match before.get(entity.id) {
            None => edits.push(Mutation::Insert(entity.clone())),
            Some(old) if old != entity => edits.push(Mutation::Replace(entity.clone())),
            _ => {}
        }
    }
    for entity in before.entities() {
        if after.get(entity.id).is_none() {
            edits.push(Mutation::Remove(entity.id));
        }
    }
    if before.hierarchy() != after.hierarchy() {
        edits.push(Mutation::SetHierarchy(after.hierarchy().clone()));
    }
    edits
}
