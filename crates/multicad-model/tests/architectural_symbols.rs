use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    project_file,
    symbols::{self, SymbolAuthorRequest, SymbolKind, architecture::*},
};
use multicad_model::{Compiler, EvaluationCache};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 0.01, "{a} != {b}");
}
fn material() -> Entity {
    Entity::new(
        "Native material",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 110, 70, 255],
            section_hatch: None,
        },
    )
}
fn authored(role: ArchitectureRole) -> (symbols::SymbolDefinition, EntityId) {
    let material = material();
    let mut author = Session::new(
        Document::new("Authored construction")
            .unwrap()
            .edited(&[Mutation::Insert(material.clone())])
            .unwrap(),
    )
    .unwrap();
    let construction = match role {
        ArchitectureRole::Wall => {
            let value = LayeredWallTypeDefinition {
                height: length(2700.),
                reference: WallLayerReference::Outside,
                layers: WallLayerStack {
                    layers: vec![
                        WallLayer {
                            id: EntityId::new(),
                            name: "Structure".into(),
                            role: None,
                            construction: WallLayerKind::Solid {
                                thickness: length(120.),
                                material: material.id,
                            },
                        },
                        WallLayer {
                            id: EntityId::new(),
                            name: "Air".into(),
                            role: None,
                            construction: WallLayerKind::Cavity {
                                thickness: length(40.),
                            },
                        },
                        WallLayer {
                            id: EntityId::new(),
                            name: "Coating".into(),
                            role: None,
                            construction: WallLayerKind::Coating {
                                material: material.id,
                            },
                        },
                    ],
                },
            };
            let (id, prepared) = prepare_layered_wall_type(
                &author,
                author.revision(),
                actor(),
                "Layered wall",
                &value,
            )
            .unwrap();
            author.commit(prepared).unwrap();
            id
        }
        ArchitectureRole::Fence => {
            let ty = FenceTypeRequest::new(
                "Fence bay",
                FenceConstruction {
                    height: length(1800.),
                    post: FencePostProfile::Rectangular {
                        width: length(90.),
                        depth: length(90.),
                    },
                    post_material: material.id,
                    preferred_post_spacing: length(2000.),
                    layout: FenceBayLayout::ShortLast,
                    infills: vec![FenceInfill {
                        id: EntityId::new(),
                        name: "Panel".into(),
                        material: material.id,
                        bottom: length(100.),
                        height: length(1600.),
                        normal_offset: length(0.),
                        construction: FenceInfillKind::Panel {
                            thickness: length(18.),
                            width: FencePanelWidth::FitBay,
                        },
                    }],
                },
            );
            let prepared = prepare_fence_change(
                &author,
                author.revision(),
                actor(),
                &FenceChange {
                    new_type: Some(ty.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
            author.commit(prepared.model.prepared).unwrap();
            ty.id
        }
    };
    let height = EntityId::new();
    let mut blueprint = ArchitectureBlueprint::new(role, construction);
    blueprint.dimensions.push(ArchitectureDimensionSlot {
        id: height,
        name: "Height".into(),
        target: match role {
            ArchitectureRole::Wall => ArchitectureDimensionTarget::WallHeight,
            ArchitectureRole::Fence => ArchitectureDimensionTarget::FenceHeight,
        },
    });
    let (root, prepared) = prepare_blueprint(
        &author,
        author.revision(),
        actor(),
        "Construction interface",
        &blueprint,
    )
    .unwrap();
    author.commit(prepared).unwrap();
    let request = SymbolAuthorRequest::new(
        "Reusable native construction",
        root,
        SymbolKind::Dynamic,
        vec![],
        None,
    );
    let prepared = symbols::prepare_author(&author, author.revision(), actor(), &request).unwrap();
    author.commit(prepared.prepared).unwrap();
    (
        symbols::read_definition(author.document(), request.definition).unwrap(),
        height,
    )
}
fn context(role: ArchitectureRole, end: f64) -> ArchitectureContext {
    match role {
        ArchitectureRole::Wall => ArchitectureContext::Wall {
            path: vec![[length(0.), length(0.)], [length(end), length(0.)]],
            closed: false,
            level: None,
            base_z: length(0.),
            flipped: false,
        },
        ArchitectureRole::Fence => ArchitectureContext::Fence {
            path: FencePath {
                vertices: [[0., 0.], [end, 0.]]
                    .into_iter()
                    .map(|p| FenceVertex {
                        id: EntityId::new(),
                        point: p.map(length),
                    })
                    .collect(),
                closed: false,
                base_z: length(0.),
            },
            level: None,
            post_positions: vec![],
        },
    }
}
#[test]
fn role_symbols_generate_real_solids_from_independent_paths_and_reopen_without_author_or_library() {
    for role in [ArchitectureRole::Wall, ArchitectureRole::Fence] {
        let (definition, height_slot) = authored(role);
        let pin = definition.pin().unwrap();
        let mut host = Session::new(Document::new("Receiving project").unwrap()).unwrap();
        let imported =
            symbols::prepare_import(&host, host.revision(), actor(), &definition).unwrap();
        let id = imported.definition;
        host.commit(imported.prepared).unwrap();
        let cases = match role {
            ArchitectureRole::Wall => [
                (3000., 2500., 1, 900_000_000.),
                (6000., 3100., 1, 2_232_000_000.),
            ],
            ArchitectureRole::Fence => [
                (4500., 2000., 7, 186_624_000.),
                (6500., 2200., 9, 265_932_000.),
            ],
        };
        let compiler = Compiler::default();
        let mut cache = EvaluationCache::default();
        let mut total_volume = 0.;
        let mut total_bodies = 0;
        let mut placements = Vec::new();
        for (end, height, count, volume) in cases {
            let mut inputs = ArchitectureInputs::default();
            inputs.dimensions.insert(height_slot, length(height));
            let request = ArchitecturePlaceRequest::new(
                host.document(),
                id,
                "Placed construction",
                inputs,
                context(role, end),
            )
            .unwrap();
            let before = host.document().clone();
            let prepared =
                prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
            let model = compiler
                .compile_cached(
                    &mut cache,
                    prepared.prepared.document(),
                    host.revision(),
                    None,
                )
                .unwrap();
            assert_eq!(host.document(), &before);
            total_volume += volume;
            total_bodies += count;
            assert_eq!(model.snapshot().bodies.len(), total_bodies);
            let actual_volume: f64 = model
                .snapshot()
                .bodies
                .keys()
                .map(|id| model.solid(*id).unwrap().volume_mm3().unwrap())
                .sum();
            near(actual_volume, total_volume);
            let bodies = match request.output {
                ArchitectureOutput::Wall { feature, body } => {
                    assert_eq!(
                        read_wall(prepared.prepared.document(), feature)
                            .unwrap()
                            .wall_type,
                        request.ids[&blueprint_for_definition(&definition)
                            .unwrap()
                            .unwrap()
                            .construction]
                    );
                    vec![body]
                }
                ArchitectureOutput::Fence { run, .. } => {
                    fence_members(prepared.prepared.document(), run)
                        .unwrap()
                        .into_iter()
                        .filter_map(|(key, _, body)| {
                            matches!(key, FenceMemberKey::Post { .. }).then_some(body)
                        })
                        .collect()
                }
            };
            for body in bodies {
                let solid = model.solid(body).unwrap();
                assert_eq!(solid.solid_count().unwrap(), 1);
                let bounds = solid.bounds().unwrap();
                near(bounds.max[2] - bounds.min[2], height);
                match role {
                    ArchitectureRole::Wall => near(bounds.max[1] - bounds.min[1], 120.),
                    ArchitectureRole::Fence => near(bounds.max[0] - bounds.min[0], 90.),
                }
            }
            host.commit(prepared.prepared).unwrap();
            assert_eq!(
                read_architecture_instance(host.document(), request.record)
                    .unwrap()
                    .pin,
                pin
            );
            placements.push((request, height));
        }
        compiler
            .compile_cached(&mut cache, host.document(), host.revision(), None)
            .unwrap();
        match role {
            ArchitectureRole::Fence => {
                assert!(cache.last_compile().evaluated.is_empty());
                assert_eq!(cache.last_compile().reused.len(), total_bodies);
            }
            ArchitectureRole::Wall => {
                // Walls intentionally bypass this cache: joins/openings read
                // graph-wide dependencies beyond its audited input closure.
                assert_eq!(cache.last_compile().uncacheable.len(), total_bodies);
                assert_eq!(cache.last_compile().evaluated.len(), total_bodies);
            }
        }
        let (first, height) = &placements[0];
        let before_edit = host.document().clone();
        let mut inputs = first.inputs.clone();
        inputs.dimensions.insert(height_slot, length(height + 200.));
        let edited = prepare_architecture_edit(
            &host,
            host.revision(),
            actor(),
            &ArchitectureEditRequest {
                record: first.record,
                inputs,
            },
        )
        .unwrap();
        let edited_model = compiler
            .compile_cached(
                &mut cache,
                edited.prepared.document(),
                host.revision(),
                None,
            )
            .unwrap();
        assert_eq!(host.document(), &before_edit);
        total_volume += match role {
            ArchitectureRole::Wall => 3000. * 120. * 200.,
            ArchitectureRole::Fence => 4. * 90. * 90. * 200.,
        };
        let edited_volume: f64 = edited_model
            .snapshot()
            .bodies
            .keys()
            .map(|id| edited_model.solid(*id).unwrap().volume_mm3().unwrap())
            .sum();
        near(edited_volume, total_volume);
        assert_eq!(
            read_architecture_instance(edited.prepared.document(), first.record)
                .unwrap()
                .ids,
            first.ids
        );
        host.commit(edited.prepared).unwrap();
        host.undo(host.revision(), actor()).unwrap();
        assert_eq!(host.document(), &before_edit);
        host.redo(host.revision(), actor()).unwrap();
        let saved = host.document().clone();
        let mut bytes = Vec::new();
        project_file::write(
            &project_file::ProjectFile::new(saved.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        drop(definition);
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let reopened = Session::new(reopened.project.document).unwrap();
        assert_eq!(reopened.document(), &saved);
        let model = compiler
            .compile(reopened.document(), reopened.revision())
            .unwrap();
        let actual_volume: f64 = model
            .snapshot()
            .bodies
            .keys()
            .map(|id| model.solid(*id).unwrap().volume_mm3().unwrap())
            .sum();
        near(actual_volume, total_volume);
    }
}

#[test]
fn an_architectural_role_cannot_bypass_native_minimum_geometry_or_publish_partial_members() {
    for role in [ArchitectureRole::Wall, ArchitectureRole::Fence] {
        let (mut definition, _) = authored(role);
        let blueprint = blueprint_for_definition(&definition).unwrap().unwrap();
        let mut entity = definition
            .template
            .get(blueprint.construction)
            .unwrap()
            .clone();
        entity.data = match role {
            ArchitectureRole::Wall => {
                let WallTypeConstruction::Layered(mut wall) =
                    read_wall_construction(&definition.template, blueprint.construction).unwrap()
                else {
                    unreachable!()
                };
                let WallLayerKind::Solid { thickness, .. } =
                    &mut wall.layers.layers[0].construction
                else {
                    unreachable!()
                };
                *thickness = length(1e-6);
                EntityData::Extension {
                    definition: wall.definition().unwrap(),
                    inputs: wall.inputs(),
                }
            }
            ArchitectureRole::Fence => {
                let mut fence =
                    read_fence_type(&definition.template, blueprint.construction).unwrap();
                fence.post = FencePostProfile::Rectangular {
                    width: length(1e-6),
                    depth: length(90.),
                };
                EntityData::Extension {
                    definition: multicad_core::document::Definition {
                        type_id: FENCE_TYPE_KIND.into(),
                        schema_version: FENCE_SCHEMA,
                        data: serde_json::to_value(&fence).unwrap(),
                    },
                    inputs: fence.inputs(),
                }
            }
        };
        definition.template = definition
            .template
            .edited(&[Mutation::Replace(entity)])
            .unwrap();
        definition.revision = "native-refusal".into();
        definition.validate().unwrap();
        let mut host = Session::new(Document::new("Native refusal").unwrap()).unwrap();
        let imported =
            symbols::prepare_import(&host, host.revision(), actor(), &definition).unwrap();
        let id = imported.definition;
        host.commit(imported.prepared).unwrap();
        let before = host.document().clone();
        let request = ArchitecturePlaceRequest::new(
            host.document(),
            id,
            "Thin construction",
            ArchitectureInputs::default(),
            context(role, 2000.),
        )
        .unwrap();
        let prepared =
            prepare_architecture_place(&host, host.revision(), actor(), &request).unwrap();
        let mut cache = EvaluationCache::default();
        let error = match Compiler::default().compile_cached(
            &mut cache,
            prepared.prepared.document(),
            host.revision(),
            None,
        ) {
            Ok(_) => panic!("Native minimum must refuse the complete candidate"),
            Err(error) => error,
        };
        assert!(error.message.contains("minimum"), "{error:?}");
        assert!(cache.is_empty());
        assert_eq!(host.document(), &before);
    }
}
