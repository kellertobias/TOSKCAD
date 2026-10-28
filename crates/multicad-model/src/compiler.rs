use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, CutTargetProbeResult, Operation},
    project_file::ContentHash,
    references::{self, Origin, ReferenceReport, TopologySnapshot},
    sketch_regions::{Extraction, RegionIntent},
};
use multicad_kernel::{
    Bounds, CurvedEdge2d, CurvedProfile, DisplayEdge, DisplayVertex, GeometryPolicy, Mesh,
    MeshOptions, NativeRegionQueries, PlaneFrame, RevolutionAxis, Solid,
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
    /// Explicit occurrence poses/world topology; component meshes remain shared.
    pub occurrences: BTreeMap<EntityId, Arc<crate::OccurrenceSnapshot>>,
    /// Canonical tool Parts retained as source/history but consumed by an
    /// unsuppressed Boolean rather than emitted as independent final bodies.
    pub consumed_bodies: BTreeSet<EntityId>,
    /// Optional derived query receipts for this exact evaluation request.
    pub cut_targets: Vec<CutTargetProbeResult>,
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
pub(crate) fn error(entity: Option<EntityId>, message: impl fmt::Display) -> ModelError {
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

struct Profiles<'a> {
    context: &'a SketchEvaluationContext<'a>,
    queries: NativeRegionQueries,
    cache: BTreeMap<EntityId, (multicad_core::sketch::Frame, Extraction)>,
}
impl Profiles<'_> {
    fn get(
        &mut self,
        sketch: EntityId,
        intent: &RegionIntent,
        feature: EntityId,
        topologies: &BTreeMap<EntityId, TopologySnapshot>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(CurvedProfile, Vec<Origin>), ModelError> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.cache.entry(sketch) {
            let report = references::inspect(
                self.context.document(),
                self.context.stamp().revision,
                topologies,
            );
            let evaluated = self
                .context
                .evaluate(sketch, Some(&report), interrupt)
                .map_err(|e| error(e.source, e))?;
            let regions = evaluated
                .regions_with_queries_and_interrupt(&self.queries, interrupt)
                .map_err(|e| error(e.source, e))?;
            entry.insert((evaluated.frame, regions));
        }
        let (frame, regions) = &self.cache[&sketch];
        let region = regions.resolve(intent).ok_or_else(|| {
            error(
                Some(feature),
                "Selected profile changed or is no longer closed; reselect its boundary and holes",
            )
        })?;
        let origins = region
            .outer
            .edges
            .iter()
            .chain(region.holes.iter().flat_map(|h| &h.edges))
            .map(|edge| Origin {
                source: edge.authored,
                role: "profile.edge".into(),
            })
            .collect();
        Ok((
            NativeRegionQueries::profile(
                region,
                PlaneFrame {
                    origin: frame.origin,
                    u: frame.u,
                    v: frame.v,
                },
            ),
            origins,
        ))
    }
}

