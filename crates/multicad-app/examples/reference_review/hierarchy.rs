//! Isolated CAD-114 acceptance fixture. Boxes are distinct authored B-reps;
//! this namespace is deliberately unavailable as a production feature provider.
use super::backend::{self, Evaluation};
use multicad_core::{
    document::*,
    hierarchy::{Hierarchy, Index},
    references,
};
use multicad_kernel::{GeometryPolicy, RigidTransform, Solid};
use multicad_ui::viewport::{self, SceneStamp};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub const BOX: &str = "hierarchy.review.box";
fn entity(number: u32, name: &str, data: EntityData) -> Entity {
    let mut entity = Entity::new(name, data);
    entity.id =
        EntityId::parse(&format!("11400000-0000-4000-8000-{number:012x}")).expect("fixture UUID");
    entity
}
pub fn document() -> Result<Document, String> {
    let a = entity(1, "Cabinet A", EntityData::Assembly { instances: vec![] });
    let b = entity(2, "Cabinet B", EntityData::Assembly { instances: vec![] });
    let nested = entity(
        3,
        "Drawer module",
        EntityData::Assembly { instances: vec![] },
    );
    let group = entity(10, "Panels", EntityData::Group {});
    let hidden_group = entity(11, "Hidden stock", EntityData::Group {});
    let material = entity(
        20,
        "Hierarchy review material",
        EntityData::Material {
            density_kg_m3: Some(700.),
            color_rgba: [137, 174, 199, 255],
            section_hatch: None,
        },
    );
    let mut document = Document::new("Multi-body hierarchy review").map_err(|e| e.to_string())?;
    let mut state = Hierarchy {
        members: BTreeMap::from([
            (document.id(), vec![a.id, b.id]),
            (a.id, vec![group.id, hidden_group.id, nested.id]),
            (b.id, vec![]),
            (nested.id, vec![]),
            (group.id, vec![]),
            (hidden_group.id, vec![]),
        ]),
        hidden: BTreeSet::from([hidden_group.id]),
        active_component: Some(a.id),
    };
    let mut edits = vec![
        a.clone(),
        b.clone(),
        nested.clone(),
        group.clone(),
        hidden_group.clone(),
        material.clone(),
    ]
    .into_iter()
    .map(Mutation::Insert)
    .collect::<Vec<_>>();
    let mut visible_body = None;
    for (index, name, size, origin, parent, hidden) in [
        (
            0,
            "Visible side A",
            [80., 16., 60.],
            [0., 0., 0.],
            a.id,
            false,
        ),
        (
            1,
            "Grouped shelf A",
            [80., 60., 12.],
            [100., 0., 0.],
            group.id,
            false,
        ),
        (
            2,
            "Hidden side A",
            [16., 60., 60.],
            [200., 0., 0.],
            a.id,
            true,
        ),
        (
            3,
            "Ancestor-hidden back A",
            [80., 12., 60.],
            [240., 0., 0.],
            hidden_group.id,
            false,
        ),
        (
            4,
            "Drawer bottom",
            [60., 40., 8.],
            [0., 90., 0.],
            nested.id,
            false,
        ),
        (
            5,
            "Sibling top B",
            [70., 55., 14.],
            [110., 90., 0.],
            b.id,
            false,
        ),
    ] {
        let producer = entity(
            100 + index * 10,
            &format!("{name} box source"),
            EntityData::Feature {
                definition: Definition {
                    type_id: BOX.into(),
                    schema_version: 1,
                    data: serde_json::json!({"size_mm":size,"origin_mm":origin}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let body = entity(
            101 + index * 10,
            name,
            EntityData::Part {
                features: vec![producer.id],
                material: Some(material.id),
                tags: BTreeSet::from([
                    "hierarchy-review".into(),
                    if hidden {
                        "locally-hidden".into()
                    } else {
                        "authored-body".into()
                    },
                ]),
            },
        );
        state
            .members
            .get_mut(&parent)
            .expect("fixture container")
            .extend([producer.id, body.id]);
        if hidden {
            state.hidden.insert(body.id);
        }
        if index == 0 {
            visible_body = Some(body.id);
        }
        edits.extend([Mutation::Insert(producer), Mutation::Insert(body)]);
    }
    let placed = entity(
        200,
        "Placed reference to side A",
        EntityData::Instance {
            component: visible_body.expect("visible body"),
            placement: Placement {
                translation_mm: [0., -90., 0.],
                ..Default::default()
            },
            placement_source: None,
        },
    );
    state.members.get_mut(&b.id).unwrap().push(placed.id);
    edits.push(Mutation::Insert(placed));
    edits.push(Mutation::SetHierarchy(state));
    document = document.edited(&edits).map_err(|e| e.to_string())?;
    Ok(document)
}
pub fn recognizes(document: &Document) -> bool {
    document
        .entities()
        .any(|e| matches!(&e.data,EntityData::Feature{definition,..}if definition.type_id==BOX))
}
fn box_payload(entity: &Entity) -> Result<([f64; 3], [f64; 3]), String> {
    let EntityData::Feature {
        definition,
        inputs,
        suppressed,
    } = &entity.data
    else {
        return Err("Choose a hierarchy fixture box source".into());
    };
    if definition.type_id != BOX
        || definition.schema_version != 1
        || *suppressed
        || !inputs.is_empty()
        || definition
            .data
            .as_object()
            .is_none_or(|fields| fields.len() != 2)
    {
        return Err("Hierarchy review supports only its isolated box schema; production features remain unavailable".into());
    }
    let size: [f64; 3] =
        serde_json::from_value(definition.data.get("size_mm").cloned().unwrap_or_default())
            .map_err(|e| e.to_string())?;
    let origin: [f64; 3] = serde_json::from_value(
        definition
            .data
            .get("origin_mm")
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    if size
        .iter()
        .any(|v| !v.is_finite() || *v <= 0. || *v > 1000.)
        || origin.iter().any(|v| !v.is_finite() || v.abs() > 5000.)
    {
        return Err("Hierarchy fixture dimensions exceed their review budget".into());
    }
    Ok((size, origin))
}
fn named(document: &Document, id: EntityId) -> serde_json::Value {
    serde_json::json!({"id":id,"name":document.get(id).map(|e|e.name.as_str()).unwrap_or("Document root")})
}
pub fn evaluate(
    document: &Document,
    stamp: SceneStamp,
    check: impl Fn() -> Result<(), String>,
) -> Result<Evaluation, String> {
    if stamp.document != document.id() {
        return Err("Hierarchy review stamp belongs to another document".into());
    }
    let index = Index::new(document).map_err(|e| e.to_string())?;
    let mut geometry = BTreeMap::new();
    let mut volumes = BTreeMap::new();
    let mut bounds = BTreeMap::new();
    for producer in document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Feature { .. }))
    {
        check()?;
        let (size, origin) = box_payload(producer)?;
        let solid = Solid::box_named(size, producer.id, GeometryPolicy::default())
            .map_err(|e| e.to_string())?
            .transformed(RigidTransform {
                translation_mm: origin,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        volumes.insert(producer.id, solid.volume_mm3().map_err(|e| e.to_string())?);
        let b = solid.bounds().map_err(|e| e.to_string())?;
        bounds.insert(producer.id, [b.min, b.max]);
        geometry.insert(
            producer.id,
            backend::geometry(&solid, producer.id, stamp.revision)?,
        );
    }
    let mut bodies = BTreeMap::new();
    let mut producer_owners = BTreeSet::new();
    let mut instances = vec![];
    let mut evidence = vec![];
    for body in document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Part { .. }))
    {
        let EntityData::Part {
            features, material, ..
        } = &body.data
        else {
            unreachable!()
        };
        let [producer] = features.as_slice() else {
            return Err("Each hierarchy review body requires its own one-box producer".into());
        };
        if !geometry.contains_key(producer) || !producer_owners.insert(*producer) {
            return Err("Distinct hierarchy bodies must have distinct native producers; reuse requires an Instance".into());
        }
        let color = material
            .and_then(|id| document.get(id))
            .and_then(|e| {
                if let EntityData::Material { color_rgba, .. } = e.data {
                    Some(color_rgba)
                } else {
                    None
                }
            })
            .unwrap_or([137, 174, 199, 255]);
        bodies.insert(body.id, (*producer, color));
        instances.push(viewport::Instance {
            id: body.id,
            geometry: *producer,
            transform: Default::default(),
            color,
        });
        let owner = index.owning_component(body.id).map_err(|e| e.to_string())?;
        evidence.push(serde_json::json!({"body":named(document,body.id),"producer":named(document,*producer),"owning_component":owner.map(|id|named(document,id)),"effective_visible":index.effective_visibility(body.id).map_err(|e|e.to_string())?,"locally_visible":!document.hierarchy().hidden.contains(&body.id),"native_volume_mm3":volumes[producer],"native_bounds_mm":bounds[producer]}));
    }
    for entry in document.entities() {
        if let EntityData::Instance {
            component,
            placement,
            ..
        } = entry.data
        {
            let (producer, color) = bodies
                .get(&component)
                .ok_or("Hierarchy review can place only one of its known body definitions")?;
            instances.push(viewport::Instance {
                id: entry.id,
                geometry: *producer,
                transform: viewport::Transform::from_placement(placement)?,
                color: *color,
            });
        }
    }
    check()?;
    let scene = Arc::new(viewport::Scene::new(
        stamp,
        geometry.into_values().collect(),
        instances,
    )?);
    let report = references::inspect(document, stamp.revision, &scene.reference_snapshots());
    let component_scopes=document.entities().filter(|e|matches!(e.data,EntityData::Assembly{..})).map(|component|{
        let candidates=index.default_cut_bodies(Some(component.id)).map_err(|e|e.to_string())?;
        Ok(serde_json::json!({"component":named(document,component.id),"visible_source_cut_bodies":candidates.into_iter().map(|id|named(document,id)).collect::<Vec<_>>()}))
    }).collect::<Result<Vec<_>,String>>()?;
    let visible = viewport::VisibilityFilter::default().apply(&scene, document, stamp.revision)?;
    let total = volumes.values().sum();
    Ok(Evaluation {
        scene: scene.clone(),
        report,
        source_volume_mm3: total,
        display_volume_mm3: total,
        hole_center_world: None,
        fixture_evidence: Some(
            serde_json::json!({"scope":"Isolated CAD-114 native hierarchy fixture; no production modeling feature provider","canonical_body_count":bodies.len(),"native_producer_count":volumes.len(),"display_occurrences":scene.instances().len(),"visible_occurrences":visible.instances().len(),"active_component":document.hierarchy().active_component.map(|id|named(document,id)),"active_visible_source_cut_bodies":index.active_cut_bodies().map_err(|e|e.to_string())?.into_iter().map(|id|named(document,id)).collect::<Vec<_>>(),"component_scopes":component_scopes,"bodies":evidence}),
        ),
    })
}
