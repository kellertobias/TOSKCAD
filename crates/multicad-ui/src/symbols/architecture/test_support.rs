use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData},
    model::*,
    symbols::{self, SymbolAuthorRequest, SymbolKind, architecture::*},
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
pub(crate) fn definition(
    role: ArchitectureRole,
) -> (symbols::SymbolDefinition, ArchitectureBlueprint) {
    let material = Entity::new(
        "Definition material",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [150, 110, 70, 255],
            section_hatch: None,
        },
    );
    let mut source = Session::new(
        Document::new("Authoring source")
            .unwrap()
            .edited(&[multicad_core::document::Mutation::Insert(material.clone())])
            .unwrap(),
    )
    .unwrap();
    let (construction, layer) = match role {
        ArchitectureRole::Wall => {
            let layer = multicad_core::document::EntityId::new();
            let value = LayeredWallTypeDefinition {
                height: length(2700.),
                reference: WallLayerReference::Outside,
                layers: WallLayerStack {
                    layers: vec![WallLayer {
                        id: layer,
                        name: "Structure".into(),
                        role: None,
                        construction: WallLayerKind::Solid {
                            thickness: length(120.),
                            material: material.id,
                        },
                    }],
                },
            };
            let (id, prepared) = prepare_layered_wall_type(
                &source,
                source.revision(),
                actor(),
                "Wall construction",
                &value,
            )
            .unwrap();
            source.commit(prepared).unwrap();
            (id, Some(layer))
        }
        ArchitectureRole::Fence => {
            let value = FenceTypeRequest::new(
                "Fence construction",
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
                        id: multicad_core::document::EntityId::new(),
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
                &source,
                source.revision(),
                actor(),
                &FenceChange {
                    new_type: Some(value.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
            source.commit(prepared.model.prepared).unwrap();
            (value.id, None)
        }
    };
    let mut blueprint = ArchitectureBlueprint::new(role, construction);
    blueprint.dimensions.push(ArchitectureDimensionSlot {
        id: multicad_core::document::EntityId::new(),
        name: "Height".into(),
        target: match role {
            ArchitectureRole::Wall => ArchitectureDimensionTarget::WallHeight,
            ArchitectureRole::Fence => ArchitectureDimensionTarget::FenceHeight,
        },
    });
    if role == ArchitectureRole::Fence {
        blueprint.dimensions.push(ArchitectureDimensionSlot {
            id: multicad_core::document::EntityId::new(),
            name: "Spacing".into(),
            target: ArchitectureDimensionTarget::FenceSpacing,
        });
    }
    blueprint.materials.push(ArchitectureMaterialSlot {
        id: multicad_core::document::EntityId::new(),
        name: "Material".into(),
        target: match layer {
            Some(layer) => ArchitectureMaterialTarget::WallLayer { layer },
            None => ArchitectureMaterialTarget::FencePost,
        },
        choices: vec![material.id],
        allow_project_materials: true,
    });
    let (root, prepared) =
        prepare_blueprint(&source, source.revision(), actor(), "Interface", &blueprint).unwrap();
    source.commit(prepared).unwrap();
    let request = SymbolAuthorRequest::new(
        "Reusable construction",
        root,
        SymbolKind::Dynamic,
        vec![],
        None,
    );
    let prepared = symbols::prepare_author(&source, source.revision(), actor(), &request).unwrap();
    source.commit(prepared.prepared).unwrap();
    (
        symbols::read_definition(source.document(), request.definition).unwrap(),
        blueprint,
    )
}
pub(crate) fn consumer(
    definition: &symbols::SymbolDefinition,
) -> (Session, multicad_core::document::EntityId) {
    let mut host = Session::new(Document::new("Receiver").unwrap()).unwrap();
    let prepared = symbols::prepare_import(&host, 0, actor(), definition).unwrap();
    let id = prepared.definition;
    host.commit(prepared.prepared).unwrap();
    (host, id)
}
