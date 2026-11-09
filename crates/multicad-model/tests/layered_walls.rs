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
#[test]
fn compound_cutout_retains_every_wall_layers_material_on_new_cut_faces() {
    let (mut session, value) = fixture(WallLayerReference::Outside);
    let ty = NewType::new("Layered receiver", NewTypeKind::LayeredWall(value));
    let wall = request(ty.id, &[[0., 0.], [4000., 0.]], false);
    let prepared = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[ty],
        &WallChange::Create(&wall),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = native(&session);
    let old = before.solid(wall.feature).unwrap();
    let receiver = model::CompoundCutoutReceiver::new(model::BodyInput {
        body: wall.body,
        producer: wall.feature,
    });
    let source = model::CompoundCutoutRequest::new(
        "Compound penetration",
        model::CompoundCutoutDefinition {
            boxes: [2000., 1900.]
                .into_iter()
                .map(|x| {
                    model::CompoundCutoutBox::new(model::PlanRectangle {
                        center: [length(x), length(0.)],
                        width: length(600.),
                        depth: length(2000.),
                        rotation_deg: 0.,
                    })
                })
                .collect(),
            receivers: vec![receiver.clone()],
            extent: model::CompoundCutoutExtent::Fixed(model::CutoutZRange {
                bottom: length(500.),
                top: length(1800.),
            }),
            levels: None,
        },
    );
    let prepared = model::prepare_compound_cutout(
        &session,
        session.revision(),
        actor(),
        &model::CompoundCutoutChange::Set(Box::new(source)),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let after = native(&session);
    let solid = after.solid(receiver.feature).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 5);
    near(
        solid.volume_mm3().unwrap(),
        old.volume_mm3().unwrap() - (80. + 120. + 40. + 100. + 12.5) * 700. * 1300.,
        1e-3,
    );
    let topology = solid
        .topology_snapshot(receiver.feature, session.revision())
        .unwrap();
    let materials = model::wall_face_materials(session.document(), &topology).unwrap();
    assert_eq!(
        materials.len(),
        topology
            .elements
            .iter()
            .filter(|face| face.key.kind == multicad_core::references::TopologyKind::Face)
            .count()
    );
    assert_eq!(
        materials
            .values()
            .map(|face| face.layer)
            .collect::<BTreeSet<_>>()
            .len(),
        5
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("compound-layered-wall.step");
    solid.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert_eq!(readback.solid_count().unwrap(), 5);
    near(
        readback.volume_mm3().unwrap(),
        solid.volume_mm3().unwrap(),
        1e-3,
    );
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
    let topology = solid
        .topology_snapshot(wall.feature, session.revision())
        .unwrap();
    let assignments = model::wall_face_materials(session.document(), &topology).unwrap();
    assert_eq!(
        assignments.len(),
        topology
            .elements
            .iter()
            .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
            .count()
    );
    let mut untagged = topology.clone();
    let face = untagged
        .elements
        .iter_mut()
        .find(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
        .unwrap();
    face.origins.retain(|o| !o.role.starts_with("component/"));
    assert!(model::wall_face_materials(session.document(), &untagged).is_err());
    let mut unknown = topology.clone();
    let face = unknown
        .elements
        .iter_mut()
        .find(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
        .unwrap();
    face.origins.insert(multicad_core::references::Origin {
        source: wall.feature,
        role: format!("component/{}", EntityId::new()),
    });
    assert!(model::wall_face_materials(session.document(), &unknown).is_err());
    let mut conflicting = topology.clone();
    let face = conflicting
        .elements
        .iter_mut()
        .find(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
        .unwrap();
    let current = assignments[&face.key.ordinal].material;
    let other = value.layers.layers.iter().find(|l| matches!(l.construction, WallLayerKind::Solid { material, .. } if material != current)).unwrap();
    face.origins.insert(multicad_core::references::Origin {
        source: wall.feature,
        role: format!("component/{}", other.id),
    });
    assert!(model::wall_face_materials(session.document(), &conflicting).is_err());
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
                let member_topology = member
                    .topology_snapshot(wall.feature, session.revision())
                    .unwrap();
                let mapped =
                    model::wall_face_materials(session.document(), &member_topology).unwrap();
                let expected_material = layer.construction.material().unwrap();
                assert!(
                    mapped
                        .values()
                        .all(|assignment| assignment.material == expected_material)
                );
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

#[test]
fn layer_plan_hatches_and_export_boundaries_follow_real_materials_at_three_scales() {
    use multicad_core::{drawings::*, materials::*, references::ReferenceFrame};
    let (session, value) = fixture(WallLayerReference::Outside);
    let mut edits = Vec::new();
    for (index, material) in session
        .document()
        .entities()
        .filter(|e| matches!(e.data, EntityData::Material { .. }))
        .enumerate()
    {
        let mut material = material.clone();
        let EntityData::Material { section_hatch, .. } = &mut material.data else {
            unreachable!()
        };
        *section_hatch = Some(multicad_core::document::Definition {
            type_id: "materials.hatch.inline".into(),
            schema_version: 1,
            data: serde_json::to_value(HatchStyle {
                color_rgb: None,
                pattern: HatchPattern::Lines,
                angle_deg: (30 + index * 30) as f64,
                spacing_mm: 0.8,
                line_width_mm: [0.11, 0.17, 0.23][index],
            })
            .unwrap(),
        });
        edits.push(Mutation::Replace(material));
    }
    let mut session = Session::new(session.document().edited(&edits).unwrap()).unwrap();
    let ty = NewType::new(
        "Layered construction",
        NewTypeKind::LayeredWall(value.clone()),
    );
    let wall = request(ty.id, &[[0., 0.], [4000., 0.]], false);
    let prepared =
        model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&wall))
            .unwrap();
    session.commit(prepared.prepared).unwrap();
    let mut pages = Vec::new();
    for denominator in [100, 50, 10] {
        let mut page = Page::new(format!("Layers 1:{denominator}"));
        page.settings.paper = PaperSize::Custom {
            width_mm: 600.,
            height_mm: 300.,
        };
        page.views.push(DrawingView {
            id: EntityId::new(),
            name: "Layer cut".into(),
            source: ViewSource::Model {
                entities: vec![wall.body],
            },
            frame: ReferenceFrame {
                origin_mm: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
            kind: ViewKind::Section {
                offset_mm: 1000.,
                range: SectionRange::SectionOnly,
                window: SectionWindow::default(),
            },
            position_mm: [30., 150.],
            scale: Scale {
                numerator: 1,
                denominator,
            },
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.01,
            guide: None,
            equipment_categories: None,
        });
        pages.push(page);
    }
    let (book, prepared) = prepare_create(
        &session,
        session.revision(),
        actor(),
        "Material plans",
        Some(pages.remove(0)),
    )
    .unwrap();
    session.commit(prepared).unwrap();
    // Additional scales use separate books to exercise the persisted page contract.
    let mut books = vec![book];
    for page in pages {
        let (book, prepared) = prepare_create(
            &session,
            session.revision(),
            actor(),
            "Material detail",
            Some(page),
        )
        .unwrap();
        session.commit(prepared).unwrap();
        books.push(book);
    }
    let model = native(&session);
    let evaluation = model
        .drawings(session.document(), session.revision(), &Default::default())
        .unwrap();
    for (book, denominator) in books.into_iter().zip([100., 50., 10.]) {
        let book_value = multicad_core::drawings::read(session.document(), book).unwrap();
        let page = &book_value.pages[0];
        let view = &page.views[0];
        let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&view.id] else {
            panic!("{:?}", evaluation.views[&view.id])
        };
        let hatches: Vec<_> = vectors
            .iter()
            .filter(|v| v.role == VectorRole::Hatch)
            .collect();
        assert!(!hatches.is_empty());
        let resolved = value
            .resolve_layers(
                session.document(),
                session.parameters(),
                &WallLayerPlacement::default(),
            )
            .unwrap();
        for band in &resolved.bands {
            let layer = value
                .layers
                .layers
                .iter()
                .find(|l| l.id == band.layer)
                .unwrap();
            let y_min = band.outside_mm.min(band.inside_mm);
            let y_max = band.outside_mm.max(band.inside_mm);
            if let WallLayerKind::Solid { material, .. } = layer.construction {
                let props = Resolver::new(session.document())
                    .unwrap()
                    .resolve_material(material)
                    .unwrap();
                let expected = props.hatch.unwrap().line_width_mm;
                let in_band: Vec<_> = hatches
                    .iter()
                    .filter(|v| {
                        v.preview_mm
                            .iter()
                            .all(|p| p[1] >= y_min - 1e-6 && p[1] <= y_max + 1e-6)
                    })
                    .collect();
                if (y_max - y_min) / denominator < 0.5 {
                    assert!(
                        in_band.is_empty(),
                        "thin layer hatch must not obscure true boundaries"
                    );
                    continue;
                }
                assert!(
                    !in_band.is_empty(),
                    "layer {} at 1:{denominator}",
                    layer.name
                );
                assert!(
                    in_band
                        .iter()
                        .all(|v| (v.line_width_mm - expected).abs() < 1e-9)
                );
            } else if y_max - y_min > 1e-6 {
                assert!(
                    !hatches.iter().any(|v| v
                        .preview_mm
                        .iter()
                        .any(|p| p[1] > y_min + 1e-6 && p[1] < y_max - 1e-6)),
                    "air has no material hatch"
                );
            }
        }
        let render = render::render_page(
            session.document(),
            session.revision(),
            book,
            page.id,
            &evaluation,
        )
        .unwrap();
        let pdf = pdf::book_pdf(
            session.document(),
            session.revision(),
            &[(book, page.id)],
            &evaluation,
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        let pdf_text = String::from_utf8_lossy(&pdf);
        let begin = pdf_text.find("stream\n").unwrap() + 7;
        let stream = &pdf_text[begin..];
        let stream = &stream[..stream.find("endstream").unwrap()];
        let pdf_paths: Vec<Vec<[f64; 2]>> = stream
            .lines()
            .filter(|l| l.ends_with(" S"))
            .map(|line| {
                let tokens: Vec<_> = line.split_whitespace().collect();
                tokens
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| {
                        if *t == "m" || *t == "l" {
                            Some([
                                tokens[i - 2].parse::<f64>().unwrap() / pdf::PT_PER_MM,
                                tokens[i - 1].parse::<f64>().unwrap() / pdf::PT_PER_MM,
                            ])
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .collect();
        let dxf = dxf::render_dxf(&render, 1., 4);
        if let Ok(directory) = std::env::var("MULTICAD_LAYERED_REVIEW") {
            let directory = std::path::Path::new(&directory);
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join(format!("layers-1-{denominator}.pdf")), &pdf).unwrap();
            std::fs::write(directory.join(format!("layers-1-{denominator}.dxf")), &dxf).unwrap();
            std::fs::write(
                directory.join(format!("layers-1-{denominator}.svg")),
                svg::render_svg(&render),
            )
            .unwrap();
            let file =
                multicad_core::project_file::ProjectFile::new(session.document().clone()).unwrap();
            multicad_core::project_file::write(
                &file,
                &mut std::fs::File::create(directory.join("layered-wall.tcd")).unwrap(),
                Default::default(),
            )
            .unwrap();
        }
        let drawing = ::dxf::Drawing::load(&mut std::io::Cursor::new(dxf.as_bytes())).unwrap();
        let lines: Vec<_> = drawing
            .entities()
            .filter_map(|e| {
                if e.common.layer != "VIEW_SECTION" {
                    return None;
                }
                if let ::dxf::entities::EntityType::Line(l) = &e.specific {
                    Some(([l.p1.x, l.p1.y], [l.p2.x, l.p2.y]))
                } else {
                    None
                }
            })
            .collect();
        for band in
            resolved.bands.iter().filter(|b| {
                value.layers.layers.iter().any(|l| {
                    l.id == b.layer && matches!(l.construction, WallLayerKind::Solid { .. })
                })
            })
        {
            for offset in [band.outside_mm, band.inside_mm] {
                let y = 150. + offset / denominator;
                assert!(
                    pdf_paths.iter().any(|path| path
                        .windows(2)
                        .any(|p| (p[0][1] - y).abs() < 0.002
                            && (p[1][1] - y).abs() < 0.002
                            && ((p[0][0] - p[1][0]).abs() - 4000. / denominator).abs() < 0.002)),
                    "native material boundary missing from PDF at {offset},1:{denominator}"
                );
                assert!(
                    lines.iter().any(|(a, b)| (a[1] - y).abs() < 1e-5
                        && (b[1] - y).abs() < 1e-5
                        && ((a[0] - b[0]).abs() - 4000. / denominator).abs() < 1e-5),
                    "native material boundary missing from DXF at {offset},1:{denominator}"
                );
            }
        }
    }
}

#[test]
fn flush_butt_t_ends_contact_without_overlap_preserve_layers_and_survive_step_readback() {
    use model::{WallEnd, WallJunction, WallJunctionStyle};
    for end in [WallEnd::Start, WallEnd::End] {
        let (mut session, value) = fixture(WallLayerReference::Center);
        let host_type = NewType::new("Main construction", NewTypeKind::LayeredWall(value.clone()));
        let branch_type = NewType::new(
            "Branch construction",
            NewTypeKind::LayeredWall(value.clone()),
        );
        let host = request(host_type.id, &[[0., 0.], [4000., 0.]], false);
        let p = model::prepare_wall_change(
            &session,
            0,
            actor(),
            &[host_type, branch_type.clone()],
            &WallChange::Create(&host),
        )
        .unwrap();
        session.commit(p.prepared).unwrap();
        let baseline = native(&session);
        let host_volume = baseline.solid(host.body).unwrap().volume_mm3().unwrap();
        let points = if end == WallEnd::End {
            [[2000., 3000.], [2000., 0.]]
        } else {
            [[2000., 0.], [2000., 3000.]]
        };
        let mut branch = request(branch_type.id, &points, false);
        let placement = branch.wall.layer_placement.as_mut().unwrap();
        placement.flipped = end == WallEnd::Start;
        placement.junctions.push(WallJunction {
            end,
            host_wall: host.feature,
            host_segment: 0,
            host_point_count: host.wall.path.len(),
            host_closed: host.wall.closed,
            style: WallJunctionStyle::FlushButt,
        });
        let before = session.document().clone();
        let p = model::prepare_wall_change(
            &session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Create(&branch),
        )
        .unwrap();
        let compiled = Compiler::default()
            .compile(p.prepared.document(), session.revision())
            .unwrap();
        let host_solid = compiled.solid(host.body).unwrap();
        let branch_solid = compiled.solid(branch.body).unwrap();
        near(host_solid.volume_mm3().unwrap(), host_volume, 0.01);
        assert_eq!(branch_solid.solid_count().unwrap(), 5);
        near(
            branch_solid.volume_mm3().unwrap(),
            352.5 * (3000. - 191.25) * 2700.,
            0.1,
        );
        for (id, member) in members(branch_solid, branch.feature) {
            let layer = value.layers.layers.iter().find(|l| l.id == id).unwrap();
            let width = layer
                .construction
                .thickness()
                .unwrap()
                .value_in(
                    multicad_core::parameters::Unit::Millimeter,
                    Some(session.parameters()),
                )
                .unwrap();
            let bounds = member.bounds().unwrap();
            near(bounds.min[1], 191.25, 1e-6);
            near(bounds.max[1], 3000., 1e-6);
            near(bounds.max[0] - bounds.min[0], width, 1e-6);
            near(
                member.volume_mm3().unwrap(),
                width * (3000. - 191.25) * 2700.,
                0.1,
            );
            let assignments = model::wall_face_materials(
                p.prepared.document(),
                &member
                    .topology_snapshot(branch.feature, session.revision())
                    .unwrap(),
            )
            .unwrap();
            assert!(
                assignments
                    .values()
                    .all(|a| a.layer == id && Some(a.material) == layer.construction.material())
            );
        }
        let pair = branch_solid
            .inspect_pair(host_solid, 1e-7, GeometryPolicy::default())
            .unwrap();
        assert_eq!(pair.relation, SolidPairRelation::ContactWithinTolerance);
        near(pair.common_volume_mm3, 0., 1e-6);
        let directory = tempfile::tempdir().unwrap();
        let host_path = directory.path().join("host.step");
        let branch_path = directory.path().join("branch.step");
        host_solid.write_step(&host_path).unwrap();
        branch_solid.write_step(&branch_path).unwrap();
        let read_host = Solid::read_step(&host_path, GeometryPolicy::default()).unwrap();
        let read_branch = Solid::read_step(&branch_path, GeometryPolicy::default()).unwrap();
        let pair = read_branch
            .inspect_pair(&read_host, 1e-7, GeometryPolicy::default())
            .unwrap();
        assert_eq!(pair.relation, SolidPairRelation::ContactWithinTolerance);
        near(pair.common_volume_mm3, 0., 1e-6);
        near(
            read_branch.volume_mm3().unwrap(),
            branch_solid.volume_mm3().unwrap(),
            0.1,
        );
        assert_eq!(read_branch.solid_count().unwrap(), 5);
        assert_eq!(
            session.document(),
            &before,
            "native checking cannot commit a draft"
        );
        session.commit(p.prepared).unwrap();
    }
}

#[test]
fn t_join_uses_a_closed_room_segment_and_extends_through_exterior_air_to_a_real_face() {
    use model::{WallEnd, WallJunction, WallJunctionStyle};
    for closed in [false, true] {
        let (mut session, value) = fixture(WallLayerReference::Center);
        let mut host_value = value.clone();
        if !closed {
            host_value.reference = WallLayerReference::Outside;
            host_value.layers.layers.insert(
                0,
                WallLayer {
                    id: EntityId::new(),
                    name: "Exterior air".into(),
                    role: None,
                    construction: WallLayerKind::Cavity {
                        thickness: length(30.),
                    },
                },
            );
        }
        let host_type = NewType::new("Host", NewTypeKind::LayeredWall(host_value));
        let branch_type = NewType::new("Branch", NewTypeKind::LayeredWall(value));
        let points = if closed {
            vec![[0., 0.], [4000., 0.], [4000., 3000.], [0., 3000.]]
        } else {
            vec![[0., 0.], [4000., 0.]]
        };
        let host = request(host_type.id, &points, closed);
        let p = model::prepare_wall_change(
            &session,
            0,
            actor(),
            &[host_type, branch_type.clone()],
            &WallChange::Create(&host),
        )
        .unwrap();
        session.commit(p.prepared).unwrap();
        let baseline = native(&session);
        let host_volume = baseline.solid(host.body).unwrap().volume_mm3().unwrap();
        let mut branch = request(branch_type.id, &[[2000., 1500.], [2000., 0.]], false);
        branch
            .wall
            .layer_placement
            .as_mut()
            .unwrap()
            .junctions
            .push(WallJunction {
                end: WallEnd::End,
                host_wall: host.feature,
                host_segment: 0,
                host_point_count: host.wall.path.len(),
                host_closed: host.wall.closed,
                style: WallJunctionStyle::FlushButt,
            });
        let p = model::prepare_wall_change(
            &session,
            session.revision(),
            actor(),
            &[],
            &WallChange::Create(&branch),
        )
        .unwrap();
        let compiled = Compiler::default()
            .compile(p.prepared.document(), session.revision())
            .unwrap();
        let solid = compiled.solid(branch.body).unwrap();
        let end = if closed { 191.25 } else { -30. };
        near(solid.bounds().unwrap().min[1], end, 1e-6);
        near(
            solid.volume_mm3().unwrap(),
            352.5 * (1500. - end) * 2700.,
            0.1,
        );
        near(
            compiled.solid(host.body).unwrap().volume_mm3().unwrap(),
            host_volume,
            0.1,
        );
        let pair = solid
            .inspect_pair(
                compiled.solid(host.body).unwrap(),
                1e-7,
                GeometryPolicy::default(),
            )
            .unwrap();
        assert_eq!(pair.relation, SolidPairRelation::ContactWithinTolerance);
        near(pair.common_volume_mm3, 0., 1e-6);
    }
}

#[test]
fn a_declared_join_cannot_hide_material_overlap_elsewhere_on_the_branch_path() {
    use model::{WallEnd, WallJunction, WallJunctionStyle};
    let (mut session, value) = fixture(WallLayerReference::Center);
    let ty = NewType::new("Construction", NewTypeKind::LayeredWall(value));
    let host = request(ty.id, &[[0., 0.], [4000., 0.], [4000., 3000.]], false);
    let p = model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&host))
        .unwrap();
    session.commit(p.prepared).unwrap();
    let mut branch = request(
        host.wall.wall_type,
        &[[4500., 1500.], [2000., 1500.], [2000., 0.]],
        false,
    );
    branch
        .wall
        .layer_placement
        .as_mut()
        .unwrap()
        .junctions
        .push(WallJunction {
            end: WallEnd::End,
            host_wall: host.feature,
            host_segment: 0,
            host_point_count: host.wall.path.len(),
            host_closed: host.wall.closed,
            style: WallJunctionStyle::FlushButt,
        });
    let before = session.document().clone();
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap();
    let error = Compiler::default()
        .compile(p.prepared.document(), session.revision())
        .err()
        .unwrap();
    assert!(
        error.message.contains("without material overlap"),
        "{}",
        error.message
    );
    assert_eq!(session.document(), &before);
}

#[test]
fn t_junction_with_door_and_window_exports_a_true_contact_boundary_at_three_scales() {
    use model::{WallEnd, WallJunction, WallJunctionStyle};
    use multicad_core::{drawings::*, references::ReferenceFrame};
    let (mut session, value) = fixture(WallLayerReference::Center);
    let ty = NewType::new("Layered T construction", NewTypeKind::LayeredWall(value));
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
    let mut host = request(ty.id, &[[0., 0.], [4000., 0.]], false);
    host.wall.openings.push(Opening {
        id: EntityId::new(),
        opening_type: door.id,
        segment: 0,
        offset: length(500.),
        width: None,
        height: None,
        sill: None,
    });
    let p = model::prepare_wall_change(
        &session,
        0,
        actor(),
        &[ty, door, window.clone()],
        &WallChange::Create(&host),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut branch = request(host.wall.wall_type, &[[2000., 3000.], [2000., 0.]], false);
    branch
        .wall
        .layer_placement
        .as_mut()
        .unwrap()
        .junctions
        .push(WallJunction {
            end: WallEnd::End,
            host_wall: host.feature,
            host_segment: 0,
            host_point_count: 2,
            host_closed: false,
            style: WallJunctionStyle::FlushButt,
        });
    branch.wall.openings.push(Opening {
        id: EntityId::new(),
        opening_type: window.id,
        segment: 0,
        offset: length(1000.),
        width: None,
        height: None,
        sill: None,
    });
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut books = Vec::new();
    for denominator in [100, 50, 10] {
        let mut page = Page::new(format!("T 1:{denominator}"));
        page.settings.paper = if denominator == 10 {
            PaperSize::A2
        } else {
            PaperSize::A3
        };
        page.info.title = "Layered flush-butt T, door and window".into();
        page.views.push(DrawingView {
            id: EntityId::new(),
            name: "Flush-butt T".into(),
            source: ViewSource::Model {
                entities: vec![host.body, branch.body],
            },
            frame: ReferenceFrame {
                origin_mm: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            },
            kind: ViewKind::Section {
                offset_mm: 1000.,
                range: SectionRange::SectionOnly,
                window: SectionWindow::default(),
            },
            position_mm: if denominator == 10 {
                [40., 320.]
            } else {
                [60., 150.]
            },
            scale: Scale {
                numerator: 1,
                denominator,
            },
            hidden_lines: false,
            tangent_edges: false,
            paper_chord_tolerance_mm: 0.01,
            guide: None,
            equipment_categories: None,
        });
        let (book, p) = prepare_create(
            &session,
            session.revision(),
            actor(),
            "Junction plans",
            Some(page),
        )
        .unwrap();
        session.commit(p).unwrap();
        books.push((book, denominator as f64));
    }
    let compiled = native(&session);
    near(
        compiled.solid(branch.body).unwrap().volume_mm3().unwrap(),
        352.5 * ((3000. - 191.25) * 2700. - 500. * 1000.),
        0.1,
    );
    near(
        compiled.solid(host.body).unwrap().volume_mm3().unwrap(),
        352.5 * (4000. * 2700. - 900. * 2100.),
        0.1,
    );
    let pair = compiled
        .solid(host.body)
        .unwrap()
        .inspect_pair(
            compiled.solid(branch.body).unwrap(),
            1e-7,
            GeometryPolicy::default(),
        )
        .unwrap();
    assert_eq!(pair.relation, SolidPairRelation::ContactWithinTolerance);
    near(pair.common_volume_mm3, 0., 1e-6);
    let evaluation = compiled
        .drawings(session.document(), session.revision(), &Default::default())
        .unwrap();
    for (book, denominator) in books {
        let book_value = multicad_core::drawings::read(session.document(), book).unwrap();
        let page = &book_value.pages[0];
        let view = &page.views[0];
        let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&view.id] else {
            panic!("{:?}", evaluation.views[&view.id])
        };
        assert_eq!(vectors.iter().filter(|v|matches!(v.curve,VectorCurve::Ellipse {major_mm,..} if (major_mm-900.).abs()<1e-6)).count(),1,"one door swing, not one per material component");
        assert!(
            vectors
                .iter()
                .any(|v| v.source == branch.body && v.role == VectorRole::Hatch)
        );
        let expected_a = [
            view.position_mm[0] + 2111.25 / denominator,
            page.settings.dimensions()[1] - view.position_mm[1] + 191.25 / denominator,
        ];
        let expected_b = [view.position_mm[0] + 2191.25 / denominator, expected_a[1]];
        let matches = |a: [f64; 2], b: [f64; 2], tolerance: f64| {
            let same = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < tolerance;
            (same(a, expected_a) && same(b, expected_b))
                || (same(b, expected_a) && same(a, expected_b))
        };
        let render = render::render_page(
            session.document(),
            session.revision(),
            book,
            page.id,
            &evaluation,
        )
        .unwrap();
        let pdf = pdf::book_pdf(
            session.document(),
            session.revision(),
            &[(book, page.id)],
            &evaluation,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&pdf);
        let begin = text.find("stream\n").unwrap() + 7;
        let stream = &text[begin..];
        let stream = &stream[..stream.find("endstream").unwrap()];
        let pdf_has_contact = stream.lines().filter(|l| l.ends_with(" S")).any(|line| {
            let tokens: Vec<_> = line.split_whitespace().collect();
            let points: Vec<[f64; 2]> = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    if *t == "m" || *t == "l" {
                        Some([
                            tokens[i - 2].parse::<f64>().unwrap() / pdf::PT_PER_MM,
                            tokens[i - 1].parse::<f64>().unwrap() / pdf::PT_PER_MM,
                        ])
                    } else {
                        None
                    }
                })
                .collect();
            points.windows(2).any(|p| matches(p[0], p[1], 0.002))
        });
        assert!(
            pdf_has_contact,
            "physical T layer contact missing from PDF at1:{denominator}"
        );
        let dxf = dxf::render_dxf(&render, 1., 4);
        let drawing = ::dxf::Drawing::load(&mut std::io::Cursor::new(dxf.as_bytes())).unwrap();
        assert!(drawing.entities().any(|e|e.common.layer=="VIEW_SECTION" && matches!(&e.specific,::dxf::entities::EntityType::Line(l) if matches([l.p1.x,l.p1.y],[l.p2.x,l.p2.y],1e-5))),"physical T layer contact missing from DXF at1:{denominator}");
        if let Ok(directory) = std::env::var("MULTICAD_JUNCTION_REVIEW") {
            let directory = std::path::Path::new(&directory);
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(
                directory.join(format!("t-junction-1-{denominator}.pdf")),
                &pdf,
            )
            .unwrap();
            std::fs::write(
                directory.join(format!("t-junction-1-{denominator}.dxf")),
                &dxf,
            )
            .unwrap();
            std::fs::write(
                directory.join(format!("t-junction-1-{denominator}.svg")),
                svg::render_svg(&render),
            )
            .unwrap();
            project_file_for_join_fixture(session.document(), directory);
        }
    }
}
fn project_file_for_join_fixture(document: &Document, directory: &std::path::Path) {
    let file = multicad_core::project_file::ProjectFile::new(document.clone()).unwrap();
    multicad_core::project_file::write(
        &file,
        &mut std::fs::File::create(directory.join("layered-t-junction.tcd")).unwrap(),
        Default::default(),
    )
    .unwrap();
}

#[test]
fn perpendicular_t_contact_is_rigidly_invariant_under_rotation_and_translation() {
    use model::{WallEnd, WallJunction, WallJunctionStyle};
    let (mut session, value) = fixture(WallLayerReference::Center);
    let ty = NewType::new("Rotated construction", NewTypeKind::LayeredWall(value));
    let angle = std::f64::consts::PI / 6.;
    let (s, c) = angle.sin_cos();
    let transform = |p: [f64; 2]| [10000. + c * p[0] - s * p[1], 20000. + s * p[0] + c * p[1]];
    let host = request(ty.id, &[[0., 0.], [4000., 0.]].map(transform), false);
    let p = model::prepare_wall_change(&session, 0, actor(), &[ty], &WallChange::Create(&host))
        .unwrap();
    session.commit(p.prepared).unwrap();
    let mut branch = request(
        host.wall.wall_type,
        &[[2000., 3000.], [2000., 0.]].map(transform),
        false,
    );
    branch
        .wall
        .layer_placement
        .as_mut()
        .unwrap()
        .junctions
        .push(WallJunction {
            end: WallEnd::End,
            host_wall: host.feature,
            host_segment: 0,
            host_point_count: 2,
            host_closed: false,
            style: WallJunctionStyle::FlushButt,
        });
    let p = model::prepare_wall_change(
        &session,
        session.revision(),
        actor(),
        &[],
        &WallChange::Create(&branch),
    )
    .unwrap();
    let compiled = Compiler::default()
        .compile(p.prepared.document(), session.revision())
        .unwrap();
    near(
        compiled.solid(branch.body).unwrap().volume_mm3().unwrap(),
        352.5 * (3000. - 191.25) * 2700.,
        0.1,
    );
    let pair = compiled
        .solid(host.body)
        .unwrap()
        .inspect_pair(
            compiled.solid(branch.body).unwrap(),
            1e-7,
            GeometryPolicy::default(),
        )
        .unwrap();
    assert_eq!(pair.relation, SolidPairRelation::ContactWithinTolerance);
    near(pair.common_volume_mm3, 0., 1e-6);
}
