use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, Operation},
    project_file::ContentHash,
    references::{self, Origin, ReferenceReport, TopologySnapshot},
};
use multicad_kernel::{
    Bounds, DisplayEdge, DisplayVertex, GeometryPolicy, Mesh, MeshOptions, NativeRegionQueries,
    PlaneFrame, Solid,
};
use multicad_solver::SketchEvaluationContext;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
}
#[derive(Clone, Debug)]
pub struct ShapeSnapshot {
    pub producer: EntityId,
    pub bounds: Bounds,
    pub volume_mm3: f64,
    pub solid_count: u32,
    pub mesh: Mesh,
    pub topology: TopologySnapshot,
    pub edges: Vec<DisplayEdge>,
    pub vertices: Vec<DisplayVertex>,
}
#[derive(Clone, Debug)]
pub struct ModelSnapshot {
    pub stamp: ModelStamp,
    /// Includes upstream producers needed by attached sketches and repair.
    pub shapes: BTreeMap<EntityId, Arc<ShapeSnapshot>>,
    /// Canonical Part identity to its final unsuppressed producing feature.
    pub bodies: BTreeMap<EntityId, EntityId>,
    pub references: ReferenceReport,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelError {
    pub entity: Option<EntityId>,
    pub message: String,
}
impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(id) = self.entity {
            write!(f, "{id}: ")?;
        }
        f.write_str(&self.message)
    }
}
impl std::error::Error for ModelError {}
fn error(entity: Option<EntityId>, message: impl fmt::Display) -> ModelError {
    ModelError {
        entity,
        message: message.to_string(),
    }
}
fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ModelError> {
    if interrupt.is_some_and(|f| f()) {
        Err(error(None, "Model evaluation superseded"))
    } else {
        Ok(())
    }
}

