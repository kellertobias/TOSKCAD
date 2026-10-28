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
