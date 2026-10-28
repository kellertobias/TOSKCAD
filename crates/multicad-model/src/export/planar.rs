use super::*;
use multicad_core::{
    manufacturing::PlanarContours,
    references::{
        self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind, TopologySnapshot,
    },
};
use multicad_kernel::PlaneFrame;
/// Optional native module seam. No developed-sheet producer is installed by
/// default; the application must not substitute projected folded geometry.
pub trait DevelopedContourProvider {
    fn contours(
        &self,
        document: &Document,
        model: &Model,
        feature: EntityId,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<StampedPlanarContours, ExportError>;
}

pub(super) fn binding(
    model: &Model,
    subject: &PartRef,
) -> Result<(EntityId, EntityId, EntityId, TopologySnapshot), ExportError> {
    if let Some(instance) = subject.occurrence_path.first() {
        let path = multicad_core::model::OccurrencePath {
            root: *instance,
            members: subject.occurrence_path[1..].to_vec(),
        };
        let o = model.leaf_snapshot(&path).ok_or_else(|| {
            failure(
                ErrorKind::Missing,
                Some(*instance),
                "Current planar occurrence is unavailable",
            )
        })?;
        if o.component != subject.part {
            return Err(failure(
                ErrorKind::Invalid,
                Some(*instance),
                "Planar occurrence does not match its Part",
            ));
        }
        Ok((*instance, *instance, o.producer, o.topology.clone()))
    } else {
        let producer = *model.snapshot().bodies.get(&subject.part).ok_or_else(|| {
            failure(
                ErrorKind::Unsupported,
                Some(subject.part),
                "A consumed or unavailable Part cannot provide current planar output",
            )
        })?;
        let mut topology = model
            .snapshot()
            .shapes
            .get(&producer)
            .ok_or_else(|| {
                failure(
                    ErrorKind::Missing,
                    Some(producer),
                    "Current native topology is missing",
                )
            })?
            .topology
            .clone();
        topology.owner = subject.part;
        Ok((subject.part, producer, producer, topology))
    }
}
impl Exporter {
    pub fn developed_contours(
        &self,
        document: &Document,
        revision: u64,
        feature: EntityId,
        provider: &dyn DevelopedContourProvider,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<StampedPlanarContours, ExportError> {
        if !matches!(
            document.get(feature).map(|e| &e.data),
            Some(multicad_core::document::EntityData::Feature {
                suppressed: false,
                ..
            })
        ) {
            return Err(failure(
                ErrorKind::Unsupported,
                Some(feature),
                "A current unsuppressed developed-source feature is required",
            ));
        }
        checkpoint(interrupt)?;
        let model = self
            .compiler
            .compile_interruptible(document, revision, interrupt)
            .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?;
        let result = provider.contours(document, &model, feature, interrupt)?;
        checkpoint(interrupt)?;
        result.stamp.check(document, revision)?;
        if result.source != feature {
            return Err(failure(
                ErrorKind::Invalid,
                Some(feature),
                "Developed contour producer identity differs from the requested feature",
            ));
        }
        result
            .contours
            .validate(self.compiler.policy.linear_tolerance_mm)
            .map_err(|e| failure(ErrorKind::Invalid, Some(feature), e))?;
        Ok(result)
    }
    pub fn planar_catalog(
        &self,
        document: &Document,
        revision: u64,
        scope: &Scope,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PlanarCatalog, ExportError> {
        checkpoint(interrupt)?;
        let parts = resolve_parts(document, scope)?;
        if parts.len() > MAX_PLANAR_BINDINGS {
            return Err(failure(
                ErrorKind::Budget,
                None,
                "Narrow the planar scope to at most 128 Part occurrences",
            ));
        }
        let model = self
            .compiler
            .compile_interruptible(document, revision, interrupt)
            .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?;
        let stamp = ExportStamp::capture(document, revision)?;
        let mut result = vec![];
        let mut total_faces = 0;
        for part in parts {
            checkpoint(interrupt)?;
            let mut value = PlanarPartChoices {
                subject: part.reference.clone(),
                name: part.name,
                faces: vec![],
                diagnostics: vec![],
            };
            match binding(&model, &part.reference) {
                Err(e) => value.diagnostics.push(e.message),
                Ok((source, solid, _, topology)) => {
                    let native = subject_solid(&model, &part.reference, solid)?;
                    let faces = native
                        .faces()
                        .map_err(|e| failure(ErrorKind::Invalid, Some(solid), e))?;
                    if faces.len() > 512 {
                        return Err(failure(
                            ErrorKind::Budget,
                            Some(source),
                            "Planar face catalog is bounded to 512 faces per source",
                        ));
                    }
                    for element in &topology.elements {
                        if element.key.kind != TopologyKind::Face {
                            continue;
                        }
                        let Some(AnalyticGeometry::Plane { frame }) = element.geometry else {
                            continue;
                        };
                        total_faces += 1;
                        if total_faces > MAX_PLANAR_FACE_CHOICES {
                            return Err(failure(
                                ErrorKind::Budget,
                                None,
                                "Narrow the planar face catalog scope; more than 4096 planar face choices are not supported",
                            ));
                        }
                        match GeometryIntent::capture(&topology, element.key) {
                            Ok(intent) => {
                                let face = faces
                                    .iter()
                                    .find(|f| f.index == element.key.ordinal)
                                    .ok_or_else(|| {
                                        failure(
                                            ErrorKind::Invalid,
                                            Some(source),
                                            "Native face index does not match exact topology",
                                        )
                                    })?;
                                value.faces.push(PlanarFaceChoice {
                                    source,
                                    intent,
                                    boundary: element.boundary.clone(),
                                    plane: frame,
                                    area_mm2: face.area_mm2,
                                    label: format!(
                                        "Face {} · normal [{:.3}, {:.3}, {:.3}] · {:.3} mm²",
                                        element.key.ordinal,
                                        frame.normal()[0],
                                        frame.normal()[1],
                                        frame.normal()[2],
                                        face.area_mm2
                                    ),
                                });
                            }
                            Err(e) => value
                                .diagnostics
                                .push(format!("Face {}: {e}", element.key.ordinal)),
                        }
                    }
                    if value.faces.is_empty() {
                        value
                            .diagnostics
                            .push("No uniquely named planar faces are available".into());
                    }
                }
            }
            result.push(value);
        }
        Ok(PlanarCatalog {
            stamp,
            scope: scope.clone(),
            parts: result,
        })
    }
}
pub(super) fn contours(
    document: &Document,
    model: &Model,
    set: EntityId,
    recipe: EntityId,
    subject: &PartRef,
) -> Result<PlanarContours, ExportError> {
    let definition = read(document, set)?;
    let face = definition
        .planar_faces
        .iter()
        .find(|f| f.recipe == recipe && f.subject == *subject)
        .ok_or_else(|| {
            failure(
                ErrorKind::Unsupported,
                Some(subject.part),
                "Choose an explicit current planar face for every matched Part occurrence",
            )
        })?;
    let links = geometry_links(document, set);
    let link = links
        .iter()
        .find(|l| l.geometry.as_ref().is_some_and(|g| g.id == face.reference))
        .ok_or_else(|| {
            failure(
                ErrorKind::Invalid,
                Some(set),
                "Planar face's persistent source link is missing",
            )
        })?;
    let (source, solid, _, topology) = binding(model, subject)?;
    if face.source != source || link.target != source {
        return Err(failure(
            ErrorKind::Invalid,
            Some(source),
            "Planar face source does not match resolved occurrence",
        ));
    }
    let intent = link.geometry.as_ref().expect("matching face link");
    let (key, plane) = match references::resolve(intent, &topology, model.snapshot().stamp.revision)
    {
        Resolution::Resolved {
            key,
            geometry: Some(AnalyticGeometry::Plane { frame }),
            ..
        } => (key, frame),
        other => {
            return Err(failure(
                ErrorKind::Unsupported,
                Some(source),
                format!("Planar face needs repair or is not planar: {other:?}"),
            ));
        }
    };
    let frame = face
        .horizontal
        .frame(plane)
        .map_err(|e| failure(ErrorKind::Unsupported, Some(source), e))?;
    subject_solid(model, subject, solid)?
        .planar_face_contours(
            key.ordinal,
            PlaneFrame {
                origin: frame.origin_mm,
                u: frame.u,
                v: frame.v,
            },
        )
        .map_err(|e| failure(ErrorKind::Unsupported, Some(source), e))
}
pub(super) fn evidence(contours: &PlanarContours) -> Result<PlanarEvidence, ExportError> {
    let bytes = serde_json::to_vec(contours).map_err(|e| failure(ErrorKind::Invalid, None, e))?;
    Ok(PlanarEvidence {
        frame: contours.frame,
        bounds_mm: contours
            .bounds_mm()
            .ok_or_else(|| failure(ErrorKind::Invalid, None, "Planar bounds are missing"))?,
        area_mm2: contours.area_mm2(),
        outer_loops: contours.loops.iter().filter(|l| l.outer).count(),
        inner_loops: contours.loops.iter().filter(|l| !l.outer).count(),
        geometry_hash: ContentHash::of(&bytes),
    })
}
