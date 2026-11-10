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
fn rust_thread_tool_previews_real_m8_ridges_and_cancel_keeps_the_document() {
    use multicad_ui::modeling::ModelToolController;

    let mut session = Session::new(Document::new("Rust thread tool").unwrap()).unwrap();
    let resources = ProjectResources::default();
    let before = session.document().clone();
    let mut tool = ModelToolController::default();
    tool.set_backend(Backend::factory(GeometryBackend::Rust).unwrap()());
    for cancel in [true, false] {
        tool.begin_thread(&session, &resources, None, &Default::default(), Channel::Ui)
            .unwrap();
        assert!(!tool.can_apply(&session, &resources));
        tool.preview(&session, &resources, Channel::Ui).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while tool.pending() {
            tool.poll(&session, &resources);
            assert!(Instant::now() < deadline, "Rust thread preview timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(tool.can_apply(&session, &resources), "{:?}", tool.error());
        let scene = tool.display_scene(&session, &resources).unwrap();
        assert_eq!(scene.instances().len(), 1);
        let hit = |z| {
            scene
                .pick_face(viewport::Ray::new([10., 0., z], [-1., 0., 0.]).unwrap())
                .unwrap()
        };
        let crest = hit(2.5);
        let root = hit(3.125);
        assert!(crest.point_mm[0] > 3.9, "{:?}", crest.point_mm);
        assert!(root.point_mm[0] < 3.6, "{:?}", root.point_mm);
        assert_ne!(crest.key, root.key);
        // A right-hand turn advances the crest by one quarter pitch from +X to +Y.
        let y_hit = |z| {
            scene
                .pick_face(viewport::Ray::new([0., 10., z], [0., -1., 0.]).unwrap())
                .unwrap()
                .point_mm[1]
        };
        assert!(y_hit(2.5 + 1.25 / 4.) > 3.9);
        assert!(y_hit(2.5 - 1.25 / 4.) < 3.6);
        assert_eq!(session.document(), &before);
        if cancel {
            tool.cancel();
            assert!(tool.display_scene(&session, &resources).is_none());
            assert_eq!(session.revision(), 0);
        } else {
            let outputs = tool.apply(&mut session, &resources).unwrap();
            assert_eq!(outputs.len(), 1);
            assert_eq!(session.revision(), 1);
            let source = model::read_thread(session.document(), outputs[0].feature).unwrap();
            assert_eq!(source.output.body, outputs[0].body);
            assert_eq!(source.parameters, model::ThreadParameters::default());
        }
    }
}

#[test]
fn rust_existing_internal_thread_previews_the_open_bore_and_its_helical_wall() {
    use multicad_core::model::{BodyInput, ThreadFit, ThreadMode, ThreadParameters, ThreadRequest};
    use multicad_ui::modeling::ModelToolController;

    let mut session = Session::new(Document::new("Rust threaded receiver").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Receiver",
        PrimitiveParameters::Cylinder {
            radius: length(5.),
            height: length(6.6),
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let cut = ThreadRequest::receiving(
        "M6 × 1 6H",
        ThreadMode::CutInternal,
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        ThreadParameters {
            major_diameter: length(6.),
            pitch: length(1.),
            length: length(6.6),
            minimum_wall: length(1.),
            fit: ThreadFit::Metric6HInternal,
            ..Default::default()
        },
    )
    .unwrap();
    let prepared = model::prepare_thread(&session, 1, Actor::local(Channel::Ui), &cut).unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    let resources = ProjectResources::default();
    let mut tool = ModelToolController::default();
    tool.set_backend(Backend::factory(GeometryBackend::Rust).unwrap()());
    tool.begin_existing(&session, &resources, cut.feature, Channel::Ui)
        .unwrap();
    tool.preview(&session, &resources, Channel::Ui).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while tool.pending() {
        tool.poll(&session, &resources);
        assert!(
            Instant::now() < deadline,
            "Rust threaded receiver preview timed out"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(tool.can_apply(&session, &resources), "{:?}", tool.error());
    let scene = tool.display_scene(&session, &resources).unwrap();
    assert_eq!(scene.instances().len(), 1);
    let pick = |origin, direction| scene.pick_face(viewport::Ray::new(origin, direction).unwrap());
    assert!(pick([0., 0., 10.], [0., 0., -1.]).is_none());
    let cap = pick([4., 0., 10.], [0., 0., -1.]).unwrap();
    assert!((cap.point_mm[2] - 6.6).abs() < 1e-6);
    let deep = pick([0., 0., 2.], [1., 0., 0.]).unwrap();
    let shallow = pick([0., 0., 2.5], [1., 0., 0.]).unwrap();
    assert!(deep.point_mm[0] > 2.95, "{:?}", deep.point_mm);
    assert!(shallow.point_mm[0] < 2.6, "{:?}", shallow.point_mm);
    assert_ne!(deep.key, shallow.key);
    tool.cancel();
    assert_eq!(session.revision(), 2);
    assert_eq!(session.document(), &before);
}

#[test]
fn native_reusable_assembly_publishes_one_root_and_exact_explodable_leaf_receipts() {
    reusable_assembly_scene(GeometryBackend::OpenCascade);
}

#[test]
fn rust_reusable_assembly_publishes_one_root_and_exact_explodable_leaf_receipts() {
    reusable_assembly_scene(GeometryBackend::Rust);
}

fn reusable_assembly_scene(geometry_backend: GeometryBackend) {
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
    let model = Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            geometry_backend,
            None,
        )
        .unwrap();
    let mut worker = Backend::factory(geometry_backend).unwrap()();
    worker
        .request(
            session.document(),
            SceneStamp {
                document: session.document().id(),
                revision: session.revision(),
                generation: 7,
            },
        )
        .unwrap();
    let scene = wait_for_scene(&mut *worker).result.unwrap();
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
            &model.snapshot().occurrences[&placed.instance].topology,
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

#[test]
fn aggregate_faces_keep_nested_member_material_and_part_override() {
    use multicad_core::{
        document::{Entity, Mutation},
        materials::{Appearance, Assignment, LibraryDefinition, prepare_assignment},
    };
    let (mut session, first) = box_session();
    let actor = || Actor::local(Channel::CommandLine);
    let red = [220, 60, 40, 255];
    let blue = [30, 100, 220, 255];
    let material = Entity::new(
        "Red material",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: red,
            section_hatch: None,
        },
    );
    let override_color = LibraryDefinition::Appearance(Appearance {
        color_rgba: blue,
        ..Default::default()
    })
    .entity(EntityId::new(), "Blue override")
    .unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Create material and appearance".into(),
            edits: vec![
                Mutation::Insert(material.clone()),
                Mutation::Insert(override_color.clone()),
            ],
        })
        .unwrap();
    let second = PrimitiveRequest::new("Blue box", first.parameters.clone());
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor(), &second).unwrap();
    session.commit(prepared.prepared).unwrap();
    for (body, appearance) in [(first.body, None), (second.body, Some(override_color.id))] {
        let prepared = prepare_assignment(
            &session,
            session.revision(),
            actor(),
            &[body],
            &Assignment {
                material: Some(material.id),
                stock: None,
                appearance,
                hatch: None,
            },
        )
        .unwrap();
        session.commit(prepared).unwrap();
    }
    let place = |session: &mut Session, component, x| {
        let request = model::ComponentPlacementRequest::new(
            component,
            model::NumericTransform {
                translation: [length(x), length(0.), length(0.)],
                ..Default::default()
            },
        );
        let prepared =
            model::prepare_place_component(session, session.revision(), actor(), &request).unwrap();
        session.commit(prepared.prepared).unwrap();
        request.instance
    };
    let red_member = place(&mut session, first.body, 0.);
    let blue_member = place(&mut session, second.body, 24.);
    let pair = model::AssemblyRequest::new(
        "Pair",
        vec![red_member, blue_member],
        vec![first.body, second.body],
    );
    let prepared = model::prepare_assembly(&session, session.revision(), actor(), &pair).unwrap();
    session.commit(prepared.prepared).unwrap();
    let pair_member = place(&mut session, pair.assembly, 0.);
    let cabinet = model::AssemblyRequest::new("Nested pair", vec![pair_member], vec![]);
    let prepared =
        model::prepare_assembly(&session, session.revision(), actor(), &cabinet).unwrap();
    session.commit(prepared.prepared).unwrap();
    let root = place(&mut session, cabinet.assembly, 100.);

    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = scene(session.document(), native.snapshot(), 1).unwrap();
    assert_eq!(
        scene
            .instances()
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [root]
    );
    let geometry = &scene.geometries()[&cabinet.assembly];
    let palette: std::collections::BTreeSet<_> = geometry
        .triangle_faces()
        .iter()
        .map(|face| {
            geometry
                .face_color(*face)
                .expect("each compound face has a member color")
        })
        .collect();
    assert_eq!(palette, [red, blue].into_iter().collect());
    for (x, color) in [(105., red), (129., blue)] {
        let hit = scene
            .pick_face(viewport::Ray::new([x, 5., 100.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.instance, root);
        assert_eq!(geometry.face_color(hit.key.ordinal), Some(color));
    }
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
fn native_architecture_plan_exposes_real_opening_gaps_symbols_and_source_bound_picking() {
    use multicad_core::{
        document::EntityId,
        drawings::{VectorCurve, VectorRole},
        model::*,
    };
    let mut session = Session::new(Document::new("Native authoring plan").unwrap()).unwrap();
    let actor = Actor::local(Channel::Ui);
    let wall_type = NewType::new(
        "Wall 200",
        NewTypeKind::Wall(WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: None,
        }),
    );
    let door = NewType::new(
        "Door",
        NewTypeKind::Opening(OpeningTypeDefinition {
            kind: OpeningKind::Door {
                hinge: Hinge::Start,
                swing: Swing::Left,
            },
            width: length(900.),
            height: length(2100.),
            sill: length(0.),
        }),
    );
    let window = NewType::new(
        "Window",
        NewTypeKind::Opening(OpeningTypeDefinition {
            kind: OpeningKind::Window,
            width: length(1200.),
            height: length(1200.),
            sill: length(900.),
        }),
    );
    let request = WallRequest::new(
        "Wall with openings",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: wall_type.id,
            layer_placement: None,
            path: vec![[length(0.), length(0.)], [length(4000.), length(0.)]],
            closed: false,
            level: None,
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            cutouts: vec![],
            openings: vec![
                Opening {
                    id: EntityId::new(),
                    opening_type: door.id,
                    segment: 0,
                    offset: length(500.),
                    width: None,
                    height: None,
                    sill: None,
                },
                Opening {
                    id: EntityId::new(),
                    opening_type: window.id,
                    segment: 0,
                    offset: length(2200.),
                    width: None,
                    height: None,
                    sill: None,
                },
            ],
        },
    );
    let prepared = prepare_wall_change(
        &session,
        session.revision(),
        actor,
        &[wall_type, door, window],
        &WallChange::Create(&request),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    let mut backend = Backend::new();
    let stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 1,
    };
    let probe = EntityId::new();
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            stamp,
            &EvaluationOptions {
                cut_targets: vec![CutTargetProbe {
                    id: probe,
                    query: CutTargetQuery {
                        tools: vec![EntityId::new()],
                        component: None,
                        include: vec![],
                        exclude: vec![],
                    },
                }],
                ..Default::default()
            },
        )
        .unwrap();
    let native = receive(&mut backend).result.unwrap();
    let queries = &native.model_evaluation().unwrap().cut_targets;
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].id, probe);
    assert!(
        queries[0].result.is_err(),
        "plan graphics must preserve per-request probe diagnostics"
    );
    let plan = native
        .architecture_plan()
        .expect("native plan receipt")
        .as_ref()
        .unwrap();
    assert_eq!(
        plan.sources,
        std::collections::BTreeSet::from([request.body])
    );
    assert_eq!(plan.source_hash, session.source_hash().unwrap());
    assert!(
        plan.vectors
            .iter()
            .any(|vector| vector.role == VectorRole::Section)
    );
    assert!(
        plan.vectors
            .iter()
            .any(|vector| vector.role == VectorRole::Hatch)
    );
    assert!(plan.vectors.iter().any(|vector|matches!(vector.curve,VectorCurve::Ellipse {major_mm,..} if (major_mm-900.).abs()<1e-6)));
    assert!(
        plan.vectors
            .iter()
            .any(|vector| matches!(vector.curve,VectorCurve::Line {a,b}
        if (a[0].min(b[0])-2200.).abs()<1e-6 && (a[0].max(b[0])-3400.).abs()<1e-6
            && (a[1].abs()-200./6.).abs()<1e-6 && (a[1]-b[1]).abs()<1e-6))
    );
    for vector in plan
        .vectors
        .iter()
        .filter(|vector| vector.role == VectorRole::Section)
    {
        for segment in vector.preview_mm.windows(2) {
            if (segment[0][1].abs() - 100.).abs() < 1e-6
                && (segment[0][1] - segment[1][1]).abs() < 1e-6
            {
                let (low, high) = (
                    segment[0][0].min(segment[1][0]),
                    segment[0][0].max(segment[1][0]),
                );
                for (a, b) in [(500., 1400.), (2200., 3400.)] {
                    assert!(
                        high.min(b) - low.max(a) < 1e-6,
                        "section incorrectly bridges an opening"
                    );
                }
            }
        }
    }
    let surface = plan.surface_scene(&native).unwrap();
    assert!(surface.instances().is_empty());
    assert_eq!(surface.reference_instances().len(), 1);
    assert!(surface.topology_snapshot(request.body).is_some());
    let hit = native
        .pick_face(viewport::Ray::new([950., 0., 5000.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(
        hit.instance, request.body,
        "the canonical wall remains the authoring target"
    );
    for (id, geometry) in native.geometries() {
        assert!(Arc::ptr_eq(geometry, &surface.geometries()[id]));
    }
    let mut copy = viewport::Scene::new(
        stamp,
        native.geometries().values().cloned().collect(),
        native.instances().to_vec(),
    )
    .unwrap()
    .with_model_evaluation(native.model_evaluation().unwrap().clone())
    .unwrap();
    assert!(
        copy.set_architecture_plan(Ok(viewport::ArchitecturePlan {
            source_hash: multicad_core::project_file::ContentHash::of(b"foreign source"),
            sources: plan.sources.clone(),
            vectors: plan.vectors.clone(),
        }))
        .is_err()
    );
    copy.set_architecture_plan(Err("Projection unavailable".into()))
        .unwrap();
    assert_eq!(
        copy.instances().len(),
        native.instances().len(),
        "optional plan failure preserves 3D geometry"
    );
    let copy = Arc::new(copy);
    let mut context = multicad_ui::architecture_view::Context::default();
    let mut camera = multicad_ui::camera::Camera::default();
    context.sync(session.document(), session.revision());
    context.set_enabled(true, &mut camera);
    context.set_plan(true, &mut camera);
    assert!(
        context
            .scene(&copy)
            .err()
            .unwrap()
            .contains("Projection unavailable")
    );
    context.set_plan(false, &mut camera);
    assert!(Arc::ptr_eq(&context.scene(&copy).unwrap(), &copy));
    assert_eq!(session.document(), &before);
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
        ..Default::default()
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
        ..Default::default()
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
        color_rgb: None,
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

#[test]
fn shared_native_owner_reuses_default_screw_preview_for_authoritative_apply() {
    fn receive_screw(backend: &mut Backend) -> SceneReply {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(reply) = backend.poll() {
                return reply;
            }
            assert!(
                Instant::now() < deadline,
                "Default M3 screw did not publish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let owner = Backend::runtime(GeometryBackend::OpenCascade).unwrap();
    let mut preview = Backend {
        runtime: owner.fork(),
        expected: None,
    };
    let mut canonical = Backend {
        runtime: owner.fork(),
        expected: None,
    };
    let mut session = Session::new(Document::new("Default M3 shared owner").unwrap()).unwrap();
    let resources = Default::default();
    for number in 1..=2 {
        let parameters = model::default_machine_screw();
        let request =
            model::MechanicalPartRequest::new(format!("Default M3 {number}"), parameters.clone());
        let prepared = model::prepare_mechanical_part(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        let original = session.document().clone();
        let stamp = SceneStamp {
            document: original.id(),
            revision: session.revision(),
            generation: number * 10,
        };
        let started = Instant::now();
        preview
            .request_evaluation(
                prepared.prepared.document(),
                &resources,
                stamp,
                &Default::default(),
            )
            .unwrap();
        let checked = receive_screw(&mut preview).result.unwrap();
        let initial_ms = started.elapsed().as_secs_f64() * 1000.;
        assert_eq!(checked.stamp(), stamp);
        assert_eq!(checked.instances().len(), number as usize);
        assert_eq!(session.document(), &original);
        let started = Instant::now();
        let unchanged_stamp = SceneStamp {
            generation: stamp.generation + 1,
            ..stamp
        };
        preview
            .request_evaluation(
                prepared.prepared.document(),
                &resources,
                unchanged_stamp,
                &Default::default(),
            )
            .unwrap();
        let unchanged = receive_screw(&mut preview).result.unwrap();
        let unchanged_ms = started.elapsed().as_secs_f64() * 1000.;
        let stats = preview
            .runtime
            .as_ref()
            .unwrap()
            .last_compile_stats()
            .unwrap();
        assert!(stats.evaluated.is_empty());
        assert!(stats.reused.contains(&request.feature));
        assert_eq!(unchanged.stamp(), unchanged_stamp);
        assert_eq!(unchanged.instances().len(), number as usize);
        let started = Instant::now();
        session.commit(prepared.prepared).unwrap();
        let applied_stamp = SceneStamp {
            revision: session.revision(),
            generation: stamp.generation + 2,
            ..stamp
        };
        canonical
            .request_evaluation(
                session.document(),
                &resources,
                applied_stamp,
                &Default::default(),
            )
            .unwrap();
        let applied = receive_screw(&mut canonical).result.unwrap();
        let apply_ready_ms = started.elapsed().as_secs_f64() * 1000.;
        let stats = canonical
            .runtime
            .as_ref()
            .unwrap()
            .last_compile_stats()
            .unwrap();
        assert!(
            stats.evaluated.is_empty(),
            "Apply re-ran native feature providers: {stats:?}"
        );
        assert!(stats.reused.contains(&request.feature));
        assert_eq!(applied.stamp(), applied_stamp);
        assert_eq!(applied.instances().len(), number as usize);
        assert!(
            applied
                .instances()
                .iter()
                .any(|instance| instance.id == request.body)
        );
        assert_eq!(
            model::read_mechanical_part(session.document(), request.feature)
                .unwrap()
                .parameters,
            parameters
        );
        assert_eq!(
            applied.model_evaluation().unwrap().source_hash,
            multicad_core::project_file::ContentHash::document(session.document()).unwrap()
        );
        eprintln!(
            "default_m3_l10_b10 feature={number} preview_ms={initial_ms:.3} unchanged_preview_ms={unchanged_ms:.3} apply_canonical_ready_ms={apply_ready_ms:.3}"
        );
    }
}

#[test]
fn app_backend_factory_keeps_candidate_and_canonical_publications_separate() {
    let factory = Backend::factory(GeometryBackend::OpenCascade).unwrap();
    let mut candidate = factory();
    let mut canonical = factory();
    let (session, _) = box_session();
    let stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 1,
    };
    candidate.request(session.document(), stamp).unwrap();
    let canonical_stamp = SceneStamp {
        generation: 2,
        ..stamp
    };
    canonical
        .request(session.document(), canonical_stamp)
        .unwrap();
    candidate.cancel();
    drop(candidate);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = canonical.poll() {
            assert_eq!(reply.stamp, canonical_stamp);
            assert_eq!(reply.result.unwrap().stamp(), canonical_stamp);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

mod painted_default_screw {
    use super::*;
    use multicad_ui::{
        egui,
        modeling::{ModelToolController, ModelToolOutcome},
        viewport::SceneFeed,
    };
    use std::sync::Mutex;

    type Records = Arc<Mutex<Vec<(SceneStamp, Option<multicad_model::CompileStats>)>>>;
    struct RecordedBackend {
        backend: Backend,
        records: Records,
    }
    impl SceneBackend for RecordedBackend {
        fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
            self.backend.request(document, stamp)
        }
        fn request_evaluation(
            &mut self,
            document: &Document,
            resources: &multicad_core::project_file::ProjectResources,
            stamp: SceneStamp,
            options: &EvaluationOptions,
        ) -> Result<(), String> {
            self.backend
                .request_evaluation(document, resources, stamp, options)
        }
        fn request_sources(
            &mut self,
            document: &Document,
            resources: &multicad_core::project_file::ProjectResources,
            stamp: SceneStamp,
            roots: &[multicad_core::document::EntityId],
        ) -> Result<(), String> {
            self.backend
                .request_sources(document, resources, stamp, roots)
        }
        fn cancel(&mut self) {
            self.backend.cancel();
        }
        fn poll(&mut self) -> Option<SceneReply> {
            let reply = self.backend.poll()?;
            let stats = self
                .backend
                .runtime
                .as_ref()
                .ok()
                .and_then(Runtime::last_compile_stats);
            self.records.lock().unwrap().push((reply.stamp, stats));
            Some(reply)
        }
    }
    struct Painted {
        ctx: egui::Context,
        controller: ModelToolController,
        canonical: SceneFeed,
        session: Session,
        applied: Vec<model::CreatedBody>,
        clock: Instant,
    }
    impl Painted {
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let resources = Default::default();
            self.controller.poll(&self.session, &resources);
            self.canonical.tick_project(
                self.session.document(),
                &resources,
                self.session.revision(),
                self.clock.elapsed().as_secs_f64(),
            );
            self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(460., 2600.),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    if self.controller.active()
                        && let Some(ModelToolOutcome::Applied(parts)) = self
                            .controller
                            .show(ui, &mut self.session, &resources)
                            .unwrap()
                    {
                        self.applied.extend(parts);
                    }
                },
            )
        }
        fn click(&mut self, label: &str) {
            self.frame(vec![]).drop_without_applying_deltas();
            let output = self.frame(vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .or_else(|| {
                    output
                        .platform_output
                        .accesskit_update
                        .as_ref()
                        .and_then(|update| {
                            update
                                .nodes
                                .iter()
                                .filter_map(|(_, node)| {
                                    (node.label() == Some(label))
                                        .then(|| node.bounds())
                                        .flatten()
                                })
                                .max_by(|a, b| {
                                    ((a.x1 - a.x0) * (a.y1 - a.y0))
                                        .total_cmp(&((b.x1 - b.x0) * (b.y1 - b.y0)))
                                })
                                .map(|r| {
                                    egui::pos2(
                                        ((r.x0 + r.x1) * 0.5) as f32,
                                        ((r.y0 + r.y1) * 0.5) as f32,
                                    )
                                })
                        })
                })
                .unwrap_or_else(|| panic!("Missing painted hardware control {label}"));
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ])
                .drop_without_applying_deltas();
            }
        }
        fn wait(&mut self, canonical: bool) {
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                self.frame(vec![]).drop_without_applying_deltas();
                let ready = if canonical {
                    !self.canonical.pending()
                        && self
                            .canonical
                            .is_current(self.session.document().id(), self.session.revision())
                } else {
                    !self.controller.pending()
                        && self
                            .controller
                            .can_apply(&self.session, &Default::default())
                };
                if ready {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "Painted default screw timed out: canonical={} controller={:?} scene={:?}",
                    canonical,
                    self.controller.error(),
                    self.canonical.error()
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn painted_rust_thread_controls_create_and_apply_an_internal_bore() {
        let factory = Backend::factory(GeometryBackend::Rust).unwrap();
        let mut controller = ModelToolController::default();
        controller.set_backend(factory());
        let mut canonical = SceneFeed::default();
        canonical.set_backend(factory());
        let mut session = Session::new(Document::new("Painted Rust receiver").unwrap()).unwrap();
        let stock = PrimitiveRequest::new(
            "Cylindrical receiver",
            PrimitiveParameters::Cylinder {
                radius: length(6.),
                height: model::ThreadParameters::default().length,
                frame: Default::default(),
            },
        );
        let prepared =
            model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &stock).unwrap();
        session.commit(prepared.prepared).unwrap();
        let before = session.document().clone();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut painted = Painted {
            ctx,
            controller,
            canonical,
            session,
            applied: vec![],
            clock: Instant::now(),
        };
        painted.wait(true);
        painted
            .controller
            .begin_thread(
                &painted.session,
                &Default::default(),
                painted.canonical.visible().map(Arc::as_ref),
                &[stock.body].into(),
                Channel::Ui,
            )
            .unwrap();
        painted.click("New threaded shaft");
        painted.click("Cut internal thread");
        painted.click("Preview");
        painted.wait(false);
        assert_eq!(painted.session.document(), &before);
        painted.click("Apply");
        assert_eq!(painted.session.revision(), 2);
        painted.wait(true);
        let [created] = painted.applied.as_slice() else {
            panic!("Expected one applied receiving feature");
        };
        assert_eq!(created.body, stock.body);
        let source = model::read_thread(painted.session.document(), created.feature).unwrap();
        assert_eq!(source.mode, model::ThreadMode::CutInternal);
        assert_eq!(source.target.unwrap().producer, stock.feature);
        let scene = painted.canonical.visible().unwrap();
        assert_eq!(scene.instances().len(), 1);
        let axis = viewport::Ray::new([0., 0., 20.], [0., 0., -1.]).unwrap();
        assert!(scene.pick_face(axis).is_none());
        let inside = viewport::Ray::new([0., 0., 2.5], [1., 0., 0.]).unwrap();
        assert!(scene.pick_face(inside).unwrap().point_mm[0] > 3.9);
    }

    #[test]
    fn actual_default_m3_controls_measure_preview_unchanged_and_apply_to_canonical_ready() {
        let owner = Backend::runtime(GeometryBackend::OpenCascade).unwrap();
        let preview_records: Records = Default::default();
        let canonical_records: Records = Default::default();
        let factory = |records| {
            Box::new(RecordedBackend {
                backend: Backend {
                    runtime: owner.fork(),
                    expected: None,
                },
                records,
            }) as Box<dyn SceneBackend>
        };
        let mut controller = ModelToolController::default();
        controller.set_backend(factory(preview_records.clone()));
        let mut canonical = SceneFeed::default();
        canonical.set_backend(factory(canonical_records.clone()));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut painted = Painted {
            ctx,
            controller,
            canonical,
            session: Session::new(Document::new("Painted default M3").unwrap()).unwrap(),
            applied: vec![],
            clock: Instant::now(),
        };
        painted.wait(true);
        for number in 1..=2 {
            let before = painted.session.document().clone();
            let revision = painted.session.revision();
            painted
                .controller
                .begin_mechanical_part(&painted.session, &Default::default(), Channel::Ui)
                .unwrap();
            painted.click("Plain washer");
            painted.click("Machine screw");
            let started = Instant::now();
            painted.click("Preview");
            painted.wait(false);
            let preview_ms = started.elapsed().as_secs_f64() * 1000.;
            assert_eq!(painted.session.document(), &before);
            assert_eq!(painted.session.revision(), revision);
            let started = Instant::now();
            painted.click("Preview");
            painted.wait(false);
            let unchanged_ms = started.elapsed().as_secs_f64() * 1000.;
            {
                let records = preview_records.lock().unwrap();
                let (_, stats) = records.last().expect("Painted native preview publication");
                assert!(
                    stats.as_ref().unwrap().evaluated.is_empty(),
                    "Unchanged preview reran native providers"
                );
                assert_eq!(stats.as_ref().unwrap().reused.len(), number);
            }
            let started = Instant::now();
            painted.click("Apply");
            assert_eq!(painted.session.revision(), revision + 1);
            assert!(!painted.controller.active());
            painted.wait(true);
            let apply_ready_ms = started.elapsed().as_secs_f64() * 1000.;
            let created = painted.applied.last().expect("Painted Apply outcome");
            assert_eq!(painted.applied.len(), number);
            assert_eq!(
                model::read_mechanical_part(painted.session.document(), created.feature)
                    .unwrap()
                    .parameters,
                model::default_machine_screw()
            );
            let scene = painted.canonical.visible().unwrap();
            assert_eq!(scene.stamp().revision, revision + 1);
            assert_eq!(scene.instances().len(), number);
            assert!(
                scene
                    .instances()
                    .iter()
                    .any(|instance| instance.id == created.body)
            );
            {
                let records = canonical_records.lock().unwrap();
                let (stamp, stats) = records.last().unwrap();
                assert_eq!(stamp.revision, revision + 1);
                let stats = stats.as_ref().unwrap();
                assert!(
                    stats.evaluated.is_empty(),
                    "Canonical Apply reran native providers: {stats:?}"
                );
                assert_eq!(stats.reused.len(), number);
                assert!(stats.reused.contains(&created.feature));
            }
            if number == 2 {
                assert_ne!(painted.applied[0].feature, created.feature);
                assert_ne!(painted.applied[0].body, created.body);
            }
            eprintln!(
                "painted_default_m3_l10_b10 feature={number} source={} preview_ready_ms={preview_ms:.3} unchanged_preview_ms={unchanged_ms:.3} apply_canonical_ready_ms={apply_ready_ms:.3}",
                created.feature
            );
        }
    }
}

#[path = "../../../multicad-model/tests/support/rust_panel.rs"]
mod rust_panel;
#[path = "../../../multicad-model/tests/support/rust_polygon.rs"]
mod rust_polygon;

fn wait_for_scene(backend: &mut dyn SceneBackend) -> SceneReply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = backend.poll() {
            return reply;
        }
        assert!(Instant::now() < deadline, "scene did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn rust_model_worker_publishes_the_exact_boolean_pocket_without_mutating_source() {
    let (mut session, stock) = box_session();
    let actor = Actor::local(Channel::Ui);
    let cutter = PrimitiveRequest::new(
        "Through-pocket cutter",
        PrimitiveParameters::Box {
            size: [length(4.), length(4.), length(32.)],
            frame: model::PrimitiveFrame {
                origin: [length(3.), length(5.), length(-1.)],
                ..Default::default()
            },
        },
    );
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor.clone(), &cutter).unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    let cut = model::BooleanRequest::new(
        model::Operation::Cut,
        model::BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        vec![model::BodyInput {
            body: cutter.body,
            producer: cutter.feature,
        }],
    );
    let prepared = model::prepare_boolean(&session, session.revision(), actor, &cut).unwrap();
    let mut worker = Backend::factory(GeometryBackend::Rust).unwrap()();
    let stamp = SceneStamp {
        document: before.id(),
        revision: session.revision() + 1,
        generation: 1,
    };
    worker.request(prepared.prepared.document(), stamp).unwrap();
    let scene = wait_for_scene(&mut *worker).result.unwrap();
    assert_eq!(scene.instances().len(), 1);
    assert_eq!(scene.instances()[0].id, stock.body);
    assert!(
        scene
            .pick_face(viewport::Ray::new([5., 7., 100.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let cap = scene
        .pick_face(viewport::Ray::new([1., 1., 100.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(cap.point_mm, [1., 1., 30.]);
    assert!(cap.intent.is_some());
    assert_eq!(session.document(), &before);
}

#[test]
fn rust_model_worker_reuses_unchanged_viewport_geometry_across_revisions() {
    use multicad_core::document::Mutation;

    let (session, height, _, body, cutter, cut) = rust_panel::fixture(false, 28.);
    let factory = Backend::factory(GeometryBackend::Rust).unwrap();
    let mut backend = factory();
    let first_stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 1,
    };
    backend.request(session.document(), first_stamp).unwrap();
    let first = wait_for_scene(&mut *backend).result.unwrap();
    let mut parameter = session.document().get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "4".into();
    let document = session
        .document()
        .edited(&[Mutation::Replace(parameter)])
        .unwrap();
    let edited_stamp = SceneStamp {
        revision: first_stamp.revision + 1,
        generation: 2,
        ..first_stamp
    };
    backend.request(&document, edited_stamp).unwrap();
    let edited = wait_for_scene(&mut *backend).result.unwrap();
    assert!(Arc::ptr_eq(
        &first.geometries()[&cutter],
        &edited.geometries()[&cutter]
    ));
    assert!(!Arc::ptr_eq(
        &first.geometries()[&cut],
        &edited.geometries()[&cut]
    ));
    assert_eq!(
        edited.geometry_snapshot(cutter).unwrap().revision,
        edited_stamp.revision
    );
    assert_eq!(
        first.geometry_snapshot(cutter).unwrap().revision,
        first_stamp.revision
    );
    assert_eq!(
        edited.native_instance_bounds(body),
        Some([[0., 0., 0.], [50., 20., 4.]])
    );
    let cap = edited
        .pick_face(viewport::Ray::new([5., 5., 20.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(cap.point_mm, [5., 5., 4.]);
}

#[test]
fn cached_viewport_geometry_respects_face_colors_and_document() {
    let (session, request) = box_session();
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &model.snapshot().shapes[&request.feature];
    let face = shape.mesh.face_indices[0];
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    let mut cache = DisplayCache::default();
    cache.prepare(session.document().id());
    let before = cache
        .geometry(request.feature, shape, [(face, red)].into())
        .unwrap();
    let after = cache
        .geometry(request.feature, shape, [(face, blue)].into())
        .unwrap();
    let same = cache
        .geometry(request.feature, shape, [(face, blue)].into())
        .unwrap();
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(Arc::ptr_eq(&after, &same));
    assert_eq!(before.face_color(face), Some(red));
    assert_eq!(after.face_color(face), Some(blue));

    let next_document = EntityId::new();
    cache.prepare(next_document);
    let next = cache
        .geometry(request.feature, shape, [(face, blue)].into())
        .unwrap();
    assert!(!Arc::ptr_eq(&next, &after));
    drop(next);
    cache.prepare(next_document);
    assert!(cache.entries.is_empty());
}

#[test]
fn rust_model_worker_publishes_pickable_panel_and_isolates_preview_failures() {
    let (session, _, extrusion, body, cutter, _) = rust_panel::fixture(true, 28.);
    let factory = Backend::factory(GeometryBackend::Rust).unwrap();
    let mut canonical = factory();
    let mut preview = factory();
    let stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 1,
    };
    canonical.request(session.document(), stamp).unwrap();
    let inspection_stamp = SceneStamp {
        generation: 2,
        ..stamp
    };
    preview
        .request_sources(
            session.document(),
            &ProjectResources::default(),
            inspection_stamp,
            &[cutter],
        )
        .unwrap();

    let canonical_reply = wait_for_scene(&mut *canonical);
    assert_eq!(canonical_reply.stamp, stamp);
    let scene = canonical_reply.result.unwrap();
    assert!(scene.is_full_model());
    assert_eq!(
        scene.native_instance_bounds(body),
        Some([[0., 0., 0.], [50., 20., 8.]])
    );
    assert_eq!(scene.instances().len(), 1);
    assert_eq!(scene.instances()[0].id, body);
    let hit_from_above =
        |x, y| scene.pick_face(viewport::Ray::new([x, y, 20.], [0., 0., -1.]).unwrap());
    assert!(hit_from_above(12., 10.).is_none());
    assert!(hit_from_above(28., 10.).is_none());
    let cap = hit_from_above(5., 5.).unwrap().intent.unwrap();
    assert!(
        cap.origins
            .iter()
            .any(|origin| origin.source == extrusion && origin.role == "cap.end")
    );

    let inspection_reply = wait_for_scene(&mut *preview);
    assert_eq!(inspection_reply.stamp, inspection_stamp);
    let inspection = inspection_reply.result.unwrap();
    assert!(!inspection.is_full_model());
    assert!(
        inspection.reference_snapshots()[&cutter]
            .elements
            .iter()
            .any(|e| {
                e.origins
                    .iter()
                    .any(|origin| origin.source == cutter && origin.role == "side.cylinder")
            })
    );

    let (mut box_doc, box_source) = box_session();
    let box_stamp = SceneStamp {
        document: box_doc.document().id(),
        revision: box_doc.revision(),
        generation: 3,
    };
    preview.request(box_doc.document(), box_stamp).unwrap();
    let box_scene = wait_for_scene(&mut *preview).result.unwrap();
    assert_eq!(box_scene.stamp(), box_stamp);
    let box_cap = box_scene
        .pick_face(viewport::Ray::new([5., 5., 40.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(box_cap.geometry, box_source.feature);
    assert_eq!(box_cap.point_mm, [5., 5., 30.]);

    let sphere = PrimitiveRequest::new(
        "Unsupported sphere",
        PrimitiveParameters::Sphere {
            radius: length(4.),
            frame: Default::default(),
        },
    );
    let create = model::prepare_primitive(
        &box_doc,
        box_doc.revision(),
        Actor::local(Channel::Ui),
        &sphere,
    )
    .unwrap();
    box_doc.commit(create.prepared).unwrap();
    let bad_stamp = SceneStamp {
        revision: box_doc.revision(),
        generation: 4,
        ..box_stamp
    };
    preview.request(box_doc.document(), bad_stamp).unwrap();
    let rejected = wait_for_scene(&mut *preview);
    assert_eq!(rejected.stamp, bad_stamp);
    assert!(
        rejected
            .result
            .err()
            .unwrap()
            .contains("box and axial cylinder")
    );
    assert!(canonical.poll().is_none());
    assert_eq!(scene.stamp(), stamp);
}

#[test]
fn rust_model_worker_picks_a_sketch_extrusion_around_its_holes_and_notch() {
    let factory = Backend::factory(GeometryBackend::Rust).unwrap();
    let mut backend = factory();
    for (i, ((session, _, feature, _), has_circle, reverse)) in [
        (rust_polygon::fixture(), false, false),
        (rust_polygon::fixture_with_circle(), true, false),
        (rust_polygon::fixture_with_height("-6"), false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: i as u64 + 1,
        };
        backend.request(session.document(), stamp).unwrap();
        let scene = wait_for_scene(&mut *backend).result.unwrap();
        assert_eq!(scene.stamp(), stamp);
        assert_eq!(scene.instances().len(), 1);
        let (start_y, end_y, cap_role) = if reverse {
            (-2., 4., "cap.start")
        } else {
            (-8., -2., "cap.end")
        };
        assert_eq!(
            scene.native_instance_bounds(scene.instances()[0].id),
            Some([[0., start_y, 0.], [12., end_y, 10.]])
        );
        let pick = |x, z| scene.pick_face(viewport::Ray::new([x, -20., z], [0., 1., 0.]).unwrap());
        assert!(pick(3., 3.).is_none(), "straight-edged through-hole");
        assert_eq!(pick(9., 3.).is_none(), has_circle, "circular through-hole");
        assert!(pick(9., 8.).is_none(), "concave notch");
        let cap = pick(11., 3.).unwrap();
        assert_eq!(cap.point_mm, [11., start_y, 3.]);
        assert!(
            cap.intent
                .unwrap()
                .origins
                .iter()
                .any(|origin| origin.source == feature && origin.role == cap_role)
        );
    }
}
