use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::*,
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, PrimitiveFrame, ThreadParameters,
        ThreadRepresentation, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, EvaluationCache, export::Exporter};
use std::f64::consts::PI;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Native mechanical parts").unwrap()).unwrap()
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn washer() -> MechanicalPartParameters {
    MechanicalPartParameters::Washer {
        bore_diameter: length(3.2),
        outside_diameter: length(7.),
        thickness: length(0.5),
        frame: Default::default(),
    }
}
fn nut(representation: ThreadRepresentation) -> MechanicalPartParameters {
    MechanicalPartParameters::HexNut {
        across_flats: length(10.),
        thread: Box::new(ThreadParameters {
            major_diameter: length(6.),
            pitch: length(1.),
            length: length(5.2),
            representation,
            ..Default::default()
        }),
    }
}
fn add(s: &mut Session, name: &str, parameters: MechanicalPartParameters) -> MechanicalPartRequest {
    let r = MechanicalPartRequest::new(name, parameters);
    let p = model::prepare_mechanical_part(s, s.revision(), actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    r
}
fn probe(solid: &Solid, center: [f64; 3]) -> f64 {
    let size = 0.005;
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(
            &cube,
            BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / size.powi(3)
}
#[test]
fn washer_and_rotated_bearing_envelopes_have_exact_mounting_dimensions_and_step_readbacks() {
    let mut s = session();
    let w = add(&mut s, "Washer", washer());
    let b = add(
        &mut s,
        "Bearing mounting envelope",
        MechanicalPartParameters::BearingEnvelope {
            bore_diameter: length(10.),
            outside_diameter: length(30.),
            width: length(9.),
            frame: PrimitiveFrame {
                origin: [length(40.), length(20.), length(30.)],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
        },
    );
    let built = compiler().compile(s.document(), s.revision()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (r, expected) in [
        (&w, PI * (3.5_f64.powi(2) - 1.6_f64.powi(2)) * 0.5),
        (&b, PI * (15_f64.powi(2) - 5_f64.powi(2)) * 9.),
    ] {
        let solid = built.solid(r.feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!((solid.volume_mm3().unwrap() - expected).abs() < 1e-6);
        let topology = solid.topology_snapshot(r.feature, s.revision()).unwrap();
        let faces: Vec<_> = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
            .collect();
        assert!(!faces.is_empty());
        assert!(faces.iter().all(|face| !face.origins.is_empty()));
        let path = dir.path().join(format!("{}.step", r.feature));
        solid.write_step(&path).unwrap();
        let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(readback.solid_count().unwrap(), 1);
        assert!((readback.volume_mm3().unwrap() - expected).abs() < 1e-6);
    }
    let bounds = built.solid(b.feature).unwrap().bounds().unwrap();
    assert!((bounds.min[0] - 40.).abs() < 1e-6);
    assert!((bounds.max[0] - 49.).abs() < 1e-6);
    assert!((bounds.min[1] - 5.).abs() < 1e-6);
    assert!((bounds.max[2] - 45.).abs() < 1e-6);
    assert!(probe(built.solid(w.feature).unwrap(), [0., 0., 0.25]) < 1e-5);
    assert!(probe(built.solid(w.feature).unwrap(), [2., 0., 0.25]) > 0.999);
}
#[test]
fn hex_nut_contains_a_real_helical_internal_thread_and_retains_flats_and_height() {
    let mut s = session();
    let r = add(
        &mut s,
        "Modeled hex nut",
        nut(ThreadRepresentation::Modeled),
    );
    let built = compiler().compile(s.document(), s.revision()).unwrap();
    let solid = built.solid(r.feature).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    assert!((bounds.min[0] + 5.).abs() < 1e-5);
    assert!((bounds.max[0] - 5.).abs() < 1e-5);
    assert!(bounds.min[2].abs() < 1e-5);
    assert!((bounds.max[2] - 5.2).abs() < 1e-5);
    let pitch_radius = 3. - 3. * 3_f64.sqrt() / 16.;
    for turn in [1., 2., 3.] {
        assert!(
            probe(solid, [pitch_radius, 0., turn + 0.2]) < 1e-5,
            "the thread cutter crest must open the bore"
        );
        assert!(
            probe(solid, [pitch_radius, 0., turn + 0.3]) > 0.999,
            "the gap between cutter ridges must retain nut material"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nut.step");
    solid.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    assert_eq!(readback.solid_count().unwrap(), 1);
    assert!((readback.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-5);
}
#[test]
fn native_cache_edits_history_and_reopen_preserve_parametric_body_identity() {
    let mut s = session();
    let r = add(&mut s, "Washer", washer());
    let committed = s.document().clone();
    let mut cache = EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(cache.last_compile().reused.contains(&r.feature));
    let mut p = r.parameters.clone();
    let MechanicalPartParameters::Washer { thickness, .. } = &mut p else {
        panic!()
    };
    *thickness = length(1.);
    let prepared =
        model::prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &p).unwrap();
    let candidate = compiler()
        .compile(prepared.prepared.document(), s.revision() + 1)
        .unwrap();
    assert_eq!(s.document(), &committed);
    assert!(
        (candidate.solid(r.feature).unwrap().volume_mm3().unwrap()
            - 2. * first.solid(r.feature).unwrap().volume_mm3().unwrap())
        .abs()
            < 1e-6
    );
    s.commit(prepared.prepared).unwrap();
    compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(cache.last_compile().evaluated.contains(&r.feature));
    assert_eq!(
        model::read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .output
            .body,
        r.body
    );
    let changed = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &committed);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &changed);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(changed.clone()).unwrap(),
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
    .project
    .document;
    let fresh = compiler().compile(&reopened, 0).unwrap();
    assert!(
        (fresh.solid(r.feature).unwrap().volume_mm3().unwrap()
            - candidate.solid(r.feature).unwrap().volume_mm3().unwrap())
        .abs()
            < 1e-6
    );
}
#[test]
fn cosmetic_nut_threads_cannot_be_published_as_modeled_manufacturing_geometry() {
    let mut s = session();
    let r = add(
        &mut s,
        "Cosmetic nut",
        nut(ThreadRepresentation::CosmeticEnvelope),
    );
    assert!(model::has_cosmetic_threads(s.document()).unwrap());
    assert_eq!(
        model::cosmetic_thread_in_history(s.document(), r.feature).unwrap(),
        Some(r.feature)
    );
    let set = EntityId::new();
    let p = prepare_set(
        &s,
        s.revision(),
        actor(),
        set,
        "Manufacturing",
        &SetDefinition {
            recipes: vec![
                Recipe::step("STEP"),
                Recipe {
                    id: EntityId::new(),
                    name: "STL".into(),
                    output: RecipeOutput::Mesh {
                        format: MeshFormat::Stl,
                        units: LengthUnit::Millimeter,
                        hierarchy: HierarchyMode::SeparateParts,
                        names: NameHandling::FileNamesOnly,
                        aids: AidPolicy::IncludeAll,
                        quality: MeshQuality::default(),
                    },
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let exporter = Exporter::default();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(!preview.can_run());
    assert!(preview.artifacts.is_empty());
    assert!(preview.recipes.iter().all(|r|matches!(&r.status,RecipeStatus::Unsupported(reason) if reason.contains("cosmetic")&&reason.contains("Modeled"))));
    let request = PrintRequest {
        source: PrintSource::Parts {
            parts: vec![r.body],
        },
        aids: AidPolicy::IncludeAll,
        quality: MeshQuality::default(),
        stl_per_part: true,
    };
    let error = exporter
        .print_preview(s.document(), s.revision(), &request, &actor(), None)
        .err()
        .unwrap();
    assert!(error.message.contains("cosmetic") && error.message.contains("Modeled"));
    let p = model::prepare_mechanical_part_edit(
        &s,
        s.revision(),
        actor(),
        r.feature,
        &nut(ThreadRepresentation::Modeled),
    )
    .unwrap();
    s.commit(p.prepared).unwrap();
    assert!(!model::has_cosmetic_threads(s.document()).unwrap());
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(preview.can_run());
    assert!(!preview.artifacts.is_empty());
    assert!(
        exporter
            .print_preview(s.document(), s.revision(), &request, &actor(), None)
            .is_ok()
    );
}
