use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    drawings::{self, Page, PageAction},
    exchange::*,
    materials::{self, TagEdit, TagQuery},
    project_file::{self, ProjectFile},
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn tags(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| (*s).into()).collect()
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Export fixture edit".into(),
        edits,
    })
    .unwrap();
}
fn parts() -> (Session, Vec<EntityId>) {
    let mut d = Document::new("Named outputs").unwrap();
    let mut ids = vec![];
    for (name, t) in [
        ("Panel", vec!["cnc"]),
        ("Shelf", vec!["wood"]),
        ("Spare", vec!["cnc", "omit"]),
    ] {
        let e = Entity::new(
            name,
            EntityData::Part {
                features: vec![],
                material: None,
                tags: tags(&t),
            },
        );
        ids.push(e.id);
        d.insert(e).unwrap();
    }
    (Session::new(d).unwrap(), ids)
}
fn save(s: &mut Session, definition: &SetDefinition) -> EntityId {
    let id = EntityId::new();
    let p = prepare_set(s, s.revision(), actor(), id, "Shop exports", definition).unwrap();
    s.commit(p).unwrap();
    id
}
#[test]
fn dynamic_tag_sets_reresolve_after_tag_part_and_display_name_changes_and_reopen() {
    let (mut s, ids) = parts();
    let definition = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Project,
            tags: Some(TagQuery {
                all: tags(&["cnc"]),
                none: tags(&["omit"]),
                ..Default::default()
            }),
        },
        ..Default::default()
    };
    let id = save(&mut s, &definition);
    let first = resolve(s.document(), s.revision(), id).unwrap();
    assert_eq!(
        first
            .parts
            .iter()
            .map(|p| p.reference.part)
            .collect::<Vec<_>>(),
        vec![ids[0]]
    );
    let p = materials::prepare_tags(
        &s,
        s.revision(),
        actor(),
        &ids[1..2],
        &TagEdit::Add(tags(&["cnc"])),
    )
    .unwrap();
    s.commit(p).unwrap();
    assert_eq!(
        resolve(s.document(), s.revision(), id).unwrap().parts.len(),
        2
    );
    let e = Entity::new(
        "Added part",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: tags(&["cnc"]),
        },
    );
    apply(&mut s, vec![Mutation::Insert(e)]);
    assert_eq!(
        resolve(s.document(), s.revision(), id).unwrap().parts.len(),
        3
    );
    let mut renamed = s.document().get(ids[0]).unwrap().clone();
    renamed.name = "Invisible/name changes do not select by name".into();
    apply(&mut s, vec![Mutation::Replace(renamed)]);
    assert_eq!(
        resolve(s.document(), s.revision(), id).unwrap().parts.len(),
        3
    );
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
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
    assert_eq!(
        resolve(&opened.project.document, 0, id).unwrap().parts,
        resolve(s.document(), s.revision(), id).unwrap().parts
    );
    assert!(first.stamp.check(s.document(), s.revision()).is_err());
}
#[test]
fn set_preparation_is_source_neutral_stable_and_one_undo_and_foreign_stale_guards_hold() {
    let (mut s, _) = parts();
    let before = s.document().clone();
    let id = EntityId::new();
    let definition = SetDefinition::default();
    let recipe = definition.recipes[0].id;
    let p = prepare_set(&s, 0, actor(), id, "Outputs", &definition).unwrap();
    let again = prepare_set(&s, 0, actor(), id, "Outputs", &definition).unwrap();
    assert_eq!(p.document(), again.document());
    assert_eq!(s.document(), &before);
    let mut foreign = Session::new(before.clone()).unwrap();
    assert_eq!(
        foreign.commit(p.clone()).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    s.commit(p).unwrap();
    assert_eq!(read(s.document(), id).unwrap().recipes[0].id, recipe);
    assert_eq!(s.commit(again).unwrap_err().code, ErrorCode::StaleRevision);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(read(s.document(), id).unwrap(), definition);
}
#[test]
fn explicit_missing_roots_remain_repairable_and_never_broaden_scope() {
    let (mut s, ids) = parts();
    let definition = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: vec![ids[1]],
            },
            tags: None,
        },
        ..Default::default()
    };
    let set = save(&mut s, &definition);
    apply(&mut s, vec![Mutation::Remove(ids[1])]);
    assert!(
        s.document()
            .reference_diagnostics()
            .iter()
            .any(|d| d.owner == set && d.target == ids[1])
    );
    assert_eq!(
        resolve(s.document(), s.revision(), set).unwrap_err().kind,
        ErrorKind::Missing
    );
    let mut invalid = definition.clone();
    invalid.scope.root = ScopeRoot::Assembly { assembly: ids[0] };
    assert!(
        prepare_set(
            &s,
            s.revision(),
            actor(),
            EntityId::new(),
            "Wrong assembly",
            &invalid
        )
        .is_err()
    );
}
#[test]
fn assembly_occurrence_paths_preserve_multiple_copies_and_explicit_root_plus_copy() {
    let (mut s, ids) = parts();
    let mut occurrences = vec![];
    for name in ["Left occurrence", "Right occurrence"] {
        let data:EntityData=serde_json::from_value(serde_json::json!({"kind":"instance","component":ids[0],"placement":Placement::default()})).unwrap();
        let e = Entity::new(name, data);
        occurrences.push(e.id);
        apply(&mut s, vec![Mutation::Insert(e)]);
    }
    let assembly = Entity::new(
        "Cabinet",
        EntityData::Assembly {
            instances: occurrences.clone(),
        },
    );
    let assembly_id = assembly.id;
    apply(&mut s, vec![Mutation::Insert(assembly)]);
    let definition = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Assembly {
                assembly: assembly_id,
            },
            tags: Some(TagQuery {
                any: tags(&["cnc"]),
                ..Default::default()
            }),
        },
        ..Default::default()
    };
    let id = save(&mut s, &definition);
    let result = resolve(s.document(), s.revision(), id).unwrap();
    assert_eq!(result.parts.len(), 2);
    assert!(
        result
            .parts
            .iter()
            .all(|p| p.reference.part == ids[0] && p.reference.root_assembly == Some(assembly_id))
    );
    assert_ne!(
        result.parts[0].reference.occurrence_path,
        result.parts[1].reference.occurrence_path
    );
    let explicit = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: vec![ids[0], occurrences[0]],
            },
            tags: None,
        },
        ..Default::default()
    };
    let id = save(&mut s, &explicit);
    let resolved = resolve(s.document(), s.revision(), id).unwrap();
    assert_eq!(resolved.parts.len(), 2);
    assert!(
        resolved
            .parts
            .iter()
            .any(|p| p.reference.occurrence_path.is_empty())
    );
}
#[test]
fn selected_pages_keep_order_and_nested_identity_through_renumber_and_delete() {
    let (mut s, _) = parts();
    let first = Page::new("1");
    let first_id = first.id;
    let (book, p) =
        drawings::prepare_create(&s, s.revision(), actor(), "Plans", Some(first)).unwrap();
    s.commit(p).unwrap();
    let second = Page::new("2");
    let second_id = second.id;
    let p = drawings::prepare(&s, s.revision(), actor(), book, PageAction::Add(second)).unwrap();
    s.commit(p).unwrap();
    let definition = SetDefinition {
        pages: Pages::Selected {
            pages: vec![
                PageRef {
                    book,
                    page: second_id,
                },
                PageRef {
                    book,
                    page: first_id,
                },
            ],
        },
        ..Default::default()
    };
    let id = save(&mut s, &definition);
    let r = resolve(s.document(), s.revision(), id).unwrap();
    assert_eq!(r.pages[0].reference.page, second_id);
    let mut page = drawings::read(s.document(), book).unwrap().pages[0].clone();
    page.number = "A-101".into();
    let p = drawings::prepare(&s, s.revision(), actor(), book, PageAction::Replace(page)).unwrap();
    s.commit(p).unwrap();
    assert_eq!(
        resolve(s.document(), s.revision(), id).unwrap().pages[1].number,
        "A-101"
    );
    let p = drawings::prepare(
        &s,
        s.revision(),
        actor(),
        book,
        PageAction::Remove(second_id),
    )
    .unwrap();
    s.commit(p).unwrap();
    assert_eq!(
        resolve(s.document(), s.revision(), id).unwrap_err().kind,
        ErrorKind::Missing
    );
}
#[test]
fn invalid_empty_queries_quality_and_naming_are_rejected_but_empty_matches_are_inspectable() {
    let (mut s, _) = parts();
    let mut d = SetDefinition::default();
    d.scope.tags = Some(TagQuery::default());
    assert!(prepare_set(&s, 0, actor(), EntityId::new(), "Ambiguous all", &d).is_err());
    d.scope.tags = Some(TagQuery {
        all: tags(&["absent"]),
        ..Default::default()
    });
    let id = save(&mut s, &d);
    assert!(
        resolve(s.document(), s.revision(), id)
            .unwrap()
            .parts
            .is_empty()
    );
    for template in ["../{part}", "{unknown}", "{part", "{part}}"] {
        d.naming.template = template.into();
        assert!(d.validate().is_err());
    }
    d = SetDefinition::default();
    d.recipes[0].output = RecipeOutput::Mesh {
        format: MeshFormat::Stl,
        units: LengthUnit::Millimeter,
        hierarchy: HierarchyMode::SeparateParts,
        names: NameHandling::FileNamesOnly,
        aids: AidPolicy::IncludeAll,
        quality: MeshQuality {
            chord_deviation_mm: 0.,
            ..Default::default()
        },
    };
    assert!(d.validate().is_err());
}
#[test]
fn snapshot_guard_rejects_modified_same_id_revision_and_capability_survives_file_roundtrip() {
    let (mut s, _) = parts();
    let id = save(&mut s, &Default::default());
    let r = resolve(s.document(), s.revision(), id).unwrap();
    let changed = s
        .document()
        .edited(&[Mutation::RenameDocument(
            "Different canonical source".into(),
        )])
        .unwrap();
    assert_eq!(
        r.stamp.check(&changed, s.revision()).unwrap_err().kind,
        ErrorKind::Stale
    );
    let file = ProjectFile::new(s.document().clone()).unwrap();
    assert!(file.capabilities().contains(CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let mut unsupported = project_file::supported_capabilities();
    unsupported.remove(CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &unsupported).is_err());
}

#[test]
fn filenames_are_stable_platform_safe_and_collisions_get_short_identity_suffixes() {
    let (mut s, ids) = parts();
    let mut d = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: vec![ids[0]],
            },
            tags: None,
        },
        ..Default::default()
    };
    d.naming.template = "{part}".into();
    let mut entity = s.document().get(ids[0]).unwrap().clone();
    entity.name = "CON".into();
    apply(&mut s, vec![Mutation::Replace(entity)]);
    let set = save(&mut s, &d);
    let plan = resolve(s.document(), s.revision(), set).unwrap();
    let files = filenames(&plan, s.document().name()).unwrap();
    assert_eq!(files[0].filename, "_CON.step");
    assert_eq!(files, filenames(&plan, s.document().name()).unwrap());
    // Names that differ only in case would overwrite each other on macOS
    // and Windows: both get a short identity suffix instead.
    let mut entity = s.document().get(ids[1]).unwrap().clone();
    entity.name = "con".into();
    apply(&mut s, vec![Mutation::Replace(entity)]);
    d.scope.root = ScopeRoot::Explicit {
        entities: ids[..2].to_vec(),
    };
    let set = save(&mut s, &d);
    let plan = resolve(s.document(), s.revision(), set).unwrap();
    let files = filenames(&plan, s.document().name()).unwrap();
    let mut names: Vec<_> = files.iter().map(|f| f.filename.as_str()).collect();
    names.sort();
    assert!(
        names[0].starts_with("_CON_") && names[1].starts_with("_con_"),
        "{names:?}"
    );
    for name in &names {
        let suffix = name.trim_end_matches(".step").rsplit('_').next().unwrap();
        assert_eq!(suffix.len(), 6, "{name}");
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()), "{name}");
    }
    assert_ne!(names[0].to_ascii_lowercase(), names[1].to_ascii_lowercase());
    // The suffix depends on identity, not order.
    let mut reversed = plan.clone();
    reversed.parts.reverse();
    let again = filenames(&reversed, s.document().name()).unwrap();
    for file in &files {
        assert!(again.contains(file), "{file:?} moved: {again:?}");
    }
    // The readable default names the project and the Part only.
    d.naming = Default::default();
    let set = save(&mut s, &d);
    let files = filenames(
        &resolve(s.document(), s.revision(), set).unwrap(),
        "Project",
    )
    .unwrap();
    let mut names: Vec<_> = files.iter().map(|f| f.filename.as_str()).collect();
    names.sort();
    assert!(
        names[0].starts_with("Project_CON_") && names[1].starts_with("Project_con_"),
        "{names:?}"
    );
}
#[test]
fn readable_default_names_suffix_only_duplicates_of_parts_and_recipes() {
    let (mut s, ids) = parts();
    // Two Parts with the same display name and two STEP recipes.
    let mut twin = s.document().get(ids[1]).unwrap().clone();
    twin.name = "Panel".into();
    apply(&mut s, vec![Mutation::Replace(twin)]);
    let d = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: ids.clone(),
            },
            tags: None,
        },
        recipes: vec![Recipe::step("STEP"), Recipe::step("STEP")],
        ..Default::default()
    };
    let set = save(&mut s, &d);
    let plan = resolve(s.document(), s.revision(), set).unwrap();
    let files = filenames(&plan, "Named outputs").unwrap();
    assert_eq!(files.len(), 6);
    let names: BTreeSet<_> = files
        .iter()
        .map(|f| f.filename.to_ascii_lowercase())
        .collect();
    assert_eq!(names.len(), 6, "no two outputs share a name: {files:?}");
    for f in &files {
        assert!(f.filename.starts_with("Named_outputs_"), "{}", f.filename);
        assert!(f.filename.ends_with(".step"));
        // Readable stem plus at most one short suffix: no raw UUIDs.
        assert!(
            f.filename.len() <= "Named_outputs_Spare_123456.step".len(),
            "{}",
            f.filename
        );
    }
    // A single recipe over uniquely named Parts needs no suffix at all.
    let mut single = d.clone();
    single.recipes.truncate(1);
    single.scope.root = ScopeRoot::Explicit {
        entities: vec![ids[0], ids[2]],
    };
    let set = save(&mut s, &single);
    let files = filenames(
        &resolve(s.document(), s.revision(), set).unwrap(),
        "Named outputs",
    )
    .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|f| f.filename.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["Named_outputs_Panel.step", "Named_outputs_Spare.step"])
    );
}
#[test]
fn pdf_recipe_plans_one_ordered_page_bundle_and_other_recipes_keep_individual_identity() {
    let (mut s, _) = parts();
    let a = Page::new("A");
    let aid = a.id;
    let (book, p) = drawings::prepare_create(&s, s.revision(), actor(), "Plans", Some(a)).unwrap();
    s.commit(p).unwrap();
    let b = Page::new("B");
    let bid = b.id;
    let p = drawings::prepare(&s, s.revision(), actor(), book, PageAction::Add(b)).unwrap();
    s.commit(p).unwrap();
    let d = SetDefinition {
        pages: Pages::Selected {
            pages: vec![PageRef { book, page: bid }, PageRef { book, page: aid }],
        },
        recipes: vec![Recipe {
            id: EntityId::new(),
            name: "Drawing package".into(),
            output: RecipeOutput::Pdf,
        }],
        ..Default::default()
    };
    let set = save(&mut s, &d);
    let plan = resolve(s.document(), s.revision(), set).unwrap();
    let files = filenames(&plan, s.document().name()).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        matches!(&files[0].subject,OutputSubject::Pages(pages) if pages==&vec![PageRef{book,page:bid},PageRef{book,page:aid}])
    );
}
#[test]
fn differently_named_recipes_of_one_format_are_told_apart_by_recipe_name() {
    let (mut s, ids) = parts();
    let d = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit {
                entities: vec![ids[0]],
            },
            tags: None,
        },
        recipes: vec![Recipe::step("Rough"), Recipe::step("Final")],
        ..Default::default()
    };
    let set = save(&mut s, &d);
    let files = filenames(
        &resolve(s.document(), s.revision(), set).unwrap(),
        "Named outputs",
    )
    .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|f| f.filename.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "Named_outputs_Panel_Rough.step",
            "Named_outputs_Panel_Final.step"
        ])
    );
}
