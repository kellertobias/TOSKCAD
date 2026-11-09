//! Row identity and hierarchy are independent of the glyph used to describe them.
use super::NodeKind;
use crate::icons::Icon;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, PrimitiveParameters},
    symbols,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Presentation {
    pub label: &'static str,
    pub icon: Icon,
    pub linked: bool,
    pub part: Option<EntityId>,
}

#[derive(Default)]
pub(super) struct Catalog {
    linked: BTreeSet<EntityId>,
    symbol_roots: BTreeMap<EntityId, EntityId>,
}

impl Catalog {
    pub(super) fn new(document: &Document) -> Result<Self, String> {
        let mut catalog = Self::default();
        for entity in document.entities() {
            let EntityData::Extension { definition, .. } = &entity.data else {
                continue;
            };
            match definition.type_id.as_str() {
                symbols::INSTANCE_TYPE => {
                    // Reading the record by its ID avoids searching for it
                    // again for every visible occurrence.
                    let record = symbols::read_instance(document, entity.id)
                        .map_err(|error| error.to_string())?;
                    let Some(EntityData::Instance { component, .. }) =
                        document.get(record.member).map(|member| &member.data)
                    else {
                        return Err("A linked symbol is missing its root member".into());
                    };
                    catalog.symbol_roots.insert(record.instance, *component);
                    catalog.linked.extend(record.ids.values().copied());
                    catalog
                        .linked
                        .extend([record.instance, record.wrapper, record.member]);
                }
                symbols::architecture::PLACEMENT_TYPE => {
                    let record =
                        symbols::architecture::read_architecture_instance(document, entity.id)
                            .map_err(|error| error.to_string())?;
                    catalog.linked.extend(record.ids.values().copied());
                    catalog.linked.extend(record.output.anchors());
                }
                _ => {}
            }
        }
        Ok(catalog)
    }

    pub(super) fn row(&self, document: &Document, id: EntityId, kind: NodeKind) -> Presentation {
        let linked = self.linked.contains(&id);
        match document.get(id).map(|entity| &entity.data) {
            Some(EntityData::Part { features, .. }) => Presentation {
                label: "Part",
                icon: part_icon(document, features),
                linked,
                part: Some(id),
            },
            Some(EntityData::Instance { component, .. }) => {
                let source = self.symbol_roots.get(&id).copied().unwrap_or(*component);
                let (label, icon, part) = match document.get(source).map(|entity| &entity.data) {
                    Some(EntityData::Part { features, .. }) => {
                        ("Placed part", part_icon(document, features), Some(source))
                    }
                    Some(EntityData::Assembly { .. }) => ("Placed component", Icon::Assembly, None),
                    _ => (kind.label(), kind.icon(), None),
                };
                Presentation {
                    label,
                    icon,
                    linked: linked || self.linked.contains(&source),
                    part,
                }
            }
            Some(EntityData::Feature { .. }) => Presentation {
                label: kind.label(),
                icon: feature_icon(document, id).unwrap_or_else(|| kind.icon()),
                linked: false,
                part: None,
            },
            _ => Presentation {
                label: kind.label(),
                icon: kind.icon(),
                linked,
                part: None,
            },
        }
    }
}

fn part_icon(document: &Document, features: &[EntityId]) -> Icon {
    // The operation that made the Part supplies its glyph. Later operations
    // such as a fillet or hole do not change a sheet or cylinder into a cut.
    features
        .iter()
        .find_map(|feature| creator_icon(document, *feature))
        .unwrap_or(Icon::Box)
}

fn creator_icon(document: &Document, feature: EntityId) -> Option<Icon> {
    let EntityData::Feature { definition, .. } = &document.get(feature)?.data else {
        return None;
    };
    Some(match definition.type_id.as_str() {
        model::PRIMITIVE_TYPE => match model::read_primitive(document, feature).ok()?.parameters {
            PrimitiveParameters::Box { .. } => Icon::Box,
            PrimitiveParameters::Cylinder { .. } => Icon::Cylinder,
            PrimitiveParameters::Sphere { .. } => Icon::Sphere,
        },
        model::SHEET_TYPE => Icon::SheetFold,
        model::MEMBER_TYPE => Icon::Sweep,
        model::PANEL_TYPE | model::SLAB_TYPE => Icon::Thickness,
        model::MECHANICAL_PART_TYPE
        | model::HARDWARE_TYPE
        | model::THREAD_TYPE
        | model::GEAR_TYPE => Icon::Joint,
        model::EXTRUDE_TYPE => Icon::Extrude,
        model::REVOLVE_TYPE => Icon::Revolve,
        model::WALL_TYPE | model::FENCE_MEMBER_TYPE => Icon::Home,
        model::STAIR_TYPE => Icon::Stairs,
        _ => return None,
    })
}

