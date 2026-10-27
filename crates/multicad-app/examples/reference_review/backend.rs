//! CAD-55 native acceptance fixture. This is deliberately not a production
//! feature provider: only the small review schema below is supported.
use multicad_core::{
    constraint_sketch::{self, source::Source},
    document::*,
    parameters::{self, Unit},
    references::*,
};
use multicad_kernel::*;
use multicad_ui::viewport::{self, SceneBackend, SceneReply, SceneStamp};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
};
pub const PANEL: &str = "reference.review.panel";
pub const CUTTER: &str = "reference.review.cutter";
pub const HOLE: &str = "reference.review.hole";
fn feature(name: &str, type_id: &str, inputs: Vec<Link>) -> Entity {
    Entity::new(
        name,
        EntityData::Feature {
            definition: Definition {
                type_id: type_id.into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs,
            suppressed: false,
        },
    )
}
fn origin(source: EntityId, role: &str) -> Origin {
    Origin {
        source,
        role: role.into(),
    }
}
pub fn document() -> Result<Document, String> {
    let parameter = |name: &str, expression: &str, unit: &str, description: &str| {
        let mut entity = Entity::new(
            name,
            EntityData::Parameter {
                expression: expression.into(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        );
        entity.description = description.into();
        entity
    };
    let width = parameter(
        "panel_width",
        "100",
        "mm",
        "Panel width; dimensional changes preserve the top-face reference.",
    );
    let height = parameter(
        "panel_height",
        "18",
        "mm",
        "Panel thickness; the face-mounted drilling follows this height.",
    );
    let mode = parameter(
        "topology_mode",
        "0",
        "ratio",
        "0 = intact; 1 = through-slot splits top; 2 = shave removes original top.",
    );
    let cutter = feature("Reference review cutting tool", CUTTER, vec![]);
    let panel = feature(
        "Upstream panel",
        PANEL,
        vec![
            Link::new(width.id, "width"),
            Link::new(height.id, "height"),
            Link::new(mode.id, "topology_mode"),
            Link::new(cutter.id, "cutter"),
        ],
    );
    let initial = Solid::box_named([100., 60., 18.], panel.id, Default::default())
        .map_err(|e| e.to_string())?
        .topology_snapshot(panel.id, 0)
        .map_err(|e| e.to_string())?;
    let top = initial
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face && e.origins.contains(&origin(panel.id, "cap.end"))
        })
        .ok_or("Native panel has no top cap")?;
    let mut intent = GeometryIntent::capture(&initial, top.key)?;
    intent.attachment = Some(Attachment {
        anchor: origin(panel.id, "cap.end"),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let mut anchor = Link::new(panel.id, "face");
    anchor.geometry = Some(intent);
    let plane = Entity::new(
        "Drilling plane on panel top",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(anchor),
        },
    );
    let mut source = Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    };
    let center = source.add_point([20., 25.]);
    source.sketch.curves.push(constraint_sketch::Curve {
        id: EntityId::new(),
        construction: false,
        kind: constraint_sketch::CurveKind::Circle {
            center,
            radius_mm: 3.,
        },
    });
    let sketch = Entity::new(
        "Face-mounted drill circle",
        EntityData::Sketch {
            plane: plane.id,
            definition: source.definition().map_err(|e| e.to_string())?,
            inputs: vec![],
        },
    );
    let hole = feature(
        "Dependent through drilling",
        HOLE,
        vec![Link::new(panel.id, "base"), Link::new(sketch.id, "profile")],
    );
    let material = Entity::new(
        "Review panel material",
        EntityData::Material {
            density_kg_m3: Some(700.),
            color_rgba: [137, 174, 199, 255],
            section_hatch: None,
        },
    );
    let part = Entity::new(
        "Drilled panel",
        EntityData::Part {
            features: vec![hole.id],
            material: Some(material.id),
            tags: BTreeSet::from(["reference-review".into()]),
        },
    );
    let instance = Entity::new(
        "Panel instance",
        EntityData::Instance {
            component: part.id,
            placement: Default::default(),
        },
    );
    let mut document = Document::new("Reference repair review").map_err(|e| e.to_string())?;
    for entity in [
        width, height, mode, cutter, panel, plane, sketch, hole, material, part, instance,
    ] {
        document.insert(entity).map_err(|e| e.to_string())?;
    }
    Ok(document)
}
fn named_feature<'a>(document: &'a Document, type_id: &str) -> Result<&'a Entity, String> {
    let mut matches=document.entities().filter(|e|matches!(&e.data,EntityData::Feature {definition,suppressed:false,..} if definition.type_id==type_id&&definition.schema_version==1));
    let entity = matches
        .next()
        .ok_or_else(|| format!("Missing review feature {type_id}"))?;
    if matches.next().is_some() {
        return Err(format!("Duplicate review feature {type_id}"));
    }
    Ok(entity)
}
fn geometry(solid: &Solid, id: EntityId, revision: u64) -> Result<Arc<viewport::Geometry>, String> {
    let mesh = solid
        .mesh(MeshOptions::default())
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(
        viewport::Geometry::new(
            id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            solid
                .topology_snapshot(id, revision)
                .map_err(|e| e.to_string())?
                .elements,
        )?
        .with_topology_display(
            solid
                .display_edges(0.1)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|e| viewport::DisplayEdge {
                    index: e.index,
                    points_mm: e.points_mm,
                })
                .collect(),
            solid
                .display_vertices()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|v| viewport::DisplayPoint {
                    index: v.index,
                    position_mm: v.position_mm,
                })
                .collect(),
            0.1,
        )?,
    ))
}
pub struct Evaluation {
    pub scene: Arc<viewport::Scene>,
    pub report: ReferenceReport,
    pub source_volume_mm3: f64,
    pub display_volume_mm3: f64,
    /// Authoring geometry coordinates before instance placement. Placed world
    /// attachment frames are available through the Scene topology snapshot.
    pub hole_center_world: Option<[f64; 3]>,
}
/// Native B-reps are created and dropped on the calling worker thread. Only
/// immutable copied display geometry crosses back to the UI.
pub fn evaluate(
    document: &Document,
    stamp: SceneStamp,
    check: impl Fn() -> Result<(), String>,
) -> Result<Evaluation, String> {
    if stamp.document != document.id() {
        return Err("Review stamp belongs to another document".into());
    }
    check()?;
    let values = parameters::evaluate(document).map_err(|e| e.to_string())?;
    let panel = named_feature(document, PANEL)?;
    let cutter = named_feature(document, CUTTER)?;
    let hole = named_feature(document, HOLE)?;
    for feature in document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Feature { .. }))
    {
        if ![panel.id, cutter.id, hole.id].contains(&feature.id) {
            return Err("Review backend does not implement arbitrary modeling features".into());
        }
    }
    let width = values
        .feature_input(panel.id, "width", Unit::Millimeter)
        .map_err(|e| e.to_string())?;
    let height = values
        .feature_input(panel.id, "height", Unit::Millimeter)
        .map_err(|e| e.to_string())?;
    let mode = values
        .feature_input(panel.id, "topology_mode", Unit::Ratio)
        .map_err(|e| e.to_string())?;
    if !(60.0..=200.0).contains(&width)
        || !(4.0..=80.0).contains(&height)
        || ![0., 1., 2.].contains(&mode)
    {
        return Err("Review dimensions require width 60..200 mm, height 4..80 mm and topology_mode 0, 1 or 2".into());
    }
    let policy = GeometryPolicy::default();
    let base =
        Solid::box_named([width, 60., height], panel.id, policy).map_err(|e| e.to_string())?;
    check()?;
    let source = if mode == 0. {
        base
    } else {
        let (size, translation_mm) = if mode == 1. {
            ([10., 80., height + 10.], [width * 0.5 - 5., -10., -5.])
        } else {
            ([width + 20., 80., height], [-10., -10., height * 0.5])
        };
        let tool = Solid::box_named(size, cutter.id, policy)
            .map_err(|e| e.to_string())?
            .transformed(RigidTransform {
                translation_mm,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        base.boolean(&tool, BooleanOperation::Subtract, policy)
            .map_err(|e| e.to_string())?
            .ok_or("Review cutter removed all material")?
    };
    check()?;
    let snapshot = source
        .topology_snapshot(panel.id, stamp.revision)
        .map_err(|e| e.to_string())?;
    let report = inspect(
        document,
        stamp.revision,
        &BTreeMap::from([(panel.id, snapshot)]),
    );
    let mut geometries = vec![geometry(&source, panel.id, stamp.revision)?];
    let source_volume_mm3 = source.volume_mm3().map_err(|e| e.to_string())?;
    let mut display_volume_mm3 = source_volume_mm3;
    let mut hole_center_world = None;
    let display_id = if report.all_resolved() {
        let profile_link = hole
            .references()
            .into_iter()
            .find(|l| l.role == "profile")
            .ok_or("Hole has no sketch profile")?;
        let (sketch, frame, _) =
            Source::read_evaluated(document, profile_link.target, stamp.revision, &report)
                .map_err(|e| e.to_string())?;
        let curves = sketch.display_geometry().map_err(|e| e.to_string())?;
        let [
            multicad_core::sketch::Geometry::Circle {
                id,
                center,
                radius_mm,
            },
        ] = curves.as_slice()
        else {
            return Err("Review hole requires its one authored circle".into());
        };
        hole_center_world = Some(frame.point(*center));
        let profile = Profile {
            frame: PlaneFrame {
                origin: frame.origin,
                u: frame.u,
                v: frame.v,
            },
            outer: vec![ProfileEdge::Circle {
                center: *center,
                radius_mm: *radius_mm,
            }],
            holes: vec![],
        };
        let tool = Solid::extrude_profile_named(
            &profile,
            -height,
            hole.id,
            &[origin(*id, "circle")],
            policy,
        )
        .map_err(|e| e.to_string())?;
        check()?;
        let drilled = source
            .boolean(&tool, BooleanOperation::Subtract, policy)
            .map_err(|e| e.to_string())?
            .ok_or("Drill removed all material")?;
        display_volume_mm3 = drilled.volume_mm3().map_err(|e| e.to_string())?;
        geometries.push(geometry(&drilled, hole.id, stamp.revision)?);
        hole.id
    } else {
        panel.id
    };
    check()?;
    let instance = document
        .entities()
        .find(|e| matches!(e.data, EntityData::Instance { .. }))
        .ok_or("Missing review instance")?;
    let EntityData::Instance { placement, .. } = instance.data else {
        unreachable!("filtered instance")
    };
    let scene = Arc::new(viewport::Scene::new(
        stamp,
        geometries,
        vec![viewport::Instance {
            id: instance.id,
            geometry: display_id,
            transform: viewport::Transform::from_placement(placement)?,
            color: [137, 174, 199, 255],
        }],
    )?);
    Ok(Evaluation {
        scene,
        report,
        source_volume_mm3,
        display_volume_mm3,
        hole_center_world,
    })
}
struct Job {
    document: Document,
    stamp: SceneStamp,
}
struct Worker {
    sender: SyncSender<Job>,
    reply: Arc<Mutex<Option<SceneReply>>>,
    generation: Arc<Mutex<u64>>,
}
pub struct Backend {
    worker: Result<Worker, String>,
}
impl Backend {
    pub fn new() -> Self {
        let spawn = || {
            let (sender, receive) = mpsc::sync_channel::<Job>(1);
            let reply = Arc::new(Mutex::new(None));
            let published = reply.clone();
            let generation = Arc::new(Mutex::new(0));
            let current = generation.clone();
            std::thread::Builder::new()
                .name("reference-review-native".into())
                .spawn(move || {
                    while let Ok(job) = receive.recv() {
                        let check = || {
                            if current.lock().is_ok_and(|g| *g == job.stamp.generation) {
                                Ok(())
                            } else {
                                Err("Reference review build superseded".into())
                            }
                        };
                        let result = evaluate(&job.document, job.stamp, check).map(|e| e.scene);
                        if check().is_ok()
                            && let Ok(mut slot) = published.lock()
                        {
                            // Only one unconsumed display receipt is retained.
                            *slot = Some(SceneReply {
                                stamp: job.stamp,
                                result,
                            });
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(Worker {
                sender,
                reply,
                generation,
            })
        };
        Self { worker: spawn() }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if let Ok(worker) = &self.worker
            && let Ok(mut generation) = worker.generation.lock()
        {
            *generation = generation.wrapping_add(1);
        }
    }
}
impl SceneBackend for Backend {
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        let worker = self.worker.as_ref().map_err(Clone::clone)?;
        // Hold the publication lock while enqueueing: the worker must not see
        // the new request before its generation has been installed.
        let mut generation = worker
            .generation
            .lock()
            .map_err(|_| "Review generation lock poisoned")?;
        // A single queued snapshot bounds memory; supersession is checked around
        // native operations. Publication still checks the full SceneStamp.
        worker
            .sender
            .try_send(Job {
                document: document.clone(),
                stamp,
            })
            .map_err(|_| "Reference review worker is busy; retry after polling".to_owned())?;
        *generation = stamp.generation;
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        self.worker.as_ref().ok()?.reply.lock().ok()?.take()
    }
}
