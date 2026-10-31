//! Cross-compile feature result cache (CAD-150).
//!
//! A full compile still walks the complete canonical document in dependency
//! order, but a Feature whose result key is unchanged since an earlier
//! compile reuses that compile's exact solid (shared, immutable B-rep handle)
//! and its display/topology snapshot instead of re-running the native
//! provider, the topology export and the tessellation.
//!
//! # Result key
//!
//! The key of a feature is a SHA-256 over:
//! - a cache format tag, the geometry policy and the display mesh options;
//! - the feature's own identity;
//! - the set of every entity in the feature's *input closure*, each as its
//!   identity plus a hash of its complete canonical source (name, links and
//!   definition) and, for Parameters, the evaluated value.
//!
//! The input closure starts at the feature and follows (a) its evaluation
//! dependencies (`Document::evaluation_dependencies`, the same graph that
//! orders the compile) and (b) every entity identity mentioned anywhere in
//! its source, including expression bindings and identities embedded in
//! strings. Both kinds of edge are followed transitively. A Part reached only
//! through a mention (for example the feature's own output Part) contributes
//! its own source plus the identity, type and suppression of its listed
//! features, not their closures: geometry can only flow along evaluation
//! dependencies, because nothing else orders a producer before its consumer.
//!
//! The evaluation revision is not part of the key. It only stamps topology
//! snapshots, so a reused snapshot is re-stamped with the current revision.
//!
//! # Purity
//!
//! Only provider types whose result is a function of that closure are
//! cached ([`cacheable_type`]). Weldment members find their connections by
//! scanning the document; those connections and the connected partner
//! members are declared as hidden inputs and join the closure. Providers that
//! otherwise look entities up by scanning the document (walls/openings,
//! levels), resolve
//! assembly poses (transfer, placement-based scale) or are unknown to this
//! module are always evaluated, and so is every feature whose closure
//! contains such a feature, an Instance, an Assembly, or a sketch targeted by
//! an occurrence reference. A new provider is therefore uncached until it is
//! audited and added here.
//!
//! # Lifetime, memory and threads
//!
//! Entries are only stored for successfully evaluated features, so a
//! superseded or failed compile can leave valid entries behind but never a
//! partial or stale one: a key describes all inputs of its result. The cache
//! holds kernel solids and is therefore `!Send`/`!Sync`; it lives on the
//! thread that compiles (the model runtime worker). After every compile the
//! entries used by that compile are kept (their geometry is shared with the
//! returned model, bounded by [`crate::MAX_MODEL_FEATURES`]); older entries
//! are evicted least-recently-used first until their estimated size is at
//! most `retained_bytes` and the entry count at most `max_entries`.
use crate::{Compiler, ShapeSnapshot, SheetLayout};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model,
    parameters::EvaluatedParameters,
    project_file::ContentHash,
    sketch_external,
};
use multicad_kernel::{PlaneFrame, Solid};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Default bound on the estimated size of entries not used by the latest
/// compile (kept for undo/redo and toggled edits).
pub const DEFAULT_RETAINED_BYTES: usize = 256 * 1024 * 1024;
/// Default bound on the total number of entries.
pub const DEFAULT_MAX_ENTRIES: usize = 4 * crate::MAX_MODEL_FEATURES;
const FORMAT: &[u8] = b"multicad.evaluation-cache/1";

