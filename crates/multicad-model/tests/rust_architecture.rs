use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{EntityData, EntityId},
    model::{self, OpeningKind, SlabHole, VerticalCutout, WallFace, WallOpeningChange, length},
    project_file,
    references::TopologyKind,
};
use multicad_model::{Compiler, GeometryBackend, Model};

fn apartment() -> Session {
    let mut bytes = include_bytes!("../../../examples/apartment.tcd").as_slice();
    let opened = project_file::read(
        &mut bytes,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    Session::new(opened.project.document).unwrap()
}

fn architecture(session: &Session, type_id: &str) -> Vec<EntityId> {
    session
        .document()
        .entities()
        .filter_map(|entity| {
            let EntityData::Feature { definition, .. } = &entity.data else {
                return None;
            };
            (definition.type_id == type_id).then_some(entity.id)
        })
        .collect()
}

fn compile(session: &Session, sources: &[EntityId], backend: GeometryBackend) -> Model {
    Compiler::default()
        .compile_sources_with_backend(
            session.document(),
            session.revision(),
            sources,
            backend,
            None,
        )
        .unwrap()
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

fn face_has_role(model: &Model, feature: EntityId, role: &str) -> bool {
    model.snapshot().shapes[&feature]
        .topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
        .flat_map(|element| &element.origins)
        .any(|origin| origin.source == feature && origin.role == role)
}

#[test]
fn shipped_apartment_walls_and_slab_rebuild_with_openings() {
    let session = apartment();
    let walls = architecture(&session, model::WALL_TYPE);
    let slabs = architecture(&session, model::SLAB_TYPE);
    assert_eq!(walls.len(), 4);
    assert_eq!(slabs.len(), 1);
    let sources: Vec<_> = walls.iter().chain(&slabs).copied().collect();
    let rust = compile(&session, &sources, GeometryBackend::Rust);
    let native = compile(&session, &sources, GeometryBackend::OpenCascade);
    for feature in sources {
        assert!(rust.solid(feature).unwrap().uses_rust_geometry());
        let shape = &rust.snapshot().shapes[&feature];
        let reference = &native.snapshot().shapes[&feature];
        assert_eq!(shape.solid_count, 1);
        near(shape.volume_mm3, reference.volume_mm3, 1e-3);
        for axis in 0..3 {
            near(shape.bounds.min[axis], reference.bounds.min[axis], 1e-6);
            near(shape.bounds.max[axis], reference.bounds.max[axis], 1e-6);
        }
        assert!(!shape.mesh.indices.is_empty());
    }
    let mut openings = 0;
    for feature in walls {
        let source = model::read_wall(session.document(), feature).unwrap();
        for opening in source.openings {
            for side in ["head", "jamb.start", "jamb.end"] {
                let role = format!("opening.{}.{side}", opening.id);
                assert!(face_has_role(&rust, feature, &role), "missing {role}");
            }
            let opening_type =
                model::read_opening_type(session.document(), opening.opening_type).unwrap();
            if opening_type.kind == OpeningKind::Window {
                let role = format!("opening.{}.sill", opening.id);
                assert!(face_has_role(&rust, feature, &role), "missing {role}");
            }
            openings += 1;
        }
    }
    assert_eq!(openings, 10);
}

#[test]
fn slab_shaft_edit_removes_material_and_keeps_the_four_sides_named() {
    let mut session = apartment();
    let [feature] = architecture(&session, model::SLAB_TYPE).try_into().unwrap();
    let baseline = compile(&session, &[feature], GeometryBackend::Rust);
    let mut slab = model::read_slab(session.document(), feature).unwrap();
    let shaft = EntityId::new();
    slab.holes.push(SlabHole {
        id: shaft,
        center: [length(500.), length(500.)],
        width: length(500.),
        depth: length(800.),
        rotation_deg: 30.,
    });
    let edit = model::prepare_slab_edit(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        feature,
        &slab,
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let updated = compile(&session, &[feature], GeometryBackend::Rust);
    let native = compile(&session, &[feature], GeometryBackend::OpenCascade);
    let old = &baseline.snapshot().shapes[&feature];
    let new = &updated.snapshot().shapes[&feature];
    near(
        new.volume_mm3,
        native.snapshot().shapes[&feature].volume_mm3,
        1e-3,
    );
    assert_eq!(old.bounds, new.bounds);
    let height = old.bounds.max[2] - old.bounds.min[2];
    near(old.volume_mm3 - new.volume_mm3, 500. * 800. * height, 1e-3);
    for side in 0..4 {
        assert!(face_has_role(
            &updated,
            feature,
            &format!("cutout.{shaft}.side.{side}")
        ));
    }
}

#[test]
fn edited_wall_can_cut_a_chase_and_shorten_its_end() {
    let mut session = apartment();
    let (feature, wall) = uncut_partition(&session);
    let baseline = compile(&session, &[feature], GeometryBackend::Rust);
    let chase = EntityId::new();
    let end = EntityId::new();
    for cutout in [
        VerticalCutout {
            id: chase,
            segment: 0,
            offset: length(600.),
            width: length(200.),
            depth: Some(length(40.)),
            face: WallFace::Right,
        },
        VerticalCutout {
            id: end,
            segment: 0,
            offset: length(3_090.),
            width: length(300.),
            depth: None,
            face: WallFace::Left,
        },
    ] {
        let edit = model::prepare_wall_opening(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            feature,
            &[],
            &WallOpeningChange::AddCutout(cutout),
        )
        .unwrap();
        session.commit(edit.prepared).unwrap();
    }
    let updated = compile(&session, &[feature], GeometryBackend::Rust);
    let old = &baseline.snapshot().shapes[&feature];
    let new = &updated.snapshot().shapes[&feature];
    assert_eq!(new.solid_count, 1);
    near(new.bounds.max[1], old.bounds.max[1] - 300., 1e-6);
    let wall = model::resolve_wall(session.document(), session.parameters(), &wall).unwrap();
    near(
        old.volume_mm3 - new.volume_mm3,
        (200. * 40. + 300. * wall.thickness_mm) * wall.height_mm,
        1e-3,
    );
    assert!(face_has_role(
        &updated,
        feature,
        &format!("cutout.{chase}.back")
    ));
    assert!(face_has_role(
        &updated,
        feature,
        &format!("cutout.{end}.jamb.start")
    ));
}

fn uncut_partition(session: &Session) -> (EntityId, model::WallDefinition) {
    architecture(session, model::WALL_TYPE)
        .into_iter()
        .find_map(|feature| {
            let wall = model::read_wall(session.document(), feature).unwrap();
            (wall.path.len() == 2 && wall.openings.is_empty()).then_some((feature, wall))
        })
        .unwrap()
}

#[test]
fn edited_diagonal_wall_matches_the_native_volume_and_boundary() {
    let mut session = apartment();
    let (feature, mut wall) = uncut_partition(&session);
    wall.path[1][0] = length(4700.);
    let edit = model::prepare_wall_edit(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        feature,
        &wall,
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let rust = compile(&session, &[feature], GeometryBackend::Rust);
    let native = compile(&session, &[feature], GeometryBackend::OpenCascade);
    let actual = &rust.snapshot().shapes[&feature];
    let expected = &native.snapshot().shapes[&feature];
    assert_eq!(actual.solid_count, 1);
    assert!(rust.solid(feature).unwrap().uses_rust_geometry());
    near(actual.volume_mm3, expected.volume_mm3, 1e-3);
    for axis in 0..3 {
        near(actual.bounds.min[axis], expected.bounds.min[axis], 1e-6);
        near(actual.bounds.max[axis], expected.bounds.max[axis], 1e-6);
    }
    assert!(face_has_role(&rust, feature, "wall.seg.0.left"));
}
