//! A wood stock envelope has real placement geometry and no manufacturing claim.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    exchange::*,
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, PrimitiveFrame, ScrewDrive,
        ScrewHead, ScrewLengthInput, WoodPointIntent, WoodScrewParameters, WoodScrewRepresentation,
        WoodVReference, length,
    },
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, EvaluationCache, export::Exporter};
use std::{collections::BTreeMap, f64::consts::PI};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn session() -> Session {
    Session::new(Document::new("Wood envelope native proof").unwrap()).unwrap()
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn wood(d: f64, l: f64, af: f64, k: f64, pitch: f64, core: f64) -> MechanicalPartParameters {
    MechanicalPartParameters::WoodScrew {
        parameters: Box::new(WoodScrewParameters {
            length_input: ScrewLengthInput::Underhead { value: length(l) },
            nominal_diameter: length(d),
            head: ScrewHead::ExternalHex {
                across_flats: length(af),
                height: length(k),
            },
            drive: ScrewDrive::ExternalHex,
            reference_profile: WoodVReference {
                pitch: length(pitch),
                core_diameter: length(core),
                included_angle: model::angle(60.),
            },
            minimum_tip_inclusive_thread_extent: length(0.6 * l),
            point: WoodPointIntent::ManufacturerUnspecified,
            representation: WoodScrewRepresentation::SimplifiedEnvelope,
            frame: Default::default(),
        }),
    }
}
fn add(s: &mut Session, p: MechanicalPartParameters) -> MechanicalPartRequest {
    let r = MechanicalPartRequest::new("DIN571-related wood envelope", p);
    s.commit(
        model::prepare_mechanical_part(s, s.revision(), actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    r
}
fn print(parts: Vec<EntityId>) -> PrintRequest {
    PrintRequest {
        source: PrintSource::Parts { parts },
        aids: AidPolicy::IncludeAll,
        quality: MeshQuality::default(),
        stl_per_part: true,
    }
}
fn probe(s: &Solid, p: [f64; 3], inside: bool) {
    let d = 0.002;
    let q = Solid::box_solid([d; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: p.map(|v| v - d / 2.),
            ..Default::default()
        })
        .unwrap();
    let fraction = s
        .boolean(&q, BooleanOperation::Intersect, GeometryPolicy::default())
        .unwrap()
        .map(|s| s.volume_mm3().unwrap())
        .unwrap_or(0.)
        / d.powi(3);
    assert!(
        if inside {
            fraction > 0.999
        } else {
            fraction < 0.001
        },
        "{p:?}: {fraction}, expected material {inside}"
    );
}
fn recipes() -> Vec<Recipe> {
    let mut rows = vec![Recipe::step("STEP")];
    for format in [
        MeshFormat::Stl,
        MeshFormat::Obj,
        MeshFormat::Glb,
        MeshFormat::ThreeMf,
    ] {
        rows.push(Recipe {
            id: EntityId::new(),
            name: format!("{format:?}"),
            output: RecipeOutput::Mesh {
                format,
                units: if format == MeshFormat::Glb {
                    LengthUnit::Meter
                } else {
                    LengthUnit::Millimeter
                },
                hierarchy: HierarchyMode::SeparateParts,
                names: NameHandling::FileNamesOnly,
                aids: AidPolicy::IncludeAll,
                quality: Default::default(),
            },
        });
    }
    rows.push(Recipe {
        id: EntityId::new(),
        name: "Manufacturing outline".into(),
        output: RecipeOutput::PlanarDxf {
            units: LengthUnit::Millimeter,
            layers: Default::default(),
            aids: AidPolicy::IncludeAll,
        },
    });
    rows
}
fn export_set(s: &mut Session, ids: Vec<EntityId>) -> EntityId {
    let id = EntityId::new();
    let set = SetDefinition {
        scope: Scope {
            root: ScopeRoot::Explicit { entities: ids },
            tags: None,
        },
        recipes: recipes(),
        ..Default::default()
    };
    s.commit(prepare_set(s, s.revision(), actor(), id, "Wood export refusal", &set).unwrap())
        .unwrap();
    id
}
fn assert_refused(s: &Session, set: EntityId, parts: Vec<EntityId>) {
    let before = s.document().clone();
    let exporter = Exporter::default();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(!preview.can_run());
    assert!(preview.artifacts.is_empty());
    assert_eq!(preview.recipes.len(), 6);
    assert!(preview.recipes.iter().all(|r|matches!(&r.status,RecipeStatus::Unsupported(reason) if reason.contains("wood")&&reason.contains("point"))),"{:?}",preview.recipes);
    let e = match exporter.print_preview(s.document(), s.revision(), &print(parts), &actor(), None)
    {
        Err(error) => error,
        Ok(_) => panic!("wood Print must refuse"),
    };
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert!(e.message.contains("wood") && e.message.contains("point"));
    assert_eq!(s.document(), &before);
}
#[test]
fn sourced_hex_stock_envelopes_have_real_bounds_flat_limit_planes_and_raw_step_readbacks() {
    let dir = tempfile::tempdir().unwrap();
    for (index, (d, l, af, k, pitch, core)) in [
        (4., 16., 7., 2.8, 1.8, 2.8),
        (6., 40., 10., 4., 2.6, 4.2),
        (10., 100., 17., 7., 4.5, 7.),
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = session();
        let r = add(&mut s, wood(d, l, af, k, pitch, core));
        let built = compiler().compile(s.document(), s.revision()).unwrap();
        let solid = built.solid(r.feature).unwrap();
        let expected = 3_f64.sqrt() / 2. * af * af * k + PI * (d / 2.).powi(2) * l;
        assert!((solid.volume_mm3().unwrap() - expected).abs() < 1e-7 * expected);
        assert_eq!(solid.solid_count().unwrap(), 1);
        let b = solid.bounds().unwrap();
        for (a, e) in b.min.into_iter().chain(b.max).zip([
            -af / 2.,
            -af / 3_f64.sqrt(),
            -k,
            af / 2.,
            af / 3_f64.sqrt(),
            l,
        ]) {
            assert!((a - e).abs() < 2e-7);
        }
        let topology = solid.topology_snapshot(r.feature, s.revision()).unwrap();
        let faces = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
            .collect::<Vec<_>>();
        assert_eq!(faces.len(), 10);
        assert!(faces.iter().all(|e| {
            !e.origins.is_empty()
                && e.origins.iter().all(|o| {
                    o.source == r.feature
                        && (o.role.starts_with("wood.envelope.")
                            || o.role.starts_with(
                                "side/d067b697-a074-4bc7-8ae8-4c05d18b3a90/wood.envelope.",
                            ))
                })
        }));
        let path = dir.path().join(format!("declared-envelope-{index}.step"));
        solid.write_step(&path).unwrap();
        let step = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert!((step.volume_mm3().unwrap() - expected).abs() < 1e-7 * expected);
        assert_eq!(step.solid_count().unwrap(), 1);
        for actual in [solid, &step] {
            for z in [l * 0.4, l * 0.6, l - 0.01] {
                probe(actual, [d / 2. - 0.01, 0., z], true);
            }
            probe(actual, [0., 0., l + 0.01], false);
            probe(actual, [d / 2. + 0.01, 0., l - 0.01], false);
        }
        // A full-radius section close to the distal plane proves this is the
        // stated stock envelope, not a recovered cone or threaded point.
        assert!(
            model::cosmetic_thread_in_history(s.document(), r.feature)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            model::manufacturing_limitation_in_history(s.document(), r.feature)
                .unwrap()
                .unwrap()
                .feature,
            r.feature
        );
    }
}
#[test]
fn edits_history_pose_and_reopen_preserve_envelope_identity_and_source_limits() {
    let mut s = session();
    let mut p = wood(6., 40., 10., 4., 2.6, 4.2);
    let MechanicalPartParameters::WoodScrew { parameters } = &mut p else {
        unreachable!()
    };
    parameters.frame = PrimitiveFrame {
        origin: [length(20.), length(-30.), length(10.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let r = add(&mut s, p.clone());
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    let mut changed = p.clone();
    let MechanicalPartParameters::WoodScrew { parameters } = &mut changed else {
        unreachable!()
    };
    parameters.length_input = ScrewLengthInput::Overall { value: length(54.) };
    parameters.minimum_tip_inclusive_thread_extent = length(30.);
    let prepared =
        model::prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &changed)
            .unwrap();
    let before = s.document().clone();
    let candidate = compiler()
        .compile(prepared.prepared.document(), s.revision() + 1)
        .unwrap();
    assert_eq!(s.document(), &before);
    assert!(
        candidate.solid(r.feature).unwrap().volume_mm3().unwrap()
            > first.solid(r.feature).unwrap().volume_mm3().unwrap()
    );
    s.commit(prepared.prepared).unwrap();
    let after = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert_eq!(
        model::read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .output
            .body,
        r.body
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(
        model::read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .parameters,
        p
    );
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(
        model::read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .parameters,
        changed
    );
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let fresh = Session::new(reopened.document).unwrap();
    let actual = compiler()
        .compile(fresh.document(), fresh.revision())
        .unwrap();
    assert_eq!(
        model::read_mechanical_part(fresh.document(), r.feature)
            .unwrap()
            .parameters,
        changed
    );
    assert!(
        (actual.solid(r.feature).unwrap().volume_mm3().unwrap()
            - after.solid(r.feature).unwrap().volume_mm3().unwrap())
        .abs()
            < 1e-7
    );
    assert_eq!(
        actual.solid(r.feature).unwrap().bounds().unwrap(),
        after.solid(r.feature).unwrap().bounds().unwrap()
    );
    let b = actual.solid(r.feature).unwrap().bounds().unwrap();
    assert!((b.min[0] - 16.).abs() < 2e-7 && (b.max[0] - 70.).abs() < 2e-7);
    assert!(
        model::manufacturing_limitation_in_history(fresh.document(), r.feature)
            .unwrap()
            .is_some()
    );
}
#[test]
fn all_common_manufacturing_and_print_refuse_wood_and_transformed_history_but_not_unrelated_parts()
{
    let mut s = session();
    let r = add(&mut s, wood(6., 40., 10., 4., 2.6, 4.2));
    let set = export_set(&mut s, vec![r.body]);
    assert_refused(&s, set, vec![r.body]);
    let transformed = model::TransformRequest::new(
        model::BodyInput {
            body: r.body,
            producer: r.feature,
        },
        model::NumericTransform {
            translation: [length(3.), length(4.), length(5.)],
            ..Default::default()
        },
    );
    s.commit(
        model::prepare_transform(&s, s.revision(), actor(), &transformed)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert_eq!(
        model::manufacturing_limitation_in_history(s.document(), transformed.feature)
            .unwrap()
            .unwrap()
            .feature,
        r.feature
    );
    assert_refused(&s, set, vec![r.body]);
    let washer = add(
        &mut s,
        MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(0.5),
            frame: Default::default(),
        },
    );
    let positive = EntityId::new();
    s.commit(
        prepare_set(
            &s,
            s.revision(),
            actor(),
            positive,
            "Unaffected exact washer",
            &SetDefinition {
                scope: Scope {
                    root: ScopeRoot::Explicit {
                        entities: vec![washer.body],
                    },
                    tags: None,
                },
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        Exporter::default()
            .preview(s.document(), s.revision(), positive, &actor(), None)
            .unwrap()
            .can_run()
    );
    assert!(
        Exporter::default()
            .print_preview(
                s.document(),
                s.revision(),
                &print(vec![washer.body]),
                &actor(),
                None
            )
            .unwrap()
            .can_write()
    );
}
#[test]
fn pinned_private_wood_occurrence_with_navigator_parent_remains_limited_after_cache_and_reopen() {
    let mut source = session();
    let r = add(&mut source, wood(6., 40., 10., 4., 2.6, 4.2));
    let author = SymbolAuthorRequest::new("Wood source", r.body, SymbolKind::Static, vec![], None);
    source
        .commit(
            symbols::prepare_author(&source, source.revision(), actor(), &author)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let definition = symbols::read_definition(source.document(), author.definition).unwrap();
    let mut s = session();
    let imported = symbols::prepare_import(&s, s.revision(), actor(), &definition).unwrap();
    let def = imported.definition;
    s.commit(imported.prepared).unwrap();
    let parent = Entity::new(
        "Parent assembly",
        EntityData::Assembly { instances: vec![] },
    );
    let parent_id = parent.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Create parent assembly".into(),
        edits: vec![Mutation::Insert(parent)],
    })
    .unwrap();
    let placed = SymbolPlaceRequest::new(
        s.document(),
        def,
        "Placed simplified wood",
        BTreeMap::new(),
        Placement {
            translation_mm: [20., 0., 0.],
            ..Default::default()
        },
    )
    .unwrap();
    s.commit(
        symbols::prepare_place_in(&s, s.revision(), actor(), &placed, Some(parent_id))
            .unwrap()
            .prepared,
    )
    .unwrap();
    let private = placed.ids[&r.feature];
    assert_eq!(
        model::manufacturing_limitation_in_history(s.document(), private)
            .unwrap()
            .unwrap()
            .feature,
        private
    );
    let set = export_set(&mut s, vec![placed.instance]);
    let mut cache = EvaluationCache::default();
    for _ in 0..2 {
        let m = compiler()
            .compile_cached(&mut cache, s.document(), s.revision(), None)
            .unwrap();
        assert_eq!(m.snapshot().leaf_occurrences.len(), 1);
        let leaf = m.snapshot().leaf_occurrences.values().next().unwrap();
        assert_eq!(leaf.path.root, placed.instance);
        assert_eq!(leaf.path.members, vec![placed.member]);
        assert_refused(&s, set, vec![placed.instance]);
    }
    let before = s.document().clone();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(before.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let p = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let fresh = Session::new(p.document).unwrap();
    assert_refused(&fresh, set, vec![placed.instance]);
    assert_eq!(s.document(), &before);
}