/// Feature provider types whose evaluation reads only the input closure
/// (plus the [`hidden_inputs`] declared for its type).
pub fn cacheable_type(type_id: &str) -> bool {
    [
        model::EXTRUDE_TYPE,
        model::PRIMITIVE_TYPE,
        model::REVOLVE_TYPE,
        model::BOOLEAN_TYPE,
        model::SPLIT_TYPE,
        model::TRANSFORM_TYPE,
        model::EDGE_TREATMENT_TYPE,
        model::ADVANCED_TYPE,
        model::ALIGN_TYPE,
        model::PANEL_TYPE,
        model::PANEL_JOINT_CUT_TYPE,
        model::MACHINING_CUT_TYPE,
        model::SHEET_TYPE,
        model::MEMBER_TYPE,
        model::HARDWARE_TYPE,
        model::AID_TYPE,
        model::imports::STEP_BODY_TYPE,
        model::imports::BOARD_TYPE,
        model::imports::COMPONENT_TYPE,
    ]
    .contains(&type_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CacheKey([u8; 32]);

/// Side results a provider records next to its solid.
#[derive(Clone, Debug, Default)]
pub(crate) struct Extras {
    pub panel_frame: Option<PlaneFrame>,
    pub sheet_layout: Option<SheetLayout>,
    pub member_path: Option<model::PathSegment>,
    pub consumed: Vec<(EntityId, EntityId)>,
}

struct Entry {
    solid: Solid,
    shape: Arc<ShapeSnapshot>,
    extras: Extras,
    bytes: usize,
    used: u64,
}

/// What one compile did with the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileStats {
    /// Features whose native provider ran (cache misses and uncacheable).
    pub evaluated: BTreeSet<EntityId>,
    /// Features reused from an earlier compile.
    pub reused: BTreeSet<EntityId>,
    /// Evaluated features that are not cacheable (see the module docs).
    pub uncacheable: BTreeSet<EntityId>,
    /// Entries evicted after this compile.
    pub evicted: usize,
    /// Entries and their estimated size after eviction.
    pub entries: usize,
    pub estimated_bytes: usize,
}

/// Exact feature results reused across compiles of one model worker.
pub struct EvaluationCache {
    entries: BTreeMap<CacheKey, Entry>,
    clock: u64,
    retained_bytes: usize,
    max_entries: usize,
    last: CompileStats,
}
impl Default for EvaluationCache {
    fn default() -> Self {
        Self::new(DEFAULT_RETAINED_BYTES, DEFAULT_MAX_ENTRIES)
    }
}
impl EvaluationCache {
    pub fn new(retained_bytes: usize, max_entries: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            clock: 0,
            retained_bytes,
            max_entries,
            last: CompileStats::default(),
        }
    }
    /// Statistics of the most recent cached compile (also when it failed or
    /// was superseded).
    pub fn last_compile(&self) -> &CompileStats {
        &self.last
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn estimated_bytes(&self) -> usize {
        self.entries.values().map(|e| e.bytes).sum()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    fn evict(&mut self) -> usize {
        let current = self.clock;
        let mut stale: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| e.used < current)
            .map(|(k, e)| (e.used, *k, e.bytes))
            .collect();
        stale.sort();
        let mut stale_bytes: usize = stale.iter().map(|s| s.2).sum();
        let mut evicted = 0;
        for (_, key, bytes) in stale {
            if stale_bytes <= self.retained_bytes && self.entries.len() <= self.max_entries {
                break;
            }
            self.entries.remove(&key);
            stale_bytes -= bytes;
            evicted += 1;
        }
        evicted
    }
}

fn estimate(shape: &ShapeSnapshot) -> usize {
    let mesh = shape.mesh.vertices.len() * 8
        + shape.mesh.indices.len() * 4
        + shape.mesh.face_indices.len() * 4;
    let edges: usize = shape
        .edges
        .iter()
        .map(|e| 32 + e.points_mm.len() * 24)
        .sum();
    // Topology records and the native B-rep are estimated per element.
    let topology = shape.topology.elements.len() * (256 + 1024);
    std::mem::size_of::<ShapeSnapshot>() + mesh + edges + shape.vertices.len() * 32 + topology
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Visit {
    Full,
    /// A Part reached only through a mention.
    PartSource,
}

/// Evaluation dependencies and other mentioned entities of one entity.
type Edges = (Vec<EntityId>, Vec<EntityId>);

/// One compile's view of the cache.
pub(crate) struct Session<'a> {
    cache: &'a mut EvaluationCache,
    document: &'a Document,
    parameters: &'a EvaluatedParameters,
    salt: Vec<u8>,
    hashes: BTreeMap<(EntityId, Visit), Option<[u8; 32]>>,
    edges: BTreeMap<EntityId, Option<Edges>>,
    volatile_sketches: BTreeSet<EntityId>,
    stats: CompileStats,
}
impl<'a> Session<'a> {
    pub(crate) fn new(
        cache: &'a mut EvaluationCache,
        compiler: &Compiler,
        document: &'a Document,
        parameters: &'a EvaluatedParameters,
    ) -> Self {
        cache.clock += 1;
        let mut salt = FORMAT.to_vec();
        salt.extend(
            format!(
                "|{:?}|{:?}|{}|",
                compiler.policy, compiler.mesh, compiler.omit_display
            )
            .bytes(),
        );
        let volatile_sketches = document
            .entities()
            .filter(|e| sketch_external::is_occurrence_reference(document, e.id))
            .filter_map(|e| sketch_external::read_occurrence(document, e.id).ok())
            .map(|request| request.query.sketch)
            .collect();
        Self {
            cache,
            document,
            parameters,
            salt,
            hashes: BTreeMap::new(),
            edges: BTreeMap::new(),
            volatile_sketches,
            stats: CompileStats::default(),
        }
    }
    /// Evaluation-dependency and mention edges of an entity.
    fn edges(&mut self, id: EntityId) -> Option<Edges> {
        if let Some(found) = self.edges.get(&id) {
            return found.clone();
        }
        let computed = (|| {
            let entity = self.document.get(id)?;
            let evaluation: BTreeSet<_> = self.document.evaluation_dependencies(id).ok()?;
            let mut mentioned = BTreeSet::new();
            mentions(&serde_json::to_value(entity).ok()?, &mut mentioned);
            mentioned.remove(&id);
            let mentioned = mentioned
                .into_iter()
                .filter(|m| !evaluation.contains(m) && self.document.get(*m).is_some())
                .collect();
            Some((evaluation.into_iter().collect(), mentioned))
        })();
        self.edges.insert(id, computed.clone());
        computed
    }
    fn hash(&mut self, id: EntityId, visit: Visit) -> Option<[u8; 32]> {
        if let Some(found) = self.hashes.get(&(id, visit)) {
            return *found;
        }
        let computed = (|| {
            let entity = self.document.get(id)?;
            let mut bytes = serde_json::to_vec(entity).ok()?;
            match (&entity.data, visit) {
                (EntityData::Parameter { .. }, _) => {
                    bytes.extend(b"|value|");
                    bytes.extend(serde_json::to_vec(self.parameters.values.get(&id)?).ok()?);
                }
                (EntityData::Part { features, .. }, Visit::PartSource) => {
                    for feature in features {
                        let Some(EntityData::Feature {
                            definition,
                            suppressed,
                            ..
                        }) = self.document.get(*feature).map(|e| &e.data)
                        else {
                            return None;
                        };
                        bytes.extend(
                            format!(
                                "|{feature}|{}|{}|{suppressed}",
                                definition.type_id, definition.schema_version
                            )
                            .bytes(),
                        );
                    }
                }
                _ => {}
            }
            Some(ContentHash::of(&bytes).0)
        })();
        self.hashes.insert((id, visit), computed);
        computed
    }
    /// The result key of a feature, or None when it must be evaluated.
    pub(crate) fn key(&mut self, feature: EntityId) -> Option<CacheKey> {
        let mut visited: BTreeMap<(EntityId, Visit), [u8; 32]> = BTreeMap::new();
        let mut pending = vec![(feature, Visit::Full)];
        while let Some((id, visit)) = pending.pop() {
            if visited.contains_key(&(id, visit)) {
                continue;
            }
            let entity = self.document.get(id)?;
            visited.insert((id, visit), self.hash(id, visit)?);
            if visit == Visit::PartSource {
                continue;
            }
            match &entity.data {
                EntityData::Instance { .. } | EntityData::Assembly { .. } => return None,
                EntityData::Feature { definition, .. } if !cacheable_type(&definition.type_id) => {
                    return None;
                }
                EntityData::Sketch { .. } if self.volatile_sketches.contains(&id) => return None,
                _ => {}
            }
            if let EntityData::Feature { definition, .. } = &entity.data {
                pending.extend(
                    hidden_inputs(self.document, id, &definition.type_id)?
                        .into_iter()
                        .map(|d| (d, Visit::Full)),
                );
            }
            let (evaluation, mentioned) = self.edges(id)?;
            pending.extend(evaluation.into_iter().map(|d| (d, Visit::Full)));
            for m in mentioned {
                let part = matches!(
                    self.document.get(m).map(|e| &e.data),
                    Some(EntityData::Part { .. })
                );
                pending.push((m, if part { Visit::PartSource } else { Visit::Full }));
            }
        }
        // The closure is a set; two features can share one (for example
        // connected weldment members), so the key also names its root.
        let mut bytes = self.salt.clone();
        bytes.extend(format!("root|{feature}|").bytes());
        for ((id, visit), hash) in &visited {
            bytes.extend(id.to_string().bytes());
            bytes.push(*visit as u8);
            bytes.extend(hash);
        }
        Some(CacheKey(ContentHash::of(&bytes).0))
    }
    /// Reuse an earlier exact result, re-stamped for this revision.
    pub(crate) fn lookup(
        &mut self,
        key: &CacheKey,
        revision: u64,
    ) -> Option<(Solid, Arc<ShapeSnapshot>, Extras)> {
        let clock = self.cache.clock;
        let entry = self.cache.entries.get_mut(key)?;
        let solid = entry.solid.share().ok()?;
        if entry.shape.topology.revision != revision {
            let mut shape = ShapeSnapshot::clone(&entry.shape);
            shape.topology.revision = revision;
            entry.shape = Arc::new(shape);
        }
        entry.used = clock;
        self.stats.reused.insert(entry.shape.producer);
        Some((solid, entry.shape.clone(), entry.extras.clone()))
    }
    pub(crate) fn evaluated(&mut self, feature: EntityId, cacheable: bool) {
        self.stats.evaluated.insert(feature);
        if !cacheable {
            self.stats.uncacheable.insert(feature);
        }
    }
    pub(crate) fn store(
        &mut self,
        key: CacheKey,
        solid: &Solid,
        shape: &Arc<ShapeSnapshot>,
        extras: Extras,
    ) {
        if let Ok(solid) = solid.share() {
            self.cache.entries.insert(
                key,
                Entry {
                    solid,
                    shape: shape.clone(),
                    extras,
                    bytes: estimate(shape),
                    used: self.cache.clock,
                },
            );
        }
    }
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        let mut stats = std::mem::take(&mut self.stats);
        stats.evicted = self.cache.evict();
        stats.entries = self.cache.entries.len();
        stats.estimated_bytes = self.cache.estimated_bytes();
        self.cache.last = stats;
    }
}

