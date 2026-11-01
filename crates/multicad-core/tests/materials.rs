use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::*,
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    project_file::{self, ContentHash, ProjectFile},
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn commit(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture".into(),
        edits,
    })
    .unwrap();
}
fn library(s: &mut Session, entries: &[Entity]) {
    let p = prepare_library(s, s.revision(), actor(), entries).unwrap();
    s.commit(p).unwrap();
}
fn tags(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| (*s).into()).collect()
}
struct Fixture {
    s: Session,
    parts: Vec<EntityId>,
    material: Entity,
    stock: Entity,
    appearance: Entity,
    hatch: Entity,
    style: Entity,
}
impl Fixture {
    fn new() -> Self {
        let mut s = Session::new(Document::new("Material furniture").unwrap()).unwrap();
        let mut parts = Vec::new();
        for name in ["Cabinet side", "Shelf"] {
            let r = PrimitiveRequest::new(
                name,
                PrimitiveParameters::Box {
                    size: [length(600.), length(400.), length(18.)],
                    frame: Default::default(),
                },
            );
            let p = model::prepare_primitive(&s, s.revision(), actor(), &r).unwrap();
            s.commit(p.prepared).unwrap();
            parts.push(r.body);
        }
        let material = Entity::new(
            "Birch plywood",
            EntityData::Material {
                density_kg_m3: Some(650.),
                color_rgba: [210, 180, 130, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: material.id,
            specification: StockKind::Panel {
                thickness_mm: 18.,
                length_mm: 2440.,
                width_mm: 1220.,
                grain: Grain::AlongLength,
            },
        })
        .entity(EntityId::new(), "18mm plywood sheet")
        .unwrap();
        let appearance = LibraryDefinition::Appearance(Appearance {
            color_rgba: [200, 165, 110, 255],
            roughness: 0.7,
            metallic: 0.0,
        })
        .entity(EntityId::new(), "Plywood appearance")
        .unwrap();
        let hatch = LibraryDefinition::Hatch(HatchStyle::default())
            .entity(EntityId::new(), "Wood section")
            .unwrap();
        library(
            &mut s,
            &[
                material.clone(),
                stock.clone(),
                appearance.clone(),
                hatch.clone(),
            ],
        );
        let style = LibraryDefinition::MaterialStyle(MaterialStyle {
            material: material.id,
            appearance: Some(LibraryRef::capture(s.document(), appearance.id).unwrap()),
            hatch: Some(LibraryRef::capture(s.document(), hatch.id).unwrap()),
        })
        .entity(EntityId::new(), "Plywood defaults")
        .unwrap();
        library(&mut s, std::slice::from_ref(&style));
        Self {
            s,
            parts,
            material,
            stock,
            appearance,
            hatch,
            style,
        }
    }
    fn assignment(&self) -> Assignment {
        Assignment {
            material: Some(self.material.id),
            stock: Some(self.stock.id),
            appearance: None,
            hatch: None,
        }
    }
    fn assign(&mut self) {
        let p = prepare_assignment(
            &self.s,
            self.s.revision(),
            actor(),
            &self.parts,
            &self.assignment(),
        )
        .unwrap();
        self.s.commit(p).unwrap();
    }
}
#[test]
fn distinct_linked_properties_roundtrip_and_single_undo_preserve_real_model_history() {
    let mut f = Fixture::new();
    let before = f.s.document().clone();
    let hash = ContentHash::document(&before).unwrap();
    let p = prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts, &f.assignment()).unwrap();
    assert_eq!(ContentHash::document(f.s.document()).unwrap(), hash);
    assert_eq!(p.actor().channel, Channel::Mcp);
    for part in &f.parts {
        let r = resolve(p.document(), *part).unwrap();
        assert_eq!(r.stock.as_ref().unwrap().0, f.stock.id);
        assert_eq!(r.appearance.color_rgba, [200, 165, 110, 255]);
        assert_eq!(r.hatch, Some(HatchStyle::default()));
    }
    f.s.commit(p).unwrap();
    let after = f.s.document().clone();
    let file = ProjectFile::new(after.clone()).unwrap();
    assert!(file.capabilities().contains(CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(CAPABILITY)
    );
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, after);
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &before);
    f.s.redo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.s.document(), &after);
}
#[test]
fn material_change_updates_inheritance_and_part_override_is_independent() {
    let mut f = Fixture::new();
    f.assign();
    let mut other = f.material.clone();
    other.id = EntityId::new();
    other.name = "Steel".into();
    other.data = EntityData::Material {
        density_kg_m3: Some(7850.),
        color_rgba: [70, 80, 90, 255],
        section_hatch: None,
    };
    library(&mut f.s, std::slice::from_ref(&other));
    let assignment = Assignment {
        material: Some(other.id),
        stock: None,
        appearance: Some(f.appearance.id),
        hatch: Some(f.hatch.id),
    };
    let p = prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts[..1], &assignment).unwrap();
    f.s.commit(p).unwrap();
    let r = resolve(f.s.document(), f.parts[0]).unwrap();
    assert_eq!(r.material, Some(other.id));
    assert_eq!(r.hatch, Some(HatchStyle::default()));
    assert_eq!(r.appearance.color_rgba, [200, 165, 110, 255]);
    let assignment = Assignment {
        appearance: None,
        hatch: None,
        ..assignment
    };
    let p = prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts[..1], &assignment).unwrap();
    f.s.commit(p).unwrap();
    let r = resolve(f.s.document(), f.parts[0]).unwrap();
    assert_eq!(r.appearance.color_rgba, [70, 80, 90, 255]);
    assert!(r.hatch.is_none());
    assert_eq!(
        resolve(f.s.document(), f.parts[1]).unwrap().material,
        Some(f.material.id)
    );
}
#[test]
fn missing_wrong_and_changed_definitions_are_explicit_and_never_silent_fallback() {
    let mut f = Fixture::new();
    f.assign();
    let original = f.s.document().clone();
    let mut changed = f.appearance.clone();
    changed.data = LibraryDefinition::Appearance(Appearance {
        color_rgba: [10, 20, 30, 255],
        ..Default::default()
    })
    .entity(changed.id, changed.name.clone())
    .unwrap()
    .data;
    library(&mut f.s, std::slice::from_ref(&changed));
    let r = resolve(f.s.document(), f.parts[0]).unwrap();
    assert_eq!(r.appearance.color_rgba, [10, 20, 30, 255]);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::DefinitionChanged && d.target == f.appearance.id)
    );
    commit(&mut f.s, vec![Mutation::Remove(f.appearance.id)]);
    assert!(resolve(f.s.document(), f.parts[0]).is_err());
    assert!(
        diagnostics(f.s.document())
            .unwrap()
            .iter()
            .any(|d| d.kind == DiagnosticKind::Missing)
    );
    let mut wrong = f.appearance.clone();
    wrong.data = EntityData::Material {
        density_kg_m3: None,
        color_rgba: [0; 4],
        section_hatch: None,
    };
    let wrong = original.edited(&[Mutation::Replace(wrong)]).unwrap();
    assert!(resolve(&wrong, f.parts[0]).is_err());
    assert!(
        diagnostics(&wrong)
            .unwrap()
            .iter()
            .any(|d| d.kind == DiagnosticKind::WrongKind)
    );
}
#[test]
fn stock_mismatch_invalid_units_duplicates_and_stale_foreign_previews_fail_atomically() {
    let mut f = Fixture::new();
    let before = f.s.document().clone();
    let assignment = Assignment {
        material: None,
        ..f.assignment()
    };
    assert!(prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts, &assignment).is_err());
    assert_eq!(f.s.document(), &before);
    for spec in [
        StockKind::Sheet {
            thickness_mm: -1.,
            length_mm: 10.,
            width_mm: 10.,
            inside_bend_radius_mm: 1.,
            k_factor: 0.5,
        },
        StockKind::Print {
            process: "FDM".into(),
            nozzle_mm: Some(0.),
            layer_height_mm: 0.2,
        },
    ] {
        assert!(
            LibraryDefinition::Stock(Stock {
                material: f.material.id,
                specification: spec
            })
            .entity(EntityId::new(), "Invalid stock")
            .is_err()
        );
    }
    let mut duplicate = f.style.clone();
    duplicate.id = EntityId::new();
    assert!(prepare_library(&f.s, f.s.revision(), actor(), &[duplicate]).is_err());
    let p = prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts, &f.assignment()).unwrap();
    let mut foreign = Session::new(before.clone()).unwrap();
    assert_eq!(
        foreign.commit(p.clone()).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    f.assign();
    assert_eq!(f.s.commit(p).unwrap_err().code, ErrorCode::StaleRevision);
}
#[test]
fn tags_multiselect_exact_queries_rename_and_undo() {
    let mut f = Fixture::new();
    let before = f.s.document().clone();
    let p = prepare_tags(
        &f.s,
        f.s.revision(),
        actor(),
        &f.parts,
        &TagEdit::Add(tags(&["Cabinet", "cnc"])),
    )
    .unwrap();
    assert_eq!(f.s.document(), &before);
    f.s.commit(p).unwrap();
    let q = TagQuery {
        all: tags(&["Cabinet"]),
        any: tags(&["cnc", "print"]),
        none: tags(&["omit"]),
    };
    assert_eq!(q.select(f.s.document()).unwrap().len(), 2);
    assert!(
        TagQuery {
            all: tags(&["cabinet"]),
            ..Default::default()
        }
        .select(f.s.document())
        .unwrap()
        .is_empty()
    );
    let mut e = f.s.document().get(f.parts[0]).unwrap().clone();
    e.name = "Renamed displayed part".into();
    commit(&mut f.s, vec![Mutation::Replace(e)]);
    assert_eq!(q.select(f.s.document()).unwrap().len(), 2);
    let p = prepare_tags(
        &f.s,
        f.s.revision(),
        actor(),
        &f.parts[..1],
        &TagEdit::Replace(tags(&["omit"])),
    )
    .unwrap();
    f.s.commit(p).unwrap();
    assert_eq!(q.select(f.s.document()).unwrap(), vec![f.parts[1]]);
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(q.select(f.s.document()).unwrap().len(), 2);
    assert!(
        prepare_tags(
            &f.s,
            f.s.revision(),
            actor(),
            &f.parts,
            &TagEdit::Add(tags(&[" invalid "]))
        )
        .is_err()
    );
}
#[test]
fn definition_links_cannot_diverge_from_source_and_legacy_documents_stay_unchanged() {
    let mut f = Fixture::new();
    let mut e = f.stock.clone();
    if let EntityData::Extension { inputs, .. } = &mut e.data {
        inputs.clear();
    }
    assert!(prepare_library(&f.s, f.s.revision(), actor(), &[e]).is_err());
    let legacy = Document::new("Empty legacy").unwrap();
    assert!(
        !ProjectFile::new(legacy.clone())
            .unwrap()
            .capabilities()
            .contains(CAPABILITY)
    );
    let mut renamed = f.material.clone();
    renamed.name = "Display rename".into();
    assert_eq!(
        fingerprint(&renamed).unwrap(),
        fingerprint(&f.material).unwrap()
    );
    library(&mut f.s, &[renamed]);
}

