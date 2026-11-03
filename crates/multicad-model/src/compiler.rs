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
#[path = "advanced_profiles.rs"]
mod advanced_profiles;

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
/// Source inspection is deliberately incomplete and cannot authorize a full-model operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvaluationScope {
    Full,
    Sources(Vec<EntityId>),
}
#[derive(Clone, Debug)]
pub struct ModelSnapshot {
    pub scope: EvaluationScope,
    pub stamp: ModelStamp,
    /// Includes upstream producers needed by attached sketches and repair.
    pub shapes: BTreeMap<EntityId, Arc<ShapeSnapshot>>,
    /// Canonical Part identity to its final unsuppressed producing feature.
    pub bodies: BTreeMap<EntityId, EntityId>,
    /// Current canonical Part definitions, including consumed tool geometry.
    /// These are reference sources, not additional physical output occurrences.
    pub body_sources: BTreeMap<EntityId, EntityId>,
    /// Explicit occurrence poses/world topology; component meshes remain shared.
    pub occurrences: BTreeMap<EntityId, Arc<crate::OccurrenceSnapshot>>,
    /// Final physical Part leaves only; source definitions are accessed explicitly.
    pub leaf_occurrences: BTreeMap<model::OccurrencePath, Arc<crate::LeafOccurrenceSnapshot>>,
    /// Canonical tool Parts retained as source/history but consumed by an
    /// unsuppressed Boolean rather than emitted as independent final bodies.
    pub consumed_bodies: BTreeSet<EntityId>,
    /// Optional derived query receipts for this exact evaluation request.
    pub cut_targets: Vec<CutTargetProbeResult>,
    /// Actual native transfer profile count by evaluated receiving feature.
    pub transfer_counts: BTreeMap<EntityId, usize>,
    pub references: ReferenceReport,
    /// Service envelope boxes in world millimeters (bottom four corners then
    /// top four), for display. Clearance checks use `Model::envelopes`.
    pub envelope_boxes: BTreeMap<EntityId, [[f64; 3]; 8]>,
    /// Drawing guide outlines (cut planes, section and elevation frames) as
    /// closed four-corner loops in world millimeters, for display.
    pub guide_outlines: BTreeMap<EntityId, Vec<[[f64; 3]; 4]>>,
    /// Cabinetry panel frames by panel feature: thickness runs along u × v.
    pub panel_frames: BTreeMap<EntityId, PlaneFrame>,
    /// Solved weldment member path segments by member feature.
    pub member_paths: BTreeMap<EntityId, model::PathSegment>,
    /// Sheet-metal base layouts by sheet feature.
    pub sheet_layouts: BTreeMap<EntityId, crate::sheet_metal::SheetLayout>,
    /// FDM aid inspections of a full evaluation, so a preview can show and
    /// gate an aid's contacts; an inspection failure is kept as its message.
    pub fdm_aids: Result<Vec<crate::AidInspection>, String>,
}
impl ModelSnapshot {
    pub fn transfer_evaluation(&self) -> model::TransferEvaluationInfo {
        model::TransferEvaluationInfo {
            source_hash: self.stamp.source_hash,
            counts: self.transfer_counts.clone(),
        }
    }
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
/// Whether an evaluation error reports an interrupted (superseded) request
/// rather than a problem with the model source.
pub fn is_superseded(error: &ModelError) -> bool {
    error.message.ends_with("superseded")
}
pub(crate) fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ModelError> {
    if interrupt.is_some_and(|f| f()) {
        Err(error(None, "Model evaluation superseded"))
    } else {
        Ok(())
    }
}

struct Profiles<'a> {
    context: &'a SketchEvaluationContext<'a>,
    queries: NativeRegionQueries,
    body_heads: &'a BTreeMap<EntityId, EntityId>,
    policy: GeometryPolicy,
    cache: BTreeMap<EntityId, (multicad_core::sketch::Frame, Extraction)>,
}
impl Profiles<'_> {
    fn get(
        &mut self,
        sketch: EntityId,
        intent: &RegionIntent,
        feature: EntityId,
        solids: &BTreeMap<EntityId, Solid>,
        topologies: &BTreeMap<EntityId, TopologySnapshot>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(CurvedProfile, Vec<Origin>), ModelError> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.cache.entry(sketch) {
            let mut report = references::inspect(
                self.context.document(),
                self.context.stamp().revision,
                topologies,
            );
            crate::external::Sources {
                document: self.context.document(),
                revision: self.context.stamp().revision,
                parameters: self.context.parameters(),
                body_heads: self.body_heads,
                solids,
                topologies,
                policy: self.policy,
            }
            .populate(&mut report, Some(sketch), interrupt)?;
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
                role: edge
                    .fragment
                    .map(|hash| format!("profile.edge.{hash}"))
                    .unwrap_or_else(|| "profile.edge".into()),
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
    /// Sketch regions or an exact current planar model face (CAD-146).
    fn model(
        &mut self,
        profile: &model::ModelProfile,
        feature: EntityId,
        solids: &BTreeMap<EntityId, Solid>,
        topologies: &BTreeMap<EntityId, TopologySnapshot>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(CurvedProfile, Vec<Origin>), ModelError> {
        match profile {
            model::ModelProfile::Sketch(source) => self.get(
                source.sketch,
                &source.region,
                feature,
                solids,
                topologies,
                interrupt,
            ),
            model::ModelProfile::Face(face) => crate::face_profile::FaceProfileInputs {
                document: self.context.document(),
                revision: self.context.stamp().revision,
                feature,
                solids,
                topologies,
                policy: self.policy,
            }
            .resolve(face.face),
        }
    }
}

/// An evaluated model owns genuine B-reps, never reconstructed display meshes.
/// The kernel's !Send/!Sync ownership also applies to this container.
pub struct Model {
    pub(crate) policy: GeometryPolicy,
    pub(crate) solids: BTreeMap<EntityId, Solid>,
    snapshot: ModelSnapshot,
    leaves: crate::assemblies::LeafStore,
    gear_pair_checks: BTreeMap<EntityId, crate::GearPairCheck>,
}
impl Model {
    pub fn gear_pair_checks(&self) -> &BTreeMap<EntityId, crate::GearPairCheck> {
        &self.gear_pair_checks
    }
    pub fn leaf_solid(&self, path: &model::OccurrencePath) -> Option<&Solid> {
        self.leaves.solids.get(path).or_else(|| {
            path.members
                .is_empty()
                .then(|| self.solid(path.root))
                .flatten()
        })
    }
    pub fn leaf_snapshot(
        &self,
        path: &model::OccurrencePath,
    ) -> Option<&crate::LeafOccurrenceSnapshot> {
        self.leaves.snapshots.get(path).map(Arc::as_ref)
    }
    pub fn assembly_evaluation(&self) -> model::AssemblyEvaluationInfo {
        model::AssemblyEvaluationInfo {
            source_hash: self.snapshot.stamp.source_hash,
            leaves: self
                .snapshot
                .leaf_occurrences
                .values()
                .map(|leaf| model::AssemblyLeafReceipt {
                    path: leaf.path.clone(),
                    component: leaf.component,
                    producer: leaf.producer,
                    pose: leaf.pose,
                })
                .collect(),
        }
    }
    pub fn snapshot(&self) -> &ModelSnapshot {
        &self.snapshot
    }
    pub fn solid(&self, source: EntityId) -> Option<&Solid> {
        self.solids.get(&source).or_else(|| {
            self.snapshot
                .body_sources
                .get(&source)
                .and_then(|producer| self.solids.get(producer))
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Compiler {
    pub policy: GeometryPolicy,
    pub mesh: MeshOptions,
    /// Exact derived operations can omit viewport-only tessellation and picking
    /// geometry. Native solids, topology and dimensional references remain complete.
    pub omit_display: bool,
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
        crate::worker_cache::with_current_cache(|cache| {
            self.compile_scoped(document, revision, None, interrupt, cache)
        })
    }
    /// A full compile that reuses unchanged feature results from earlier
    /// compiles through `cache` and records new ones (see [`EvaluationCache`]).
    /// The result is identical to [`Self::compile_interruptible`].
    pub fn compile_cached(
        &self,
        cache: &mut crate::EvaluationCache,
        document: &Document,
        revision: u64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Model, ModelError> {
        self.compile_scoped(document, revision, None, interrupt, Some(cache))
    }
    /// [`Self::compile_sources_interruptible`] through an evaluation cache.
    pub fn compile_sources_cached(
        &self,
        cache: &mut crate::EvaluationCache,
        document: &Document,
        revision: u64,
        roots: &[EntityId],
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Model, ModelError> {
        let roots = validate_source_roots(document, roots)?;
        self.compile_scoped(document, revision, Some(roots), interrupt, Some(cache))
    }
    /// Evaluate only explicitly selected current source roots and their dependencies.
    /// The stamp still covers the complete canonical document. The result is an
    /// incomplete inspection model, never a substitute for a full Apply/export.
    pub fn compile_sources(
        &self,
        document: &Document,
        revision: u64,
        roots: &[EntityId],
    ) -> Result<Model, ModelError> {
        self.compile_sources_interruptible(document, revision, roots, None)
    }
    pub fn compile_sources_interruptible(
        &self,
        document: &Document,
        revision: u64,
        roots: &[EntityId],
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Model, ModelError> {
        let roots = validate_source_roots(document, roots)?;
        crate::worker_cache::with_current_cache(|cache| {
            self.compile_scoped(document, revision, Some(roots), interrupt, cache)
        })
    }
    /// Nested evaluators report their own cancellation wording (for example
    /// a sketch "cancelled" or a wrapped native failure). A failure observed
    /// while the request is interrupted is reported as supersession so that
    /// callers never present an abandoned rebuild as a modeling error.
    fn compile_scoped(
        &self,
        document: &Document,
        revision: u64,
        roots: Option<Vec<EntityId>>,
        interrupt: Option<&dyn Fn() -> bool>,
        mut cache: Option<&mut crate::EvaluationCache>,
    ) -> Result<Model, ModelError> {
        if let Some(cache) = cache.as_deref_mut() {
            cache.begin_symbol_proofs();
        }
        let result = self
            .compile_scoped_unchecked(document, revision, roots, interrupt, cache.as_deref_mut())
            .and_then(|model| {
                checkpoint(interrupt)?;
                Ok(model)
            })
            .map_err(|e| {
                if !is_superseded(&e) && interrupt.is_some_and(|f| f()) {
                    error(e.entity, "Model evaluation superseded")
                } else {
                    e
                }
            });
        if let Some(cache) = cache {
            cache.finish_symbol_proofs(result.is_ok());
        }
        result
    }
    fn compile_scoped_unchecked(
        &self,
        document: &Document,
        revision: u64,
        roots: Option<Vec<EntityId>>,
        interrupt: Option<&dyn Fn() -> bool>,
        mut cache: Option<&mut crate::EvaluationCache>,
    ) -> Result<Model, ModelError> {
        checkpoint(interrupt)?;
        document.validate().map_err(|e| error(None, e))?;
        self.policy.validate().map_err(|e| error(None, e))?;
        if roots.is_none() {
            crate::symbols::validate_definitions(
                self,
                document,
                revision,
                interrupt,
                cache.as_deref_mut(),
            )?;
        }
        let context =
            SketchEvaluationContext::new(document, revision).map_err(|e| error(e.source, e))?;
        if roots.is_none() {
            crate::gear_pairs::validate_parameters(document, context.parameters())?;
            for entity in model::carcasses(document) {
                checkpoint(interrupt)?;
                model::check_cabinet_connections(
                    document,
                    context.parameters(),
                    entity.id,
                    interrupt,
                )
                .map_err(|e| error(Some(entity.id), e))?;
            }
        }
        let stamp = ModelStamp {
            document: document.id(),
            revision,
            source_hash: context.stamp().source_hash,
        };
        let mut bodies = BTreeMap::new();
        for entity in document.entities() {
            if let EntityData::Part { features, .. } = &entity.data {
                for feature in features {
                    if roots.is_none()
                        && !matches!(
                            document.get(*feature).map(|e| &e.data),
                            Some(EntityData::Feature { .. })
                        )
                    {
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
        let mut pending: Vec<_> = roots.clone().unwrap_or_else(|| {
            bodies
                .values()
                .copied()
                .chain(
                    document
                        .entities()
                        .filter(|e| matches!(&e.data, EntityData::Instance { .. })||matches!(&e.data,EntityData::Assembly{instances}if !instances.is_empty()))
                        .map(|e| e.id),
                )
                .collect()
        });
        while let Some(id) = pending.pop() {
            if needed.insert(id) {
                let entity = document
                    .get(id)
                    .ok_or_else(|| error(Some(id), "Missing model dependency"))?;
                if let EntityData::Part { features, .. } = &entity.data {
                    if features.iter().any(|id| {
                        !matches!(
                            document.get(*id).map(|e| &e.data),
                            Some(EntityData::Feature { .. })
                        )
                    }) {
                        return Err(error(
                            Some(id),
                            "Source body history contains a missing or invalid producer",
                        ));
                    }
                    if let Some(head) = bodies.get(&id) {
                        pending.push(*head);
                    }
                } else {
                    pending.extend(
                        document
                            .evaluation_dependencies(id)
                            .map_err(|e| error(Some(id), e))?,
                    );
                }
            }
        }
        let mut cached = cache.as_deref_mut().map(|cache| {
            crate::eval_cache::Session::new(cache, self, document, context.parameters())
        });
        let mut profiles = Profiles {
            context: &context,
            queries: NativeRegionQueries::new(self.policy).map_err(|e| error(None, e))?,
            body_heads: &bodies,
            policy: self.policy,
            cache: BTreeMap::new(),
        };
        let mut solids: BTreeMap<EntityId, Solid> = BTreeMap::new();
        let mut consumed_versions = BTreeSet::new();
        let mut shapes = BTreeMap::new();
        let mut occurrences = BTreeMap::new();
        let mut transfer_counts = BTreeMap::new();
        let mut panel_frames = BTreeMap::new();
        let mut member_paths = BTreeMap::new();
        let mut weld_paths = crate::weldment::Paths::new(&context);
        let mut sheet_layouts = BTreeMap::new();
        let mut fences = crate::fences::Context::new(document, context.parameters());
        let mut compound_cutouts = crate::compound_cutouts::Context::default();
        let mut leaves = crate::assemblies::LeafStore::default();
        let mut topologies: BTreeMap<EntityId, TopologySnapshot> = BTreeMap::new();
        let mut triangles = 0usize;
        for id in document.topological_order().map_err(|e| error(None, e))? {
            checkpoint(interrupt)?;
            let entity = document
                .get(id)
                .ok_or_else(|| error(Some(id), "Missing model source"))?;
            if matches!(entity.data, EntityData::Part { .. }) {
                if roots.is_some() && !needed.contains(&id) {
                    continue;
                }
                if let Some(head) = bodies.get(&id) {
                    let mut topology = topologies
                        .get(head)
                        .ok_or_else(|| {
                            error(Some(id), "Current Part head has no evaluated topology")
                        })?
                        .clone();
                    topology.owner = id;
                    topologies.insert(id, topology);
                }
                continue;
            }
            if !needed.contains(&id) {
                continue;
            }
            if matches!(entity.data, EntityData::Assembly { .. }) {
                if matches!(&entity.data,EntityData::Assembly{instances}if instances.is_empty()) {
                    continue;
                }
                let solid = crate::assemblies::component(
                    document,
                    id,
                    &occurrences,
                    &solids,
                    &mut leaves,
                    revision,
                    self.policy,
                )?;
                let topology = solid
                    .topology_snapshot(id, revision)
                    .map_err(|e| error(Some(id), e))?;
                multicad_core::model::wall_face_materials(document, &topology)
                    .map_err(|e| error(Some(id), e))?;
                let snapshot = ShapeSnapshot {
                    producer: id,
                    bounds: solid.bounds().map_err(|e| error(Some(id), e))?,
                    volume_mm3: solid.volume_mm3().map_err(|e| error(Some(id), e))?,
                    solid_count: solid.solid_count().map_err(|e| error(Some(id), e))?,
                    mesh: if self.omit_display {
                        Mesh {
                            vertices: vec![],
                            indices: vec![],
                            face_indices: vec![],
                        }
                    } else {
                        solid.mesh(self.mesh).map_err(|e| error(Some(id), e))?
                    },
                    edges: if self.omit_display {
                        vec![]
                    } else {
                        solid
                            .display_edges(self.mesh.linear_deflection_mm)
                            .map_err(|e| error(Some(id), e))?
                    },
                    vertices: if self.omit_display {
                        vec![]
                    } else {
                        solid.display_vertices().map_err(|e| error(Some(id), e))?
                    },
                    topology: topology.clone(),
                };
                triangles = triangles.saturating_add(snapshot.mesh.indices.len() / 3);
                if triangles > MAX_MODEL_TRIANGLES {
                    return Err(error(
                        Some(id),
                        "Assembly display exceeds the native triangle budget",
                    ));
                }
                topologies.insert(id, topology);
                shapes.insert(id, Arc::new(snapshot));
                solids.insert(id, solid);
                continue;
            }
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
                leaves.instance(document, &occurrence, revision)?;
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
            // CAD-150: an unchanged result key reuses the earlier exact result.
            let fence_source = if definition.type_id == model::FENCE_MEMBER_TYPE {
                Some(fences.prepare(id, interrupt)?)
            } else {
                None
            };
            let key = cached.as_mut().and_then(|c| c.key(id));
            if let Some((solid, shape, extras)) =
                key.and_then(|key| cached.as_mut().and_then(|c| c.lookup(&key, revision)))
            {
                extras.apply(
                    id,
                    crate::eval_cache::Sides {
                        panel_frames: &mut panel_frames,
                        sheet_layouts: &mut sheet_layouts,
                        member_paths: &mut member_paths,
                        consumed: &mut consumed_versions,
                    },
                );
                triangles = triangles.saturating_add(shape.mesh.indices.len() / 3);
                if triangles > MAX_MODEL_TRIANGLES {
                    return Err(error(
                        Some(id),
                        "Model display exceeds the triangle budget; use coarser detail",
                    ));
                }
                multicad_core::model::wall_face_materials(document, &shape.topology)
                    .map_err(|e| error(Some(id), e))?;
                topologies.insert(id, shape.topology.clone());
                shapes.insert(id, shape);
                solids.insert(id, solid);
                continue;
            }
            if let Some(cached) = cached.as_mut() {
                cached.evaluated(id, key.is_some());
            }
            let consumed_before = key.map(|_| consumed_versions.clone());
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
                    let (profile, origins) =
                        profiles.model(&source.profile, id, &solids, &topologies, interrupt)?;
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
                (model::SLAB_TYPE, 1) => {
                    crate::levels::build_slab(document, context.parameters(), id, self.policy)?
                }
                (model::COMPOUND_CUTOUT_TYPE, model::COMPOUND_CUTOUT_SCHEMA) => compound_cutouts
                    .build(
                        document,
                        context.parameters(),
                        id,
                        &solids,
                        self.policy,
                        interrupt,
                    )?,
                (model::COMPOUND_CUTOUT_RECEIVER_TYPE, model::COMPOUND_CUTOUT_SCHEMA) => {
                    compound_cutouts.take(id)?
                }
                (model::THREAD_TYPE, model::THREAD_SCHEMA) => {
                    let source =
                        model::read_thread(document, id).map_err(|e| error(Some(id), e))?;
                    crate::threads::build(id, &source, context.parameters(), &solids, self.policy)?
                }
                (model::MECHANICAL_PART_TYPE, model::MECHANICAL_PART_SCHEMA) => {
                    let source = model::read_mechanical_part(document, id)
                        .map_err(|e| error(Some(id), e))?;
                    crate::mechanical_parts::build(id, &source, context.parameters(), self.policy)?
                }
                (model::GEAR_TYPE, model::GEAR_SCHEMA) => {
                    let source = model::read_gear(document, id).map_err(|e| error(Some(id), e))?;
                    crate::gears::build(id, &source, context.parameters(), self.policy)?
                }
                (model::STAIR_TYPE, 1) => {
                    crate::levels::build_stair(document, context.parameters(), id, self.policy)?
                }
                (model::PANEL_TYPE, model::PANEL_SCHEMA | model::RECIPE_PANEL_SCHEMA) => {
                    let (solid, frame) = crate::cabinetry::build(
                        document,
                        context.parameters(),
                        id,
                        self.policy,
                        |sketch, region| {
                            profiles.get(sketch, region, id, &solids, &topologies, interrupt)
                        },
                    )?;
                    panel_frames.insert(id, frame);
                    solid
                }
                (model::FENCE_MEMBER_TYPE, model::FENCE_SCHEMA) => fences.build(
                    id,
                    fence_source.as_ref().expect("prepared fence source"),
                    self.policy,
                    interrupt,
                )?,
                (model::PANEL_JOINT_CUT_TYPE, model::PANEL_JOINT_CUT_SCHEMA) => {
                    crate::cabinetry::build_joint_cut(
                        document,
                        context.parameters(),
                        id,
                        self.policy,
                        &solids,
                    )?
                }
                (model::LOOSE_TENON_TYPE, model::LOOSE_TENON_SCHEMA) => {
                    crate::cabinetry::build_loose_tenon(
                        document,
                        context.parameters(),
                        id,
                        self.policy,
                    )?
                }
                (model::MACHINING_CUT_TYPE, model::MACHINING_CUT_SCHEMA) => {
                    crate::cabinetry::build_machining_cut(
                        document,
                        context.parameters(),
                        id,
                        self.policy,
                        &solids,
                    )?
                }
                (model::imports::STEP_BODY_TYPE, 1) => {
                    crate::imports::build_step_body(document, id, self.policy)?
                }
                (model::imports::BOARD_TYPE, 1) => {
                    crate::imports::build_board(document, id, self.policy)?
                }
                (model::imports::COMPONENT_TYPE, 1) => {
                    crate::imports::build_component(document, id, self.policy)?
                }
                (model::AID_TYPE, model::AID_SCHEMA) => {
                    let target = model::read_aid(document, id)
                        .map_err(|e| error(Some(id), e))?
                        .target;
                    let target = bodies.get(&target).and_then(|head| solids.get(head));
                    crate::fdm::build(document, context.parameters(), id, self.policy, target)?
                }
                (model::MEMBER_TYPE, model::MEMBER_SCHEMA) => {
                    let (solid, member) = crate::weldment::build(
                        document,
                        context.parameters(),
                        id,
                        self.policy,
                        &mut weld_paths,
                        interrupt,
                    )?;
                    member_paths.insert(id, member.path);
                    solid
                }
                (model::SHEET_TYPE, model::SHEET_SCHEMA) => {
                    let (solid, layout) = crate::sheet_metal::build(
                        document,
                        context.parameters(),
                        id,
                        self.policy,
                        |sketch, region| {
                            profiles.get(sketch, region, id, &solids, &topologies, interrupt)
                        },
                    )?;
                    sheet_layouts.insert(id, layout);
                    solid
                }
                (model::HARDWARE_TYPE, model::HARDWARE_SCHEMA) => {
                    crate::sheet_hardware::build(document, id, &sheet_layouts, self.policy)?
                }
                (model::WALL_TYPE, model::WALL_SCHEMA | model::LAYERED_WALL_SCHEMA) => {
                    crate::walls::build(
                        document,
                        context.parameters(),
                        id,
                        self.policy,
                        &solids,
                        interrupt,
                    )?
                }
                (model::REVOLVE_TYPE, model::REVOLVE_SCHEMA) => {
                    let source =
                        model::read_revolve(document, id).map_err(|e| error(Some(id), e))?;
                    let resolved = source
                        .parameters
                        .resolve(context.parameters())
                        .map_err(|e| error(Some(id), e))?;
                    let (profile, origins) =
                        profiles.model(&source.profile, id, &solids, &topologies, interrupt)?;
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
                (model::SCALE_TYPE, model::SCALE_SCHEMA) => {
                    let source = model::read_scale(document, id).map_err(|e| error(Some(id), e))?;
                    crate::scale::build(
                        crate::scale::Inputs {
                            document,
                            revision,
                            parameters: context.parameters(),
                            body_heads: &bodies,
                            solids: &solids,
                            topologies: &topologies,
                            policy: self.policy,
                        },
                        id,
                        &source,
                    )?
                }
                (
                    model::EDGE_TREATMENT_TYPE,
                    model::EDGE_TREATMENT_SCHEMA | model::EDGE_TREATMENT_V2_SCHEMA,
                ) => {
                    let (source, captures) =
                        model::read_edge_treatment(document, id).map_err(|e| error(Some(id), e))?;
                    crate::edge_treatment::build(
                        crate::edge_treatment::Inputs {
                            revision,
                            parameters: context.parameters(),
                            solids: &solids,
                            topologies: &topologies,
                            policy: self.policy,
                        },
                        id,
                        &source,
                        &captures,
                    )?
                }
                (model::TRANSFER_TYPE, 1) => {
                    let (_, source, target, captures) = model::read_transfer_feature(document, id)
                        .map_err(|e| error(Some(id), e))?;
                    let result = crate::transfer::build(
                        crate::transfer::Inputs {
                            document,
                            revision,
                            parameters: context.parameters(),
                            body_heads: &bodies,
                            solids: &solids,
                            topologies: &topologies,
                            policy: self.policy,
                        },
                        id,
                        &source,
                        &target,
                        &captures,
                    )?;
                    transfer_counts.insert(id, result.count);
                    result.solid
                }
                (model::ADVANCED_TYPE, model::ADVANCED_SCHEMA) => {
                    let (source, captures) =
                        model::read_advanced(document, id).map_err(|e| error(Some(id), e))?;
                    let selected =
                        profiles.advanced(&source, id, &solids, &topologies, interrupt)?;
                    crate::advanced::build(
                        crate::advanced::Inputs {
                            revision,
                            parameters: context.parameters(),
                            solids: &solids,
                            topologies: &topologies,
                            policy: self.policy,
                            sections: selected.sections,
                            path: selected.path,
                        },
                        id,
                        &source,
                        &captures,
                    )?
                }
                (model::SPLIT_TYPE, model::SPLIT_V2_SCHEMA) => {
                    let source =
                        model::read_split_v2(document, id).map_err(|e| error(Some(id), e))?;
                    let solid = crate::split::ExplicitSplit {
                        document,
                        revision,
                        parameters: context.parameters(),
                        solids: &solids,
                        topologies: &topologies,
                        policy: self.policy,
                        interrupt,
                    }
                    .build(id, &source)?;
                    consumed_versions.extend(
                        source
                            .consumed_inputs()
                            .iter()
                            .map(|input| (input.body, input.producer)),
                    );
                    solid
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
                mesh: if self.omit_display {
                    Mesh {
                        vertices: vec![],
                        indices: vec![],
                        face_indices: vec![],
                    }
                } else {
                    solid.mesh(self.mesh).map_err(|e| error(Some(id), e))?
                },
                edges: if self.omit_display {
                    vec![]
                } else {
                    solid
                        .display_edges(self.mesh.linear_deflection_mm)
                        .map_err(|e| error(Some(id), e))?
                },
                vertices: if self.omit_display {
                    vec![]
                } else {
                    solid.display_vertices().map_err(|e| error(Some(id), e))?
                },
                topology: topology.clone(),
            };
            triangles = triangles.saturating_add(snapshot.mesh.indices.len() / 3);
            if triangles > MAX_MODEL_TRIANGLES {
                return Err(error(
                    Some(id),
                    "Model display exceeds the triangle budget; use coarser detail",
                ));
            }
            let snapshot = Arc::new(snapshot);
            if let (Some(cached), Some(key), Some(before)) = (cached.as_mut(), key, consumed_before)
            {
                let extras = crate::eval_cache::capture(
                    id,
                    crate::eval_cache::Sides {
                        panel_frames: &mut panel_frames,
                        sheet_layouts: &mut sheet_layouts,
                        member_paths: &mut member_paths,
                        consumed: &mut consumed_versions,
                    },
                    &before,
                );
                cached.store(key, &solid, &snapshot, extras);
            }
            topologies.insert(id, topology);
            shapes.insert(id, snapshot);
            solids.insert(id, solid);
        }
        drop(cached);
        let gear_pair_checks = if roots.is_none() {
            crate::gear_pairs::validate(
                document,
                context.parameters(),
                &solids,
                self.policy,
                interrupt,
                cache.map(|c| &mut c.gear_pairs),
            )?
        } else {
            BTreeMap::new()
        };
        checkpoint(interrupt)?;
        let mut references = references::inspect(document, revision, &topologies);
        crate::external::complete_report(
            self,
            &crate::external::Sources {
                document,
                revision,
                parameters: context.parameters(),
                body_heads: &bodies,
                solids: &solids,
                topologies: &topologies,
                policy: self.policy,
            },
            &mut references,
            roots.is_none(),
            interrupt,
        )?;
        if roots.is_some() {
            bodies.retain(|_, producer| shapes.contains_key(producer));
        }
        let consumed_bodies = bodies
            .iter()
            .filter_map(|(body, producer)| {
                consumed_versions
                    .contains(&(*body, *producer))
                    .then_some(*body)
            })
            .collect::<BTreeSet<_>>();
        let body_sources = bodies.clone();
        bodies.retain(|body, _| !consumed_bodies.contains(body));
        for body in body_sources.keys() {
            if model::scale_component_owner(document, *body)
                .map_err(|e| error(Some(*body), e))?
                .is_some()
            {
                bodies.remove(body);
            }
        }
        let definitions = model::assembly_definition_parts(document).map_err(|e| error(None, e))?;
        let reusable = model::reusable_assemblies(document).map_err(|e| error(None, e))?;
        let referenced_components = document
            .entities()
            .filter_map(|e| {
                if let EntityData::Instance { component, .. } = e.data {
                    Some(component)
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        for entity in document.entities() {
            if matches!(&entity.data,EntityData::Assembly{instances}if !instances.is_empty())
                && !reusable.contains(&entity.id)
                && !referenced_components.contains(&entity.id)
                && let Some(shape) = shapes.get(&entity.id)
            {
                occurrences.insert(
                    entity.id,
                    Arc::new(crate::OccurrenceSnapshot {
                        instance: entity.id,
                        component: entity.id,
                        producer: entity.id,
                        pose: Default::default(),
                        bounds: shape.bounds,
                        topology: shape.topology.clone(),
                    }),
                );
            }
        }
        let members = model::assembly_member_instances(document).map_err(|e| error(None, e))?;
        bodies.retain(|body, _| !definitions.contains(body));
        occurrences.retain(|instance, _| !members.contains(instance));
        for (body, producer) in &body_sources {
            if let Some(shape) = shapes.get(producer) {
                leaves.part(
                    *body,
                    *body,
                    *producer,
                    Default::default(),
                    shape.bounds,
                    shape.topology.clone(),
                )?;
            }
        }
        let leaf_occurrences = leaves
            .snapshots
            .iter()
            .filter(|(path, _)| {
                bodies.contains_key(&path.root) || occurrences.contains_key(&path.root)
            })
            .map(|(path, leaf)| (path.clone(), leaf.clone()))
            .collect::<BTreeMap<_, _>>();
        if leaf_occurrences.len() > model::MAX_ASSEMBLY_LEAVES {
            return Err(error(
                None,
                format!(
                    "Physical Assembly expansion exceeds the {}-leaf budget",
                    model::MAX_ASSEMBLY_LEAVES
                ),
            ));
        }
        let full = roots.is_none();
        let mut model = Model {
            policy: self.policy,
            solids,
            leaves,
            gear_pair_checks,
            snapshot: ModelSnapshot {
                scope: roots.map_or(EvaluationScope::Full, EvaluationScope::Sources),
                stamp,
                shapes,
                bodies,
                body_sources,
                occurrences,
                leaf_occurrences,
                consumed_bodies,
                cut_targets: vec![],
                transfer_counts,
                references,
                envelope_boxes: BTreeMap::new(),
                guide_outlines: BTreeMap::new(),
                panel_frames,
                member_paths,
                sheet_layouts,
                fdm_aids: Ok(vec![]),
            },
        };
        if full {
            model.snapshot.envelope_boxes = model.envelope_boxes(document);
            model.snapshot.guide_outlines = model.guide_outlines(document);
            if model::aids(document).next().is_some() {
                model.snapshot.fdm_aids = model.fdm_aids(document).map_err(|e| e.to_string());
            }
        }
        Ok(model)
    }
}

/// Explicit source queries are bounded and preserve identity rather than silently
/// substituting the latest display geometry or guessing sibling producers.
pub fn validate_source_roots(
    document: &Document,
    roots: &[EntityId],
) -> Result<Vec<EntityId>, ModelError> {
    let unique: BTreeSet<_> = roots.iter().copied().collect();
    if roots.is_empty()
        || roots.len() > 128
        || unique.len() != roots.len()
        || unique.iter().any(EntityId::is_nil)
    {
        return Err(error(
            None,
            "Source inspection requires 1..128 unique non-nil roots",
        ));
    }
    for id in &unique {
        if !matches!(
            document.get(*id).map(|e| &e.data),
            Some(
                EntityData::Feature { .. }
                    | EntityData::Part { .. }
                    | EntityData::Instance { .. }
                    | EntityData::Assembly { .. }
                    | EntityData::Plane { .. }
            )
        ) {
            return Err(error(
                Some(*id),
                "Source inspection roots must be Features, Parts, Instances or sketch Planes",
            ));
        }
    }
    Ok(unique.into_iter().collect())
}

#[cfg(test)]
mod owner_cache_scope_tests {
    use super::*;
    use multicad_core::commands::{Actor, Channel, Session};

    fn fixture() -> (Document, EntityId) {
        let mut session = Session::new(Document::new("Owner cache").unwrap()).unwrap();
        let request = model::PrimitiveRequest::new(
            "Box",
            model::PrimitiveParameters::Box {
                size: [model::length(10.), model::length(20.), model::length(5.)],
                frame: Default::default(),
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        (session.document().clone(), request.feature)
    }
    fn reused(feature: EntityId) -> bool {
        crate::worker_cache::with_current_cache(|cache| {
            cache.unwrap().last_compile().reused.contains(&feature)
        })
    }
    #[test]
    fn ordinary_and_source_compilers_share_only_the_current_owner_scope() {
        let (document, feature) = fixture();
        crate::worker_cache::with_current_cache(|cache| assert!(cache.is_none()));
        crate::with_evaluation_cache(|| {
            let compiler = Compiler::default();
            compiler.compile(&document, 1).unwrap();
            assert!(!reused(feature));
            compiler.compile(&document, 2).unwrap();
            assert!(reused(feature));
            compiler.compile_sources(&document, 3, &[feature]).unwrap();
            assert!(reused(feature));
            // Holding the ambient mutable borrow makes recursive compilation
            // deliberately uncached rather than panicking on RefCell reentry.
            crate::worker_cache::with_current_cache(|cache| {
                assert!(cache.is_some());
                compiler.compile(&document, 4).unwrap();
            });
            let other_document = document.clone();
            std::thread::spawn(move || {
                crate::worker_cache::with_current_cache(|cache| assert!(cache.is_none()));
                crate::with_evaluation_cache(|| {
                    Compiler::default().compile(&other_document, 1).unwrap();
                    assert!(!reused(feature));
                });
            })
            .join()
            .unwrap();
        });
        crate::worker_cache::with_current_cache(|cache| assert!(cache.is_none()));
        crate::with_evaluation_cache(|| {
            Compiler::default().compile(&document, 1).unwrap();
            assert!(!reused(feature));
        });
    }
}