/// An evaluated model owns genuine B-reps, never reconstructed display meshes.
/// The kernel's !Send/!Sync ownership also applies to this container.
pub struct Model {
    solids: BTreeMap<EntityId, Solid>,
    snapshot: ModelSnapshot,
}
impl Model {
    pub fn snapshot(&self) -> &ModelSnapshot {
        &self.snapshot
    }
    pub fn solid(&self, producer: EntityId) -> Option<&Solid> {
        self.solids.get(&producer)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Compiler {
    pub policy: GeometryPolicy,
    pub mesh: MeshOptions,
}
pub const MAX_MODEL_FEATURES: usize = 4096;
pub const MAX_MODEL_TRIANGLES: usize = 4_000_000;
impl Compiler {
    pub fn compile(&self, document: &Document, revision: u64) -> Result<Model, ModelError> {
        self.compile_interruptible(document, revision, None)
    }
    pub fn compile_interruptible(
        &self,
        document: &Document,
        revision: u64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Model, ModelError> {
        checkpoint(interrupt)?;
        document.validate().map_err(|e| error(None, e))?;
        self.policy.validate().map_err(|e| error(None, e))?;
        let context =
            SketchEvaluationContext::new(document, revision).map_err(|e| error(e.source, e))?;
        let stamp = ModelStamp {
            document: document.id(),
            revision,
            source_hash: context.stamp().source_hash,
        };
        let mut bodies = BTreeMap::new();
        for entity in document.entities() {
            if let EntityData::Part { features, .. } = &entity.data
                && let Some(producer) = features.iter().rev().find(|id| {
                    matches!(
                        document.get(**id).map(|e| &e.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
            {
                bodies.insert(entity.id, *producer);
            }
        }
        // Only model roots and their dependencies are executable. Unconsumed
        // sketches and opaque extensions do not prevent a document from opening.
        let mut needed = BTreeSet::new();
        let mut pending: Vec<_> = bodies.values().copied().collect();
        while let Some(id) = pending.pop() {
            if needed.insert(id) {
                let entity = document
                    .get(id)
                    .ok_or_else(|| error(Some(id), "Missing model dependency"))?;
                pending.extend(entity.references().into_iter().map(|link| link.target));
            }
        }
        let queries = NativeRegionQueries::new(self.policy).map_err(|e| error(None, e))?;
        let mut solids = BTreeMap::new();
        let mut shapes = BTreeMap::new();
        let mut topologies = BTreeMap::new();
        let mut sketches = BTreeMap::new();
        let mut triangles = 0usize;
        for id in document.topological_order().map_err(|e| error(None, e))? {
            if !needed.contains(&id) {
                continue;
            }
            checkpoint(interrupt)?;
            let entity = document
                .get(id)
                .ok_or_else(|| error(Some(id), "Missing model source"))?;
            let EntityData::Feature {
                definition,
                suppressed,
                ..
            } = &entity.data
            else {
                continue;
            };
            if *suppressed {
                return Err(error(Some(id), "Required model producer is suppressed"));
            }
            if solids.len() >= MAX_MODEL_FEATURES {
                return Err(error(Some(id), "Model exceeds the native feature budget"));
            }
            // Provider dispatch is confined to the native model layer. The UI,
            // document and storage layers do not implement geometry algorithms.
            let solid = match (definition.type_id.as_str(), definition.schema_version) {
                (model::EXTRUDE_TYPE, model::EXTRUDE_SCHEMA) => {
                    let source =
                        model::read_extrude(document, id).map_err(|e| error(Some(id), e))?;
                    if source.operation != Operation::NewBody
                        || !source.targets.is_empty()
                        || source.keep_tools
                    {
                        return Err(error(
                            Some(id),
                            "This evaluator currently supports New body extrusion only",
                        ));
                    }
                    let extent = source
                        .parameters
                        .resolve(context.parameters())
                        .map_err(|e| error(Some(id), e))?;
                    let sketch = source.profile.sketch;
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        sketches.entry(sketch)
                    {
                        let report = references::inspect(document, revision, &topologies);
                        let evaluated = context
                            .evaluate(sketch, Some(&report), interrupt)
                            .map_err(|e| error(e.source, e))?;
                        let regions = evaluated
                            .regions_with_queries_and_interrupt(&queries, interrupt)
                            .map_err(|e| error(e.source, e))?;
                        entry.insert((evaluated.frame, regions));
                    }
                    let (frame, regions) = &sketches[&sketch];
                    let region = regions.resolve(&source.profile.region).ok_or_else(||error(Some(id),"Selected profile changed or is no longer closed; reselect its boundary and holes"))?;
                    let direction = frame.vector(extent.direction);
                    let origin = std::array::from_fn(|i| {
                        frame.origin[i] + frame.n[i] * extent.start_offset_mm
                            - direction[i] * extent.backward_mm
                    });
                    let profile = NativeRegionQueries::profile(
                        region,
                        PlaneFrame {
                            origin,
                            u: frame.u,
                            v: frame.v,
                        },
                    );
                    let origins: Vec<_> = region
                        .outer
                        .edges
                        .iter()
                        .chain(region.holes.iter().flat_map(|h| &h.edges))
                        .map(|edge| Origin {
                            source: edge.authored,
                            role: "profile.edge".into(),
                        })
                        .collect();
                    let sweep = direction.map(|n| n * (extent.forward_mm + extent.backward_mm));
                    Solid::extrude_curved_profile_vector_named(
                        &profile,
                        sweep,
                        id,
                        &origins,
                        self.policy,
                    )
                    .map_err(|e| error(Some(id), e))?
                }
                (kind, version) => {
                    return Err(error(
                        Some(id),
                        format!("No native model provider for {kind} schema {version}"),
                    ));
                }
            };
            checkpoint(interrupt)?;
            let topology = solid
                .topology_snapshot(id, revision)
                .map_err(|e| error(Some(id), e))?;
            let snapshot = ShapeSnapshot {
                producer: id,
                bounds: solid.bounds().map_err(|e| error(Some(id), e))?,
                volume_mm3: solid.volume_mm3().map_err(|e| error(Some(id), e))?,
                solid_count: solid.solid_count().map_err(|e| error(Some(id), e))?,
                mesh: solid.mesh(self.mesh).map_err(|e| error(Some(id), e))?,
                edges: solid
                    .display_edges(self.mesh.linear_deflection_mm)
                    .map_err(|e| error(Some(id), e))?,
                vertices: solid.display_vertices().map_err(|e| error(Some(id), e))?,
                topology: topology.clone(),
            };
            triangles = triangles.saturating_add(snapshot.mesh.indices.len() / 3);
            if triangles > MAX_MODEL_TRIANGLES {
                return Err(error(
                    Some(id),
                    "Model display exceeds the triangle budget; use coarser detail",
                ));
            }
            topologies.insert(id, topology);
            shapes.insert(id, Arc::new(snapshot));
            solids.insert(id, solid);
        }
        checkpoint(interrupt)?;
        let references = references::inspect(document, revision, &topologies);
        Ok(Model {
            solids,
            snapshot: ModelSnapshot {
                stamp,
                shapes,
                bodies,
                references,
            },
        })
    }
}