#[test]
fn every_stock_family_is_reusable_and_changed_material_requires_acknowledgement() {
    let mut f = Fixture::new();
    f.assign();
    let specs = [
        StockKind::Sheet {
            thickness_mm: 1.5,
            length_mm: 2000.,
            width_mm: 1000.,
            inside_bend_radius_mm: 2.,
            k_factor: 0.42,
        },
        StockKind::Profile {
            designation: "RHS 30×20×2".into(),
            length_mm: 6000.,
            width_mm: 30.,
            height_mm: 20.,
            wall_mm: Some(2.),
            shape: ProfileShape::Rectangular,
        },
        StockKind::Print {
            process: "Resin".into(),
            nozzle_mm: None,
            layer_height_mm: 0.05,
        },
    ];
    for specification in specs {
        let entry = LibraryDefinition::Stock(Stock {
            material: f.material.id,
            specification: specification.clone(),
        })
        .entity(EntityId::new(), "Reusable stock")
        .unwrap();
        library(&mut f.s, std::slice::from_ref(&entry));
        let assignment = Assignment {
            stock: Some(entry.id),
            ..f.assignment()
        };
        let p = prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts, &assignment).unwrap();
        f.s.commit(p).unwrap();
        let resolver = Resolver::new(f.s.document()).unwrap();
        for part in &f.parts {
            assert_eq!(
                resolver
                    .resolve(*part)
                    .unwrap()
                    .stock
                    .unwrap()
                    .1
                    .specification,
                specification
            );
        }
    }
    let mut material = f.material.clone();
    if let EntityData::Material { density_kg_m3, .. } = &mut material.data {
        *density_kg_m3 = Some(710.);
    }
    library(&mut f.s, &[material]);
    assert!(
        diagnostics(f.s.document())
            .unwrap()
            .iter()
            .any(|d| d.target == f.material.id && d.kind == DiagnosticKind::DefinitionChanged)
    );
    let assignment = Assignment::from_part(f.s.document(), f.parts[0]).unwrap();
    let p = prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts, &assignment).unwrap();
    f.s.commit(p).unwrap();
    assert!(
        !diagnostics(f.s.document())
            .unwrap()
            .iter()
            .any(|d| d.target == f.material.id && d.kind == DiagnosticKind::DefinitionChanged)
    );
}

