use super::*;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
};
use std::time::{Duration, Instant};

fn box_session() -> (Session, PrimitiveRequest) {
    let mut session = Session::new(Document::new("Native backend").unwrap()).unwrap();
    let request = PrimitiveRequest::new(
        "Box",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::CommandLine), &request)
            .unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, request)
}
#[test]
fn native_reusable_assembly_publishes_one_root_and_exact_explodable_leaf_receipts() {
    use std::collections::BTreeMap;
    let (mut session, stock) = box_session();
    let actor = || Actor::local(Channel::CommandLine);
    let mut members = vec![];
    for x in [0., 20.] {
        let request = model::ComponentPlacementRequest::new(
            stock.body,
            model::NumericTransform {
                translation: [length(x), length(0.), length(0.)],
                ..Default::default()
            },
        );
        session
            .commit(
                model::prepare_place_component(&session, session.revision(), actor(), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        members.push(request.instance);
    }
    let assembly = model::AssemblyRequest::new("Reusable pair", members, vec![stock.body]);
    session
        .commit(
            model::prepare_assembly(&session, session.revision(), actor(), &assembly)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let placed = model::ComponentPlacementRequest::new(
        assembly.assembly,
        model::NumericTransform {
            translation: [length(100.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_place_component(&session, session.revision(), actor(), &placed)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let original = session.document().clone();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = scene(session.document(), native.snapshot(), 7).unwrap();
    assert!(scene.transfer_evaluation().unwrap().counts.is_empty());
    assert_eq!(scene.instances().len(), 1);
    assert_eq!(scene.instances()[0].id, placed.instance);
    let info = scene.assembly_evaluation().unwrap();
    assert_eq!(info.leaves.len(), 2);
    assert!(info.leaves.iter().all(|leaf| leaf.component == stock.body
        && leaf.path.root == placed.instance
        && leaf.path.members.len() == 1));
    let mut x: Vec<_> = info
        .leaves
        .iter()
        .map(|leaf| leaf.pose.translation_mm[0])
        .collect();
    x.sort_by(f64::total_cmp);
    assert_eq!(x, vec![100., 120.]);
    assert_eq!(
        scene.instance_bounds(placed.instance).unwrap(),
        [[100., 0., 0.], [130., 20., 30.]]
    );
    let hit = scene
        .pick_face(viewport::Ray::new([105., 5., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    let intent = hit.intent.unwrap();
    assert!(matches!(
        multicad_core::references::resolve(
            &intent,
            &native.snapshot().occurrences[&placed.instance].topology,
            session.revision()
        ),
        multicad_core::references::Resolution::Resolved { .. }
    ));
    let offsets = info
        .leaves
        .iter()
        .enumerate()
        .map(|(index, leaf)| (leaf.path.clone(), [0., index as f64 * 50., 0.]))
        .collect();
    let exploded = scene
        .presentation_leaf_offsets(
            &offsets,
            &BTreeMap::from([(stock.body, [45, 75, 105, 255])]),
        )
        .unwrap();
    assert_eq!(exploded.instances().len(), 2);
    assert!(
        exploded
            .instances()
            .iter()
            .all(|instance| session.document().get(instance.id).is_none()
                && instance.color == [45, 75, 105, 255])
    );
    assert!(!exploded.is_full_model());
    assert!(exploded.assembly_evaluation().is_none());
    assert!(exploded.transfer_evaluation().is_none());
    assert!(exploded.model_evaluation().is_none());
    assert_eq!(scene.instances().len(), 1);
    assert_eq!(session.document(), &original);
}
fn receive(backend: &mut Backend) -> SceneReply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = backend.poll() {
            return reply;
        }
        assert!(Instant::now() < deadline, "Native backend did not publish");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn native_backend_publishes_only_latest_exact_candidate_at_the_same_base_revision() {
    let (session, request) = box_session();
    let mut backend = Backend::new();
    let first = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 17,
    };
    backend.request(session.document(), first).unwrap();
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        request.feature,
        &PrimitiveParameters::Box {
            size: [length(40.), length(20.), length(30.)],
            frame: Default::default(),
        },
    )
    .unwrap();
    let latest = SceneStamp {
        generation: 18,
        ..first
    };
    backend.request(edit.prepared.document(), latest).unwrap();
    let reply = receive(&mut backend);
    assert_eq!(reply.stamp, latest);
    let scene = reply.result.unwrap();
    assert_eq!(scene.stamp(), latest);
    assert_eq!(scene.reference_instances().len(), 1);
    assert_eq!(scene.reference_instances()[0].id, request.body);
    assert_eq!(scene.reference_instances()[0].geometry, request.feature);
    let [min, max] = scene.world_bounds().unwrap();
    for (actual, expected) in min
        .into_iter()
        .zip([0.; 3])
        .chain(max.into_iter().zip([40., 20., 30.]))
    {
        assert!((actual - expected).abs() < 1e-5);
    }
    assert!(backend.poll().is_none());
    // Requesting a candidate did not commit or replace its canonical source.
    assert_eq!(session.revision(), 1);
    assert_eq!(
        model::read_primitive(session.document(), request.feature)
            .unwrap()
            .parameters,
        request.parameters
    );
}
#[test]
fn foreign_scene_scope_is_rejected_without_relabeling_the_document() {
    let (session, _) = box_session();
    let foreign = Document::new("Foreign").unwrap();
    let mut backend = Backend::new();
    assert!(
        backend
            .request(
                session.document(),
                SceneStamp {
                    document: foreign.id(),
                    revision: 0,
                    generation: 1
                }
            )
            .is_err()
    );
    assert!(backend.poll().is_none());
}

#[test]
fn native_backend_receipts_bind_exact_queries_to_source_and_latest_options() {
    use multicad_core::{
        document::EntityId,
        model::{CutTargetProbe, CutTargetQuery, PrimitiveFrame},
        project_file::ContentHash,
    };
    let (mut session, target) = box_session();
    let tool = PrimitiveRequest::new(
        "Cut tool",
        PrimitiveParameters::Box {
            size: [length(2.), length(2.), length(40.)],
            frame: PrimitiveFrame {
                origin: [length(4.), length(4.), length(-5.)],
                ..Default::default()
            },
        },
    );
    let prepared = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        &tool,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 41,
    };
    let probe = EntityId::new();
    let mut options = EvaluationOptions {
        cut_targets: vec![CutTargetProbe {
            id: probe,
            query: CutTargetQuery {
                tools: vec![tool.feature],
                component: None,
                include: vec![],
                exclude: vec![],
            },
        }],
    };
    let mut backend = Backend::new();
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            stamp,
            &options,
        )
        .unwrap();
    // Options can change without changing the source or even the public stamp.
    // The backend's private job identity must still suppress the older receipt.
    options.cut_targets[0].query.exclude.push(target.body);
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            stamp,
            &options,
        )
        .unwrap();
    let reply = receive(&mut backend);
    assert_eq!(reply.stamp, stamp);
    let scene = reply.result.unwrap();
    let info = scene.model_evaluation().unwrap();
    assert_eq!(
        info.source_hash,
        ContentHash::document(session.document()).unwrap()
    );
    assert_eq!(info.cut_targets.len(), 1);
    assert_eq!(info.cut_targets[0].id, probe);
    let selection = info.cut_targets[0].result.as_ref().unwrap();
    assert_eq!(
        selection.defaults,
        vec![model::BodyInput {
            body: target.body,
            producer: target.feature
        }]
    );
    assert!(selection.targets.is_empty());
    assert_eq!(scene.reference_instances().len(), 2);
    assert!(backend.poll().is_none());

    options.cut_targets[0].query.exclude.clear();
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            SceneStamp {
                generation: 42,
                ..stamp
            },
            &options,
        )
        .unwrap();
    let scene = receive(&mut backend).result.unwrap();
    assert_eq!(
        scene.model_evaluation().unwrap().cut_targets[0]
            .result
            .as_ref()
            .unwrap()
            .targets
            .len(),
        1
    );
    assert_eq!(session.revision(), 2);
}

#[test]
fn failed_native_probe_keeps_valid_scene_and_diagnostic_identity() {
    use multicad_core::{
        document::EntityId,
        model::{CutTargetProbe, CutTargetQuery},
    };
    let (session, _) = box_session();
    let missing = EntityId::new();
    let probe = EntityId::new();
    let options = EvaluationOptions {
        cut_targets: vec![CutTargetProbe {
            id: probe,
            query: CutTargetQuery {
                tools: vec![missing],
                component: None,
                include: vec![],
                exclude: vec![],
            },
        }],
    };
    let mut backend = Backend::new();
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            SceneStamp {
                document: session.document().id(),
                revision: session.revision(),
                generation: 1,
            },
            &options,
        )
        .unwrap();
    let scene = receive(&mut backend).result.unwrap();
    assert_eq!(scene.reference_instances().len(), 1);
    let result = &scene.model_evaluation().unwrap().cut_targets[0];
    assert_eq!(result.id, probe);
    assert!(result.result.is_err());
}

#[test]
fn production_scene_retains_consumed_tool_history_with_source_bound_metadata() {
    use multicad_core::{
        model::{BodyInput, BooleanRequest, Operation, PrimitiveFrame},
        project_file::ContentHash,
    };
    let (mut session, target) = box_session();
    let tool = PrimitiveRequest::new(
        "Tool",
        PrimitiveParameters::Box {
            size: [length(2.), length(2.), length(40.)],
            frame: PrimitiveFrame {
                origin: [length(4.), length(4.), length(-5.)],
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        &tool,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: target.body,
            producer: target.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    cut.keep_tools = false;
    let p = model::prepare_boolean(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        &cut,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut backend = Backend::new();
    backend
        .request(
            session.document(),
            SceneStamp {
                document: session.document().id(),
                revision: session.revision(),
                generation: 9,
            },
        )
        .unwrap();
    let scene = receive(&mut backend).result.unwrap();
    let info = scene.model_evaluation().unwrap();
    assert_eq!(
        info.source_hash,
        ContentHash::document(session.document()).unwrap()
    );
    assert_eq!(info.consumed_bodies, [tool.body].into());
    assert!(info.cut_targets.is_empty());
    assert_eq!(scene.reference_instances().len(), 1);
    assert_eq!(scene.reference_instances()[0].id, target.body);
    assert_eq!(scene.reference_instances()[0].geometry, cut.feature);
    assert!(scene.geometry_snapshot(tool.feature).is_some());
    assert!(scene.geometry_snapshot(target.feature).is_some());
    let alias = scene
        .topology_snapshot(tool.body)
        .expect("Consumed definitions stay referenceable without becoming physical occurrences");
    assert_eq!(alias.owner, tool.body);
    assert!(scene.reference_snapshots().contains_key(&tool.body));
    let isolated = scene.isolated_source(tool.body).unwrap();
    assert_eq!(isolated.instances().len(), 1);
    assert_eq!(isolated.instances()[0].id, tool.body);
    assert_eq!(isolated.instances()[0].geometry, tool.feature);
    let hidden = scene.visible_subset(&Default::default()).unwrap();
    assert!(hidden.instances().is_empty());
    assert!(hidden.topology_snapshot(tool.body).is_some());
}

#[test]
fn native_scene_uses_reusable_appearance_without_changing_exact_stock_geometry() {
    use multicad_core::{
        document::{Entity, EntityData, EntityId, Mutation},
        materials::*,
        project_file::{self, ProjectFile},
    };
    let (mut session, request) = box_session();
    let material = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: Some(7850.),
            color_rgba: [50, 60, 70, 255],
            section_hatch: None,
        },
    );
    let appearance = LibraryDefinition::Appearance(Appearance {
        color_rgba: [170, 80, 30, 255],
        roughness: 0.4,
        metallic: 0.8,
    })
    .entity(EntityId::new(), "Painted steel")
    .unwrap();
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        pattern: HatchPattern::Cross,
        ..Default::default()
    })
    .entity(EntityId::new(), "Steel section")
    .unwrap();
    let p = prepare_library(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &[material.clone(), appearance.clone(), hatch.clone()],
    )
    .unwrap();
    session.commit(p).unwrap();
    let assignment = Assignment {
        material: Some(material.id),
        stock: None,
        appearance: Some(appearance.id),
        hatch: Some(hatch.id),
    };
    let p = prepare_assignment(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &[request.body],
        &assignment,
    )
    .unwrap();
    session.commit(p).unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let document = opened.project.document;
    let model = Compiler::default()
        .compile(&document, session.revision())
        .unwrap();
    let solid = &model.snapshot().shapes[&request.feature];
    assert!((solid.volume_mm3 - 6000.).abs() < 1e-6);
    assert_eq!(solid.solid_count, 1);
    let current = scene(&document, model.snapshot(), 10).unwrap();
    assert_eq!(current.reference_instances()[0].color, [170, 80, 30, 255]);
    assert_eq!(
        resolve(&document, request.body)
            .unwrap()
            .hatch
            .unwrap()
            .pattern,
        HatchPattern::Cross
    );
    let broken = document.edited(&[Mutation::Remove(appearance.id)]).unwrap();
    let native = Compiler::default()
        .compile(&broken, session.revision() + 1)
        .unwrap();
    assert!((native.snapshot().shapes[&request.feature].volume_mm3 - 6000.).abs() < 1e-6);
    let error = scene(&broken, native.snapshot(), 11)
        .err()
        .expect("broken appearance must fail publication");
    assert!(error.message.contains("Missing appearance"));
    // No replacement color or geometry is fabricated for a broken assignment.
    assert_eq!(current.reference_instances()[0].color, [170, 80, 30, 255]);
}

#[test]
fn native_backend_places_shared_meshes_and_picks_current_world_topology() {
    use multicad_core::model::{
        BodyInput, InstanceCopyRequest, InstanceCopySource, NumericTransform, angle,
    };
    let (mut session, part) = box_session();
    let copy = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        },
        NumericTransform {
            translation: [length(100.), length(0.), length(0.)],
            angle: angle(90.),
            ..Default::default()
        },
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        std::slice::from_ref(&copy),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let mut backend = Backend::new();
    let stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 22,
    };
    backend.request(session.document(), stamp).unwrap();
    let scene = receive(&mut backend).result.unwrap();
    assert_eq!(scene.geometries().len(), 1);
    assert_eq!(scene.instances().len(), 2);
    assert!(
        scene
            .instances()
            .iter()
            .all(|instance| instance.geometry == part.feature)
    );
    let hit = scene
        .pick_face(viewport::Ray::new([90., 5., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(hit.instance, copy.instance);
    assert_eq!(hit.geometry, part.feature);
    assert_eq!(hit.point_mm, [90., 5., 30.]);
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let actual = scene.topology_snapshot(copy.instance).unwrap();
    let expected = &native.snapshot().occurrences[&copy.instance].topology;
    assert_eq!(
        actual
            .elements
            .iter()
            .map(|e| e.geometry)
            .collect::<Vec<_>>(),
        expected
            .elements
            .iter()
            .map(|e| e.geometry)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        scene.model_evaluation().unwrap().source_hash,
        native.snapshot().stamp.source_hash
    );
}

#[test]
fn native_source_inspection_is_explicitly_partial_and_does_not_relabel_a_full_request() {
    let (mut session, source) = box_session();
    let other = PrimitiveRequest::new(
        "Independent body",
        PrimitiveParameters::Box {
            size: std::array::from_fn(|_| length(5.)),
            frame: model::PrimitiveFrame {
                origin: [length(100.), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    let edit = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &other,
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let mut backend = Backend::new();
    let stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 91,
    };
    backend
        .request_sources(
            session.document(),
            &Default::default(),
            stamp,
            &[source.feature],
        )
        .unwrap();
    let scene = receive(&mut backend).result.unwrap();
    assert_eq!(
        scene.scope(),
        &viewport::SceneScope::Sources(vec![source.feature])
    );
    assert!(!scene.is_full_model());
    assert_eq!(
        scene.model_evaluation().unwrap().source_hash,
        multicad_core::project_file::ContentHash::document(session.document()).unwrap()
    );
    assert!(scene.geometries().contains_key(&source.feature));
    assert!(!scene.geometries().contains_key(&other.feature));
    assert!(
        scene
            .reference_instances()
            .iter()
            .all(|instance| instance.geometry == source.feature)
    );
    assert!(scene.world_bounds().unwrap()[1][0] < 11.);
    let full_stamp = SceneStamp {
        generation: 92,
        ..stamp
    };
    backend.request(session.document(), full_stamp).unwrap();
    let full = receive(&mut backend).result.unwrap();
    assert!(full.is_full_model());
    assert_eq!(full.reference_instances().len(), 2);
    assert!(full.world_bounds().unwrap()[1][0] > 104.);
}

#[test]
fn layered_wall_scene_publishes_each_live_source_material_without_flattening_the_part() {
    use model::{
        BodyOutput, LayeredWallTypeDefinition, NewType, NewTypeKind, WallChange, WallDefinition,
        WallLayer, WallLayerKind, WallLayerReference, WallLayerStack, WallRequest,
    };
    use multicad_core::document::{Entity, EntityData, EntityId, Mutation};
    let material = |name, color| {
        Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: color,
                section_hatch: None,
            },
        )
    };
    let brick = material("Brick", [160, 80, 60, 255]);
    let lining = material("Lining", [230, 235, 240, 255]);
    let doc = Document::new("Layer colors")
        .unwrap()
        .edited(&[
            Mutation::Insert(brick.clone()),
            Mutation::Insert(lining.clone()),
        ])
        .unwrap();
    let mut session = Session::new(doc).unwrap();
    let actor = || Actor::local(Channel::CommandLine);
    let ty = NewType::new(
        "Construction",
        NewTypeKind::LayeredWall(LayeredWallTypeDefinition {
            layers: WallLayerStack {
                layers: [(brick.id, 100.), (lining.id, 50.)]
                    .into_iter()
                    .map(|(material, width)| WallLayer {
                        id: EntityId::new(),
                        name: "Layer".into(),
                        role: None,
                        construction: WallLayerKind::Solid {
                            material,
                            thickness: length(width),
                        },
                    })
                    .collect(),
            },
            height: length(2500.),
            reference: WallLayerReference::Center,
        }),
    );
    let wall = WallRequest::new(
        "Layered wall",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty.id,
            layer_placement: Some(Default::default()),
            path: vec![[length(0.), length(0.)], [length(1000.), length(0.)]],
            closed: false,
            level: None,
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    );
    let p = model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&wall))
        .unwrap();
    session.commit(p.prepared).unwrap();
    let mut cache = multicad_model::EvaluationCache::default();
    for (generation, color) in [(1, [160, 80, 60, 255]), (2, [40, 100, 180, 255])] {
        if generation == 2 {
            let mut changed = brick.clone();
            if let EntityData::Material { color_rgba, .. } = &mut changed.data {
                *color_rgba = color;
            }
            session
                .apply(multicad_core::commands::Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Material appearance".into(),
                    edits: vec![Mutation::Replace(changed)],
                })
                .unwrap();
        }
        let model = Compiler::default()
            .compile_cached(&mut cache, session.document(), session.revision(), None)
            .unwrap();
        let scene = scene(session.document(), model.snapshot(), generation).unwrap();
        let geometry = &scene.geometries()[&wall.feature];
        let materials = model::wall_face_materials(
            session.document(),
            &model.snapshot().shapes[&wall.feature].topology,
        )
        .unwrap();
        assert_eq!(scene.reference_instances().len(), 1);
        for (face, assignment) in materials {
            assert_eq!(
                geometry.face_color(face),
                Some(if assignment.material == brick.id {
                    color
                } else {
                    [230, 235, 240, 255]
                })
            );
        }
        let colors: std::collections::BTreeSet<_> = geometry
            .triangle_faces()
            .iter()
            .filter_map(|face| geometry.face_color(*face))
            .collect();
        assert_eq!(colors.len(), 2);
    }
}