fn feature_icon(document: &Document, feature: EntityId) -> Option<Icon> {
    if let Some(icon) = creator_icon(document, feature) {
        return Some(icon);
    }
    let EntityData::Feature { definition, .. } = &document.get(feature)?.data else {
        return None;
    };
    Some(match definition.type_id.as_str() {
        model::EDGE_TREATMENT_TYPE => Icon::Edge,
        model::BOOLEAN_TYPE => Icon::Boolean,
        model::SCALE_TYPE => Icon::Scale,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        document::Placement,
        project_file,
    };

    #[test]
    fn linked_and_local_placements_use_the_actual_part_kind() {
        let mut session = Session::new(Document::new("Tree kinds").unwrap()).unwrap();
        let actor = || Actor::local(Channel::Ui);
        let part = model::PrimitiveRequest::new(
            "Cylinder",
            PrimitiveParameters::Cylinder {
                radius: model::length(10.),
                height: model::length(20.),
                frame: Default::default(),
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), &part).unwrap();
        session.commit(prepared.prepared).unwrap();

        let author = symbols::SymbolAuthorRequest::new(
            "Cylinder symbol",
            part.body,
            symbols::SymbolKind::Static,
            vec![],
            None,
        );
        let authored =
            symbols::prepare_author(&session, session.revision(), actor(), &author).unwrap();
        let symbol =
            symbols::read_definition(authored.prepared.document(), authored.definition).unwrap();

        let local = model::ComponentPlacementRequest::new(part.body, Default::default());
        let prepared =
            model::prepare_place_component(&session, session.revision(), actor(), &local).unwrap();
        session.commit(prepared.prepared).unwrap();

        let linked = symbols::SymbolPlaceRequest::from_definition(
            EntityId::new(),
            &symbol,
            "Library cylinder",
            Default::default(),
            Placement::default(),
        )
        .unwrap();
        let prepared = symbols::prepare_library_place(
            &session,
            session.revision(),
            actor(),
            &symbol,
            &linked,
            None,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();

        let document = session.document();
        let catalog = Catalog::new(document).unwrap();
        let original = catalog.row(document, part.body, NodeKind::Body);
        assert_eq!(original.label, "Part");
        assert!(matches!(original.icon, Icon::Cylinder));
        assert_eq!(original.part, Some(part.body));
        assert!(!original.linked);
        let local_row = catalog.row(document, local.instance, NodeKind::Instance);
        assert_eq!(local_row.label, "Placed part");
        assert!(matches!(local_row.icon, Icon::Cylinder));
        assert_eq!(local_row.part, Some(part.body));
        assert!(!local_row.linked);
        let linked_row = catalog.row(document, linked.instance, NodeKind::Instance);
        assert_eq!(linked_row.label, "Placed part");
        assert!(matches!(linked_row.icon, Icon::Cylinder));
        assert_eq!(linked_row.part, Some(linked.ids[&symbol.root]));
        assert!(linked_row.linked);
        assert!(
            catalog
                .row(document, linked.wrapper, NodeKind::Component)
                .linked
        );
    }

    #[test]
    fn shipped_sheet_metal_assembly_distinguishes_placed_sheet_and_component() {
        let project = project_file::read(
            &mut include_bytes!("../../../../examples/front-panel.tcd").as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let document = &project.document;
        let catalog = Catalog::new(document).unwrap();
        let find = |name| {
            document
                .entities()
                .find(|entity| entity.name == name)
                .unwrap()
                .id
        };
        let placed_assembly = find("Assembled front panel");
        let assembly = catalog.row(document, placed_assembly, NodeKind::Instance);
        assert_eq!(assembly.label, "Placed component");
        assert!(matches!(assembly.icon, Icon::Assembly));
        assert!(!assembly.linked);
        let EntityData::Instance { component, .. } = document.get(placed_assembly).unwrap().data
        else {
            panic!("bundled assembly must have a placed component");
        };
        let definition = catalog.row(document, component, NodeKind::Component);
        assert_eq!(definition.label, "Component");
        assert!(matches!(definition.icon, Icon::Assembly));
        let placed = catalog.row(
            document,
            find("Sheet-metal front panel"),
            NodeKind::Instance,
        );
        assert_eq!(placed.label, "Placed part");
        assert!(matches!(placed.icon, Icon::SheetFold));
        assert_eq!(placed.part, Some(find("Front panel sheet")));
        assert!(!placed.linked);
    }
}