#[test]
fn explicit_part_overrides_do_not_depend_on_broken_unused_material_defaults() {
    let mut f = Fixture::new();
    f.assign();
    let appearance = LibraryDefinition::Appearance(Appearance::default())
        .entity(EntityId::new(), "Override appearance")
        .unwrap();
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        pattern: HatchPattern::Cross,
        ..Default::default()
    })
    .entity(EntityId::new(), "Override hatch")
    .unwrap();
    library(&mut f.s, &[appearance.clone(), hatch.clone()]);
    let assignment = Assignment {
        appearance: Some(appearance.id),
        hatch: Some(hatch.id),
        ..f.assignment()
    };
    let p = prepare_assignment(&f.s, f.s.revision(), actor(), &f.parts[..1], &assignment).unwrap();
    f.s.commit(p).unwrap();
    commit(
        &mut f.s,
        vec![
            Mutation::Remove(f.appearance.id),
            Mutation::Remove(f.hatch.id),
        ],
    );
    assert!(
        diagnostics(f.s.document())
            .unwrap()
            .iter()
            .any(|d| d.owner == f.style.id && d.kind == DiagnosticKind::Missing)
    );
    let effective = resolve(f.s.document(), f.parts[0]).unwrap();
    assert_eq!(effective.appearance, Appearance::default());
    assert_eq!(effective.hatch.unwrap().pattern, HatchPattern::Cross);
    assert!(resolve(f.s.document(), f.parts[1]).is_err());
}