/// Entities a provider finds by scanning the document rather than through
/// its own source. None when they cannot be determined.
fn hidden_inputs(document: &Document, feature: EntityId, type_id: &str) -> Option<Vec<EntityId>> {
    if type_id != model::MEMBER_TYPE {
        return Some(vec![]);
    }
    // A member's end cuts come from every connection naming its Part and
    // from each connected partner member's resolved path and profile.
    let part = model::read_member(document, feature).ok()?.output.body;
    let mut inputs = vec![];
    for (connection, definition) in model::member_connections(document, part).ok()? {
        inputs.push(connection);
        for member in definition.members {
            if member != part {
                inputs.push(model::member_feature(document, member)?);
            }
        }
    }
    Some(inputs)
}

/// Every existing-format entity identity mentioned in a source value:
/// identity strings, identities embedded in longer strings and map keys.
fn mentions(value: &serde_json::Value, found: &mut BTreeSet<EntityId>) {
    match value {
        serde_json::Value::String(text) => scan(text, found),
        serde_json::Value::Array(items) => items.iter().for_each(|v| mentions(v, found)),
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                scan(key, found);
                mentions(value, found);
            }
        }
        _ => {}
    }
}
fn scan(text: &str, found: &mut BTreeSet<EntityId>) {
    if let Ok(id) = EntityId::parse(text) {
        found.insert(id);
        return;
    }
    let bytes = text.as_bytes();
    if bytes.len() < 36 {
        return;
    }
    for start in 0..=bytes.len() - 36 {
        let window = &bytes[start..start + 36];
        let shaped = window.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        });
        if shaped
            && let Ok(text) = std::str::from_utf8(window)
            && let Ok(id) = EntityId::parse(text)
        {
            found.insert(id);
        }
    }
}

