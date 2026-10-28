use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId, Link},
    exchange::*,
    model::{self, length},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, export::Exporter};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn placed(session: &mut Session, component: EntityId, x: f64) -> EntityId {
    let request = model::ComponentPlacementRequest::new(
        component,
        model::NumericTransform {
            translation: [length(x), length(0.), length(0.)],
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_place_component(session, session.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    request.instance
}
#[test]
fn repeated_nested_members_export_their_actual_world_step_and_planar_contours() {
    let mut session = Session::new(Document::new("Nested output binding").unwrap()).unwrap();
    let stock = model::PrimitiveRequest::new(
        "Panel",
        model::PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, 0, actor(), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let inner_members = [
        placed(&mut session, stock.body, 0.),
        placed(&mut session, stock.body, 20.),
    ];
    let inner = model::AssemblyRequest::new("Pair", inner_members.to_vec(), vec![stock.body]);
    session
        .commit(
            model::prepare_assembly(&session, session.revision(), actor(), &inner)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let wrapper_members = [
        placed(&mut session, inner.assembly, 100.),
        placed(&mut session, inner.assembly, 200.),
    ];
    let wrapper = model::AssemblyRequest::new("Wrapper", wrapper_members.to_vec(), vec![]);
    session
        .commit(
            model::prepare_assembly(&session, session.revision(), actor(), &wrapper)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let root = placed(&mut session, wrapper.assembly, 1000.);
    let scope = Scope {
        root: ScopeRoot::Project,
        tags: None,
    };
    let exporter = Exporter::default();
    let catalog = exporter
        .planar_catalog(session.document(), session.revision(), &scope, None)
        .unwrap();
    assert_eq!(
        catalog.parts.len(),
        4,
        "Definition Parts must not be emitted twice"
    );
    let recipe = Recipe {
        id: EntityId::new(),
        name: "Planar".into(),
        output: RecipeOutput::PlanarDxf {
            units: LengthUnit::Millimeter,
            layers: Default::default(),
            aids: AidPolicy::IncludeAll,
        },
    };
    let mut definition = SetDefinition {
        scope,
        recipes: vec![Recipe::step("STEP"), recipe.clone()],
        ..Default::default()
    };
    let mut links = vec![];
    for part in &catalog.parts {
        assert_eq!(part.subject.occurrence_path.len(), 3);
        assert_eq!(part.subject.occurrence_path[0], root);
        assert_eq!(part.subject.part, stock.body);
        assert!(part.diagnostics.is_empty(), "{:?}", part.diagnostics);
        let face = part
            .faces
            .iter()
            .find(|face| face.plane.normal()[2] > 0.9999)
            .unwrap();
        definition.planar_faces.push(PlanarFace {
            recipe: recipe.id,
            subject: part.subject.clone(),
            source: face.source,
            reference: face.intent.id,
            horizontal: PlanarAxis::WorldX,
        });
        links.push(Link {
            target: face.source,
            role: PLANAR_FACE_ROLE.into(),
            geometry: Some(face.intent.clone()),
        });
    }
    let set = EntityId::new();
    session
        .commit(
            prepare_set_with_geometry(
                &session,
                session.revision(),
                actor(),
                set,
                "Nested STEP and DXF",
                &definition,
                &links,
            )
            .unwrap(),
        )
        .unwrap();
    let original = session.document().clone();
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    assert!(
        model.snapshot().references.all_resolved(),
        "{:?}",
        model.snapshot().references
    );
    let preview = exporter
        .preview(session.document(), session.revision(), set, &actor(), None)
        .unwrap();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    assert_eq!(preview.artifacts.len(), 8);
    let mut xmin: Vec<_> = preview
        .artifacts
        .iter()
        .filter(|a| a.planar.is_none())
        .map(|a| a.bounds_mm[0][0])
        .collect();
    xmin.sort_by(f64::total_cmp);
    assert_eq!(xmin, vec![1100., 1120., 1200., 1220.]);
    for artifact in &preview.artifacts {
        assert_eq!(artifact.solid, root);
        assert_eq!(artifact.producer, stock.feature);
        assert_eq!(artifact.solid_count, 1);
        assert!((artifact.volume_mm3 - 6000.).abs() < 1e-6);
        if let Some(planar) = &artifact.planar {
            assert_eq!((planar.outer_loops, planar.inner_loops), (1, 0));
            assert!((planar.area_mm2 - 200.).abs() < 1e-6);
        }
    }
    let output = tempfile::tempdir().unwrap();
    let receipt = exporter
        .run(
            session.document(),
            session.revision(),
            &preview.key,
            output.path(),
            &actor(),
            None,
        )
        .unwrap();
    assert_eq!(receipt.files.len(), 8);
    for file in &receipt.files {
        let path = std::path::Path::new(&receipt.directory).join(&file.artifact.output.filename);
        if file.artifact.planar.is_some() {
            let drawing = ::dxf::Drawing::load_file(&path).unwrap();
            assert!(
                drawing
                    .entities()
                    .any(|e| matches!(e.specific, ::dxf::entities::EntityType::LwPolyline(_)))
            );
        } else {
            let solid = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            assert_eq!(solid.solid_count().unwrap(), 1);
            assert!((solid.volume_mm3().unwrap() - 6000.).abs() < 1e-6);
            assert!((solid.bounds().unwrap().min[0] - file.artifact.bounds_mm[0][0]).abs() < 1e-6);
        }
    }
    assert_eq!(session.document(), &original);
}