#[test]
fn metadata_changes_invalidate_owner_parts_and_drawing_consumers_without_dependency_cycles() {
    use multicad_core::document::{Definition, Link};
    let mut f = Fixture::new();
    f.assign();
    let drawing = Entity::new(
        "Cabinet drawing",
        EntityData::Drawing {
            definition: Definition {
                type_id: "fixture.drawing".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: f
                .parts
                .iter()
                .map(|id| Link::new(*id, "view_source"))
                .collect(),
        },
    );
    commit(&mut f.s, vec![Mutation::Insert(drawing.clone())]);
    for mut changed in [f.appearance.clone(), f.hatch.clone()] {
        if let Some(mut definition) = read(&changed).unwrap() {
            match &mut definition {
                LibraryDefinition::Appearance(a) => a.roughness = 0.3,
                LibraryDefinition::Hatch(h) => h.angle_deg = 60.,
                _ => unreachable!(),
            };
            changed = definition.entity(changed.id, changed.name).unwrap();
        }
        let change =
            f.s.apply(Request {
                expected_revision: f.s.revision(),
                actor: actor(),
                label: "Change section/appearance".into(),
                edits: vec![Mutation::Replace(changed.clone())],
            })
            .unwrap();
        for id in [f.material.id, f.parts[0], f.parts[1], drawing.id] {
            assert!(
                change.invalidated.contains(&id),
                "metadata did not reach {id}"
            );
        }
        let canonical = f.s.document().affected([changed.id]).unwrap();
        assert!(canonical.contains(&f.style.id));
        assert!(!canonical.contains(&f.material.id));
        assert!(!canonical.contains(&drawing.id));
        assert_eq!(
            f.s.document().topological_order().unwrap().len(),
            f.s.document().entities().count()
        );
    }
    let missing =
        f.s.document()
            .edited(&[Mutation::Remove(f.material.id)])
            .unwrap();
    assert!(
        missing
            .invalidated([f.appearance.id])
            .unwrap()
            .contains(&drawing.id)
    );
}
#[test]
fn removing_or_reassigning_associations_invalidates_old_and_new_owners() {
    use multicad_core::document::{Definition, Link};
    let mut f = Fixture::new();
    let properties = LibraryDefinition::PartProperties(PartProperties {
        part: f.parts[0],
        material: None,
        stock: None,
        appearance: None,
        hatch: None,
    })
    .entity(EntityId::new(), "Properties")
    .unwrap();
    let drawings: Vec<_> = f
        .parts
        .iter()
        .map(|part| {
            Entity::new(
                "Part drawing",
                EntityData::Drawing {
                    definition: Definition {
                        type_id: "fixture.drawing".into(),
                        schema_version: 1,
                        data: serde_json::json!({}),
                    },
                    inputs: vec![Link::new(*part, "source")],
                },
            )
        })
        .collect();
    let mut edits = vec![Mutation::Insert(properties.clone())];
    edits.extend(drawings.iter().cloned().map(Mutation::Insert));
    commit(&mut f.s, edits);
    let LibraryDefinition::PartProperties(mut p) = read(&properties).unwrap().unwrap() else {
        unreachable!()
    };
    p.part = f.parts[1];
    let moved = LibraryDefinition::PartProperties(p)
        .entity(properties.id, "Properties")
        .unwrap();
    let change =
        f.s.apply(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Reassign owner".into(),
            edits: vec![Mutation::Replace(moved)],
        })
        .unwrap();
    for id in f.parts.iter().copied().chain(drawings.iter().map(|d| d.id)) {
        assert!(
            change.invalidated.contains(&id),
            "old/new owner missing {id}"
        );
    }
    let change =
        f.s.apply(Request {
            expected_revision: f.s.revision(),
            actor: actor(),
            label: "Remove properties".into(),
            edits: vec![Mutation::Remove(properties.id)],
        })
        .unwrap();
    assert!(change.invalidated.contains(&f.parts[1]));
    assert!(change.invalidated.contains(&drawings[1].id));
}