/// Side-result maps a provider may write next to its solid.
pub(crate) struct Sides<'a> {
    pub panel_frames: &'a mut BTreeMap<EntityId, PlaneFrame>,
    pub sheet_layouts: &'a mut BTreeMap<EntityId, SheetLayout>,
    pub member_paths: &'a mut BTreeMap<EntityId, model::PathSegment>,
    pub consumed: &'a mut BTreeSet<(EntityId, EntityId)>,
}
/// Side results recorded by the provider of `feature` in this compile.
pub(crate) fn capture(
    feature: EntityId,
    sides: Sides,
    consumed_before: &BTreeSet<(EntityId, EntityId)>,
) -> Extras {
    let consumed_after = &*sides.consumed;
    Extras {
        panel_frame: sides.panel_frames.get(&feature).copied(),
        sheet_layout: sides.sheet_layouts.get(&feature).cloned(),
        member_path: sides.member_paths.get(&feature).copied(),
        consumed: consumed_after
            .difference(consumed_before)
            .copied()
            .collect(),
    }
}
impl Extras {
    pub(crate) fn apply(self, feature: EntityId, sides: Sides) {
        if let Some(frame) = self.panel_frame {
            sides.panel_frames.insert(feature, frame);
        }
        if let Some(layout) = self.sheet_layout {
            sides.sheet_layouts.insert(feature, layout);
        }
        if let Some(path) = self.member_path {
            sides.member_paths.insert(feature, path);
        }
        sides.consumed.extend(self.consumed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_find_identities_in_values_strings_and_keys() {
        let [a, b, c, d] = std::array::from_fn(|_| EntityId::new());
        let value = serde_json::json!({
            "target": a.to_string(),
            "role": format!("side/{b}/panel.v_start"),
            "by_id": { c.to_string(): [1, 2, format!("x{d}")] },
            "noise": "0123456789abcdef-not-an-identity",
        });
        let mut found = BTreeSet::new();
        mentions(&value, &mut found);
        assert_eq!(found, BTreeSet::from([a, b, c, d]));
    }

    #[test]
    fn cacheable_types_exclude_scanning_and_unknown_providers() {
        assert!(cacheable_type(model::EXTRUDE_TYPE));
        assert!(cacheable_type(model::MEMBER_TYPE));
        for excluded in [
            model::WALL_TYPE,
            model::SLAB_TYPE,
            model::STAIR_TYPE,
            model::TRANSFER_TYPE,
            model::SCALE_TYPE,
            "vendor.unknown",
        ] {
            assert!(!cacheable_type(excluded), "{excluded}");
        }
    }
}
