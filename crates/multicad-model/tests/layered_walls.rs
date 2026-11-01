//! Exact native layer members, independent measurements and manufacturing
//! readback. These do not establish graphical editor or junction acceptance.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyOutput, LayeredWallTypeDefinition, NewType, NewTypeKind, Opening, OpeningKind,
        OpeningTypeDefinition, WallChange, WallDefinition, WallLayer, WallLayerKind,
        WallLayerPlacement, WallLayerReference, WallLayerStack, WallRequest, length,
    },
};
use multicad_kernel::{GeometryPolicy, Solid, SolidPairRelation};
use multicad_model::{Compiler, Model};
use std::collections::{BTreeMap, BTreeSet};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} ± {tolerance}"
    );
}
fn fixture(reference: WallLayerReference) -> (Session, LayeredWallTypeDefinition) {
    let material = |name: &str, color| {
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
    let insulation = material("Insulation", [220, 210, 120, 255]);
    let finish = material("Finish", [235, 235, 235, 255]);
    let document = Document::new("Native wall layers")
        .unwrap()
        .edited(
            &[brick.clone(), insulation.clone(), finish.clone()]
                .into_iter()
                .map(Mutation::Insert)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let mut layers = vec![WallLayer {
        id: EntityId::new(),
        name: "Paint".into(),
        role: None,
        construction: WallLayerKind::Coating {
            material: finish.id,
        },
    }];
    for (name, width, material) in [
        ("Insulation", 80., insulation.id),
        ("Brick", 120., brick.id),
        ("Second insulation", 40., insulation.id),
        ("Second brick", 100., brick.id),
        ("Lining", 12.5, finish.id),
    ] {
        layers.push(WallLayer {
            id: EntityId::new(),
            name: name.into(),
            role: None,
            construction: WallLayerKind::Solid {
                thickness: length(width),
                material,
            },
        });
    }
    layers.insert(
        3,
        WallLayer {
            id: EntityId::new(),
            name: "Air cavity".into(),
            role: None,
            construction: WallLayerKind::Cavity {
                thickness: length(30.),
            },
        },
    );
    (
        Session::new(document).unwrap(),
        LayeredWallTypeDefinition {
            layers: WallLayerStack { layers },
            height: length(2700.),
            reference,
        },
    )
}
fn request(ty: EntityId, points: &[[f64; 2]], closed: bool) -> WallRequest {
    WallRequest::new(
        "Layered wall",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty,
            layer_placement: Some(WallLayerPlacement::default()),
            path: points.iter().map(|p| p.map(length)).collect(),
            closed,
            level: None,
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    )
}
fn native(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn members(solid: &Solid, owner: EntityId) -> BTreeMap<EntityId, Solid> {
    let mut result = BTreeMap::new();
    for member in solid.solid_components(GeometryPolicy::default()).unwrap() {
        let snapshot = member.topology_snapshot(owner, 0).unwrap();
        let labels = snapshot
            .elements
            .iter()
            .flat_map(|e| &e.origins)
            .filter(|o| o.source == owner && o.role.starts_with("component/"))
            .map(|o| o.role.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(labels.len(), 1);
        let id = EntityId::parse(
            labels
                .iter()
                .next()
                .unwrap()
                .strip_prefix("component/")
                .unwrap(),
        )
        .unwrap();
        assert!(
            result.insert(id, member).is_none(),
            "Fixture expects one connected member per material layer"
        );
    }
    result
}
fn no_overlap(members: &BTreeMap<EntityId, Solid>) {
    let values = members.values().collect::<Vec<_>>();
    for i in 0..values.len() {
        for j in i + 1..values.len() {
            let pair = values[i]
                .inspect_pair(values[j], 1e-7, GeometryPolicy::default())
                .unwrap();
            assert_ne!(pair.relation, SolidPairRelation::Overlapping);
            near(pair.common_volume_mm3, 0., 1e-5);
        }
    }
}
#[test]
fn real_material_layers_and_openings_have_true_thickness_and_independent_step_readback() {
    let (mut session, value) = fixture(WallLayerReference::Outside);
    let ty = NewType::new(
        "Exterior construction",
        NewTypeKind::LayeredWall(value.clone()),
    );
    let door = NewType::new(
        "Door",
        NewTypeKind::Opening(OpeningTypeDefinition {
            kind: OpeningKind::Door {
                hinge: model::Hinge::Start,
                swing: model::Swing::Left,
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
            width: length(500.),
            height: length(1000.),
            sill: length(900.),
        }),
    );
    let mut wall = request(ty.id, &[[0., 0.], [4000., 0.]], false);
    for (opening_type, offset) in [(door.id, 500.), (window.id, 2000.)] {
        wall.wall.openings.push(Opening {
            id: EntityId::new(),
            opening_type,
            segment: 0,
            offset: length(offset),
            width: None,
            height: None,
            sill: None,
        });
    }
    let preview = model::prepare_wall_change(
        &session,
        0,
        actor(),
        &[ty, door, window],
        &WallChange::Create(&wall),
    )
    .unwrap();
    session.commit(preview.prepared).unwrap();
    let model = native(&session);
    let solid = model.solid(wall.body).unwrap();
    let parts = members(solid, wall.feature);
    assert_eq!(parts.len(), 5);
    assert_eq!(solid.solid_count().unwrap(), 5);
    near(
        solid.volume_mm3().unwrap(),
        352.5 * (4000. * 2700. - 900. * 2100. - 500. * 1000.),
        0.1,
    );
    for layer in &value.layers.layers {
        match &layer.construction {
            WallLayerKind::Solid { thickness, .. } => {
                let thickness_mm = thickness
                    .value_in(
                        multicad_core::parameters::Unit::Millimeter,
                        Some(session.parameters()),
                    )
                    .unwrap();
                let member = parts.get(&layer.id).unwrap();
                let bounds = member.bounds().unwrap();
                near(bounds.max[1] - bounds.min[1], thickness_mm, 1e-6);
                near(
                    member.volume_mm3().unwrap(),
                    thickness_mm * (4000. * 2700. - 900. * 2100. - 500. * 1000.),
                    0.1,
                );
            }
            _ => assert!(
                !parts.contains_key(&layer.id),
                "Coatings and air cavities are not fictitious solids"
            ),
        }
    }
    no_overlap(&parts);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("layered-wall.step");
    solid.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert_eq!(readback.solid_count().unwrap(), 5);
    near(
        readback.volume_mm3().unwrap(),
        solid.volume_mm3().unwrap(),
        0.1,
    );
    let bounds = readback.bounds().unwrap();
    near(bounds.min[1], -382.5, 1e-6);
    near(bounds.max[1], 0., 1e-6);
}
#[test]
fn layered_l_corners_have_disjoint_real_bands_and_flip_uses_the_same_thicknesses() {
    let (session, value) = fixture(WallLayerReference::Center);
    let before = session.document().clone();
    let ty = NewType::new("L construction", NewTypeKind::LayeredWall(value));
    let mut wall = request(ty.id, &[[0., 0.], [4000., 0.], [4000., 3000.]], false);
    for (flipped, volume) in [(false, 6_658_402_500.), (true, 6_666_097_500.)] {
        wall.wall.layer_placement.as_mut().unwrap().flipped = flipped;
        let preview = model::prepare_wall_change(
            &session,
            0,
            actor(),
            std::slice::from_ref(&ty),
            &WallChange::Create(&wall),
        )
        .unwrap();
        let model = Compiler::default()
            .compile(preview.prepared.document(), 1)
            .unwrap();
        let solid = model.solid(wall.body).unwrap();
        near(solid.volume_mm3().unwrap(), volume, 0.1);
        no_overlap(&members(solid, wall.feature));
    }
    assert_eq!(
        session.document(),
        &before,
        "Native preview does not commit the type or wall"
    );
}
#[test]
fn closed_room_outside_is_independent_of_clockwise_or_counterclockwise_drawing() {
    let (session, value) = fixture(WallLayerReference::Outside);
    let ty = NewType::new("Room construction", NewTypeKind::LayeredWall(value.clone()));
    let mut points = vec![[0., 0.], [4000., 0.], [4000., 3000.], [0., 3000.]];
    let mut facts: Option<BTreeMap<EntityId, (multicad_kernel::Bounds, f64)>> = None;
    for _ in 0..2 {
        let wall = request(ty.id, &points, true);
        let preview = model::prepare_wall_change(
            &session,
            0,
            actor(),
            std::slice::from_ref(&ty),
            &WallChange::Create(&wall),
        )
        .unwrap();
        let model = Compiler::default()
            .compile(preview.prepared.document(), 1)
            .unwrap();
        let solid = model.solid(wall.body).unwrap();
        near(solid.volume_mm3().unwrap(), 11_883_712_500., 0.1);
        let parts = members(solid, wall.feature);
        no_overlap(&parts);
        let current = parts
            .iter()
            .map(|(id, part)| (*id, (part.bounds().unwrap(), part.volume_mm3().unwrap())))
            .collect::<BTreeMap<_, _>>();
        if let Some(previous) = &facts {
            for (id, (bounds, volume)) in &current {
                let (old_bounds, old_volume) = previous.get(id).unwrap();
                for axis in 0..3 {
                    near(bounds.min[axis], old_bounds.min[axis], 1e-6);
                    near(bounds.max[axis], old_bounds.max[axis], 1e-6);
                }
                near(*volume, *old_volume, 0.1);
            }
        } else {
            facts = Some(current);
        }
        points.reverse();
    }
}
#[test]
fn unsupported_finite_finish_size_does_not_become_a_thicker_wall_layer() {
    let (mut session, mut value) = fixture(WallLayerReference::Outside);
    if let WallLayerKind::Solid { thickness, .. } = &mut value.layers.layers[1].construction {
        *thickness = length(0.000001);
    }
    let ty = NewType::new("Thin construction", NewTypeKind::LayeredWall(value));
    let wall = request(ty.id, &[[0., 0.], [4000., 0.]], false);
    let preview =
        model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&wall))
            .unwrap();
    session.commit(preview.prepared).unwrap();
    let before = session.document().clone();
    let result = Compiler::default().compile(session.document(), session.revision());
    let error = match result {
        Ok(_) => panic!("Unsupported layer thickness was inflated or accepted"),
        Err(error) => error,
    };
    assert!(
        error.message.contains("Insulation") && error.message.contains("minimum"),
        "{error:?}"
    );
    assert_eq!(session.document(), &before);
}

#[test]
fn partial_depth_chase_cuts_only_the_material_it_physically_reaches() {
    let (mut session, value) = fixture(WallLayerReference::Outside);
    let first_layer = value.layers.layers[1].id;
    let second_layer = value.layers.layers[2].id;
    let ty = NewType::new("Chased construction", NewTypeKind::LayeredWall(value));
    let mut wall = request(ty.id, &[[0., 0.], [4000., 0.]], false);
    let cutout = EntityId::new();
    wall.wall.cutouts.push(model::VerticalCutout {
        id: cutout,
        segment: 0,
        offset: length(1500.),
        width: length(400.),
        depth: Some(length(100.)),
        face: model::WallFace::Left,
    });
    let preview =
        model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&wall))
            .unwrap();
    session.commit(preview.prepared).unwrap();
    let model = native(&session);
    let solid = model.solid(wall.body).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 6);
    near(solid.volume_mm3().unwrap(), 3_699_000_000., 0.1);
    let mut volume = BTreeMap::<EntityId, f64>::new();
    let mut count = BTreeMap::<EntityId, usize>::new();
    for part in solid.solid_components(GeometryPolicy::default()).unwrap() {
        let snapshot = part.topology_snapshot(wall.feature, 0).unwrap();
        let labels = snapshot
            .elements
            .iter()
            .flat_map(|element| &element.origins)
            .filter(|o| o.source == wall.feature && o.role.starts_with("component/"))
            .map(|o| o.role.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(labels.len(), 1);
        let layer = EntityId::parse(
            labels
                .iter()
                .next()
                .unwrap()
                .strip_prefix("component/")
                .unwrap(),
        )
        .unwrap();
        *volume.entry(layer).or_default() += part.volume_mm3().unwrap();
        *count.entry(layer).or_default() += 1;
    }
    assert_eq!(count[&first_layer], 2);
    near(volume[&first_layer], 777_600_000., 0.1);
    near(volume[&second_layer], 1_274_400_000., 0.1);
    let snapshot = solid.topology_snapshot(wall.feature, 0).unwrap();
    let back = multicad_core::references::Origin {
        source: wall.feature,
        role: format!("cutout.{cutout}.back"),
    };
    let faces = snapshot
        .elements
        .iter()
        .filter(|element| {
            element.key.kind == multicad_core::references::TopologyKind::Face
                && element.origins.contains(&back)
        })
        .collect::<Vec<_>>();
    assert!(!faces.is_empty());
    for face in faces {
        assert!(face.origins.contains(&multicad_core::references::Origin {
            source: wall.feature,
            role: format!("component/{second_layer}")
        }));
        if let Some(multicad_core::references::AnalyticGeometry::Plane { frame }) = face.geometry {
            near(frame.origin_mm[1], -100., 1e-6);
        } else {
            panic!("Chase back must be an exact native plane");
        }
    }
}

#[test]
fn exterior_cavity_partial_cut_limit_is_diagnosed_without_flattening_the_construction() {
    let (mut session, mut value) = fixture(WallLayerReference::Outside);
    value.layers.layers.insert(
        0,
        WallLayer {
            id: EntityId::new(),
            name: "Exterior air datum".into(),
            role: None,
            construction: WallLayerKind::Cavity {
                thickness: length(20.),
            },
        },
    );
    let ty = NewType::new("Exterior cavity", NewTypeKind::LayeredWall(value));
    let mut wall = request(ty.id, &[[0., 0.], [4000., 0.]], false);
    wall.wall.cutouts.push(model::VerticalCutout {
        id: EntityId::new(),
        segment: 0,
        offset: length(1500.),
        width: length(400.),
        depth: Some(length(50.)),
        face: model::WallFace::Left,
    });
    let preview =
        model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&wall))
            .unwrap();
    session.commit(preview.prepared).unwrap();
    let before = session.document().clone();
    let error = match Compiler::default().compile(session.document(), session.revision()) {
        Ok(_) => panic!("An unsupported cut was silently approximated"),
        Err(error) => error,
    };
    assert!(error.message.contains("exterior cavity"), "{error:?}");
    assert_eq!(session.document(), &before);
}