#[test]
fn inline_and_unknown_legacy_hatches_are_versioned_or_diagnosed_without_erasing_source() {
    use multicad_core::document::Definition;
    let mut f = Fixture::new();
    let inline = Definition {
        type_id: "materials.hatch.inline".into(),
        schema_version: 1,
        data: serde_json::to_value(HatchStyle::default()).unwrap(),
    };
    let mut material = f.material.clone();
    if let EntityData::Material { section_hatch, .. } = &mut material.data {
        *section_hatch = Some(inline.clone());
    }
    library(&mut f.s, std::slice::from_ref(&material));
    let assignment = Assignment {
        material: Some(material.id),
        stock: None,
        appearance: None,
        hatch: None,
    };
    let mut bare = f.s.document().clone();
    for entity in f.s.document().entities() {
        if read(entity).unwrap().is_some() {
            bare.remove(entity.id).unwrap();
        }
    }
    assert!(
        ProjectFile::new(bare.clone())
            .unwrap()
            .capabilities()
            .contains(CAPABILITY)
    );
    let mut session = Session::new(bare).unwrap();
    let p = prepare_assignment(&session, 0, actor(), &f.parts[..1], &assignment).unwrap();
    session.commit(p).unwrap();
    assert_eq!(
        resolve(session.document(), f.parts[0]).unwrap().hatch,
        Some(HatchStyle::default())
    );
    let mut unsupported = material.clone();
    if let EntityData::Material { section_hatch, .. } = &mut unsupported.data {
        *section_hatch = Some(Definition {
            type_id: "external.hatch".into(),
            schema_version: 1,
            data: serde_json::json!({"preserved":true}),
        });
    }
    let doc = session
        .document()
        .edited(&[Mutation::Replace(unsupported.clone())])
        .unwrap();
    assert_eq!(doc.get(unsupported.id).unwrap(), &unsupported);
    assert!(
        diagnostics(&doc)
            .unwrap()
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnsupportedLegacyHatch)
    );
    assert!(resolve(&doc, f.parts[0]).is_err());
}