/// An evaluated model owns genuine B-reps, never reconstructed display meshes.
/// The kernel's !Send/!Sync ownership also applies to this container.
pub struct Model {
    pub(crate) policy: GeometryPolicy,
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
            if let EntityData::Part { features, .. } = &entity.data {
                for feature in features {
                    if !matches!(
                        document.get(*feature).map(|e| &e.data),
                        Some(EntityData::Feature { .. })
                    ) {
                        return Err(error(
                            Some(entity.id),
                            format!(
                                "Body history producer {feature} is missing or invalid; repair its source before rebuilding"
                            ),
                        ));
                    }
                }
                if let Some(producer) = features.iter().rev().find(|id| {
                    matches!(
                        document.get(**id).map(|e| &e.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                }) {
                    bodies.insert(entity.id, *producer);
                }
            }
        }
        // Only model roots and their dependencies are executable. Unconsumed
        // sketches and opaque extensions do not prevent a document from opening.
        let mut needed = BTreeSet::new();
        let mut pending: Vec<_> = bodies.values().copied().collect();
        pending.extend(
            document
                .entities()
                .filter(|e| matches!(e.data, EntityData::Instance { .. }))
                .map(|e| e.id),
        );
        while let Some(id) = pending.pop() {
            if needed.insert(id) {
                let entity = document
                    .get(id)
                    .ok_or_else(|| error(Some(id), "Missing model dependency"))?;
                pending.extend(entity.references().into_iter().map(|link| link.target));
            }
        }
        let mut profiles = Profiles {
            context: &context,
            queries: NativeRegionQueries::new(self.policy).map_err(|e| error(None, e))?,
            cache: BTreeMap::new(),
        };
        let mut solids: BTreeMap<EntityId, Solid> = BTreeMap::new();
        let mut consumed_versions = BTreeSet::new();
        let mut shapes = BTreeMap::new();
        let mut occurrences = BTreeMap::new();
        let mut topologies = BTreeMap::new();
        let mut triangles = 0usize;
        for id in document.topological_order().map_err(|e| error(None, e))? {
            if !needed.contains(&id) {
                continue;
            }
            checkpoint(interrupt)?;
            let entity = document
                .get(id)
                .ok_or_else(|| error(Some(id), "Missing model source"))?;
            if matches!(entity.data, EntityData::Instance { .. }) {
                if occurrences.len() >= crate::MAX_MODEL_OCCURRENCES {
                    return Err(error(
                        Some(id),
                        "Model exceeds the explicit occurrence budget",
                    ));
                }
                let (occurrence, solid) = crate::occurrences::build(
                    crate::occurrences::Inputs {
                        document,
                        parameters: context.parameters(),
                        body_heads: &bodies,
                        solids: &solids,
                        topologies: &topologies,
                        policy: self.policy,
                    },
                    id,
                    revision,
                )?;
                topologies.insert(id, occurrence.topology.clone());
                solids.insert(id, solid);
                occurrences.insert(id, Arc::new(occurrence));
                continue;
            }
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
            if shapes.len() >= MAX_MODEL_FEATURES {
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
                        .resolve_controls(context.parameters())
                        .map_err(|e| error(Some(id), e))?;
                    let (profile, origins) = profiles.get(
                        source.profile.sketch,
                        &source.profile.region,
                        id,
                        &topologies,
                        interrupt,
                    )?;
                    crate::extrude::Extrusion {
                        document,
                        revision,
                        feature: id,
                        solids: &solids,
                        topologies: &topologies,
                        policy: self.policy,
                        interrupt,
                    }
                    .build(profile, &origins, &extent)?
                }
                (model::PRIMITIVE_TYPE, model::PRIMITIVE_SCHEMA) => {
                    let source =
                        model::read_primitive(document, id).map_err(|e| error(Some(id), e))?;
                    let (frame, outer, height) = match source
                        .parameters
                        .resolve(context.parameters())
                        .map_err(|e| error(Some(id), e))?
                    {
                        model::ResolvedPrimitive::Box {
                            size_mm: [width, depth, height],
                            frame,
                        } => {
                            let points = [[0., 0.], [width, 0.], [width, depth], [0., depth]];
                            (
                                frame,
                                (0..4)
                                    .map(|i| CurvedEdge2d::Line {
                                        a: points[i],
                                        b: points[(i + 1) % 4],
                                    })
                                    .collect(),
                                height,
                            )
                        }
                        model::ResolvedPrimitive::Cylinder {
                            radius_mm,
                            height_mm,
                            frame,
                        } => (
                            frame,
                            vec![CurvedEdge2d::Circle {
                                center: [0., 0.],
                                radius_mm,
                            }],
                            height_mm,
                        ),
                    };
                    let origins: Vec<_> = (0..outer.len())
                        .map(|i| Origin {
                            source: id,
                            role: format!("primitive.edge.{i}"),
                        })
                        .collect();
                    let profile = CurvedProfile {
                        frame: PlaneFrame {
                            origin: frame.origin,
                            u: frame.u,
                            v: frame.v,
                        },
                        outer,
                        holes: vec![],
                    };
                    Solid::extrude_curved_profile_vector_named(
                        &profile,
                        frame.n.map(|n| n * height),
                        id,
                        &origins,
                        self.policy,
                    )
                    .map_err(|e| error(Some(id), e))?
                }
                (model::REVOLVE_TYPE, model::REVOLVE_SCHEMA) => {
                    let source =
                        model::read_revolve(document, id).map_err(|e| error(Some(id), e))?;
                    let resolved = source
                        .parameters
                        .resolve(context.parameters())
                        .map_err(|e| error(Some(id), e))?;
                    let (profile, origins) = profiles.get(
                        source.profile.sketch,
                        &source.profile.region,
                        id,
                        &topologies,
                        interrupt,
                    )?;
                    let axis = RevolutionAxis {
                        origin_mm: profile.frame.point(resolved.axis_origin_mm),
                        direction: std::array::from_fn(|i| {
                            profile.frame.u[i] * resolved.axis_direction[0]
                                + profile.frame.v[i] * resolved.axis_direction[1]
                        }),
                    };
                    Solid::revolve_curved_profile_named(
                        &profile,
                        axis,
                        resolved.angle_rad,
                        id,
                        &origins,
                        self.policy,
                    )
                    .map_err(|e| error(Some(id), e))?
                }
                (model::ALIGN_TYPE, model::ALIGN_SCHEMA) => {
                    let (source, references) =
                        model::read_alignment(document, id).map_err(|e| error(Some(id), e))?;
                    crate::alignment::build(
                        crate::alignment::Inputs {
                            document,
                            revision,
                            parameters: context.parameters(),
                            solids: &solids,
                            topologies: &topologies,
                        },
                        id,
                        &source,
                        &references,
                    )?
                }
                (model::TRANSFORM_TYPE, model::TRANSFORM_SCHEMA) => {
                    let source =
                        model::read_transform(document, id).map_err(|e| error(Some(id), e))?;
                    crate::transform::build(id, &source, context.parameters(), &solids)?
                }
                (model::SPLIT_TYPE, model::SPLIT_SCHEMA) => {
                    let source = model::read_split(document, id).map_err(|e| error(Some(id), e))?;
                    let solid = crate::split::build(
                        document,
                        id,
                        &source,
                        &solids,
                        self.policy,
                        interrupt,
                    )?;
                    consumed_versions.extend(
                        source
                            .consumed_inputs()
                            .iter()
                            .map(|input| (input.body, input.producer)),
                    );
                    solid
                }
                (model::BOOLEAN_TYPE, model::BOOLEAN_SCHEMA) => {
                    let source =
                        model::read_boolean(document, id).map_err(|e| error(Some(id), e))?;
                    source
                        .target
                        .resolve_producer(document)
                        .map_err(|e| error(Some(id), e))?;
                    let target = solids.get(&source.target.producer).ok_or_else(|| {
                        error(Some(id), "Boolean target has no current exact solid")
                    })?;
                    let operation = match source.operation {
                        Operation::Join => multicad_kernel::BooleanOperation::Union,
                        Operation::Cut => multicad_kernel::BooleanOperation::Subtract,
                        Operation::Intersect => multicad_kernel::BooleanOperation::Intersect,
                        _ => {
                            return Err(error(
                                Some(id),
                                "Unsupported standalone Boolean operation",
                            ));
                        }
                    };
                    let mut result: Option<Solid> = None;
                    for input in &source.tools {
                        checkpoint(interrupt)?;
                        input
                            .resolve_producer(document)
                            .map_err(|e| error(Some(id), e))?;
                        let tool = solids.get(&input.producer).ok_or_else(|| {
                            error(Some(id), "Boolean tool has no current exact solid")
                        })?;
                        result=Some(result.as_ref().unwrap_or(target).boolean(tool,operation,self.policy).map_err(|e|error(Some(id),e))?.ok_or_else(||error(Some(id),"Boolean removed all material; no output body can be committed"))?);
                    }
                    let solid = result.ok_or_else(|| error(Some(id), "Boolean requires a tool"))?;
                    if solid.solid_count().map_err(|e| error(Some(id), e))? != 1 {
                        return Err(error(
                            Some(id),
                            "Boolean produced disconnected fragments; independent Split outputs are not supported yet",
                        ));
                    }
                    if !source.keep_tools {
                        consumed_versions
                            .extend(source.tools.iter().map(|tool| (tool.body, tool.producer)));
                    }
                    solid
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
        let consumed_bodies = bodies
            .iter()
            .filter_map(|(body, producer)| {
                consumed_versions
                    .contains(&(*body, *producer))
                    .then_some(*body)
            })
            .collect::<BTreeSet<_>>();
        bodies.retain(|body, _| !consumed_bodies.contains(body));
        Ok(Model {
            policy: self.policy,
            solids,
            snapshot: ModelSnapshot {
                stamp,
                shapes,
                bodies,
                occurrences,
                consumed_bodies,
                cut_targets: vec![],
                references,
            },
        })
    }
}