#[test]
fn stock_without_material_names_the_prerequisite_instead_of_a_self_cycle() {
    let mut s = Session::new(Document::new("Empty library").unwrap()).unwrap();
    let before = s.document().clone();
    let id = EntityId::new();
    let panel = StockKind::Panel {
        thickness_mm: 18.,
        length_mm: 2440.,
        width_mm: 1220.,
        grain: Grain::AlongLength,
    };
    let stock = |material| {
        LibraryDefinition::Stock(Stock {
            material,
            specification: panel.clone(),
        })
        .entity(id, "Birch 18")
        .unwrap()
    };
    let error = prepare_library(&s, 0, actor(), &[stock(id)]).unwrap_err();
    assert!(
        error.message.contains("Stock 'Birch 18'")
            && error.message.contains("manufacturing material")
            && !error.message.contains("cycle"),
        "{}",
        error.message
    );
    let missing = EntityId::new();
    let error = prepare_library(&s, 0, actor(), &[stock(missing)]).unwrap_err();
    assert!(error.message.contains("requires a manufacturing material"));
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), 0);
    // Material and stock in one batch is the safe continuation.
    let mut material = Entity::new(
        "Birch",
        EntityData::Material {
            density_kg_m3: Some(680.),
            color_rgba: [200, 180, 150, 255],
            section_hatch: None,
        },
    );
    material.id = EntityId::new();
    library(&mut s, &[material.clone(), stock(material.id)]);
    assert!(s.document().get(id).is_some() && s.document().get(material.id).is_some());
    s.undo(1, actor()).unwrap();
    assert_eq!(s.document(), &before);
}

#[test]
fn exact_component_material_resolves_shared_style_without_part_overrides() {
    let mut f = Fixture::new();
    f.assign();
    let resolver = Resolver::new(f.s.document()).unwrap();
    let direct = resolver.resolve_material(f.material.id).unwrap();
    assert_eq!(direct.material, Some(f.material.id));
    assert_eq!(direct.stock, None);
    assert_eq!(direct.appearance.color_rgba, [200, 165, 110, 255]);
    assert_eq!(direct.hatch, Some(HatchStyle::default()));
    assert!(resolver.resolve_material(f.parts[0]).is_err());
    assert!(resolver.resolve_material(EntityId::new()).is_err());
}
