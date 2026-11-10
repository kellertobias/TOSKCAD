use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::*,
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, PrimitiveFrame, ThreadFit,
        ThreadHand, ThreadParameters, ThreadRepresentation, length,
    },
    project_file::{self, ProjectFile},
    references::{GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{Compiler, EvaluationCache, GeometryBackend, export::Exporter};
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
fn rust_annular_catalog_parts_match_native_faces_and_step_readback() {
    let directory = tempfile::tempdir().unwrap();
    let policy = GeometryPolicy::default();
    for file in ["washer-normal-m6.tcd", "bearing-open-608.tcd"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../library")
            .join(file);
        let project = project_file::read(
            &mut std::fs::File::open(path).unwrap(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let native = compiler().compile(&project.document, 0).unwrap();
        let rust = compiler()
            .compile_with_backend(&project.document, 0, GeometryBackend::Rust, None)
            .unwrap();
        let (&part, &feature) = rust.snapshot().bodies.iter().next().unwrap();
        assert_eq!(native.snapshot().bodies[&part], feature);
        let ring = rust.solid(feature).unwrap();
        let control = native.solid(feature).unwrap();
        assert!(ring.uses_rust_geometry());
        assert_eq!(ring.solid_count().unwrap(), 1);
        let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-7, "{file}: {a} != {b}");
        near(ring.volume_mm3().unwrap(), control.volume_mm3().unwrap());
        let bounds = ring.bounds().unwrap();
        let reference = control.bounds().unwrap();
        for (a, b) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip(reference.min.into_iter().chain(reference.max))
        {
            near(a, b);
        }
        let native_topology = control.topology_snapshot(feature, 0).unwrap();
        let rust_topology = ring.topology_snapshot(feature, 0).unwrap();
        let faces = ring.faces().unwrap();
        let native_faces = control.faces().unwrap();
        assert_eq!(faces.len(), 4);
        for element in native_topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let intent = GeometryIntent::capture(&native_topology, element.key).unwrap();
            let Resolution::Resolved { key, .. } = resolve(&intent, &rust_topology, 0) else {
                panic!("{file} lost source face intent {intent:?}");
            };
            near(
                faces[key.ordinal as usize].area_mm2,
                native_faces[element.key.ordinal as usize].area_mm2,
            );
        }
        let step = directory.path().join(file).with_extension("step");
        ring.write_step(&step).unwrap();
        let imported = Solid::read_step(&step, policy).unwrap();
        assert_eq!(imported.solid_count().unwrap(), 1);
        near(imported.volume_mm3().unwrap(), ring.volume_mm3().unwrap());
        assert!(probe(&imported, [0., 0., bounds.max[2] / 2.]) < 1e-8);
    }
}

#[test]
fn rust_hex_nut_rebuilds_iso_and_din_m6_catalog_with_original_stock_face_intents() {
    for file in ["nut-iso4032-2012-m6.tcd", "nut-din934-1987-m6.tcd"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../library")
            .join(file);
        let project = project_file::read(
            &mut std::fs::File::open(path).unwrap(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let document = &project.document;
        let rust = compiler()
            .compile_with_backend(document, 0, GeometryBackend::Rust, None)
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let native = compiler().compile(document, 0).unwrap();
        let (&part, &feature) = rust.snapshot().bodies.iter().next().unwrap();
        assert_eq!(rust.snapshot().bodies.len(), 1);
        assert_eq!(native.snapshot().bodies[&part], feature);
        let nut = rust.solid(feature).unwrap();
        let control = native.solid(feature).unwrap();
        assert!(nut.uses_rust_geometry());
        assert_eq!(nut.solid_count().unwrap(), 1);
        let expected = control.volume_mm3().unwrap();
        assert!(
            (nut.volume_mm3().unwrap() - expected).abs() < 5e-4,
            "{file}"
        );
        let a = nut.bounds().unwrap();
        let b = control.bounds().unwrap();
        for (a, b) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            assert!((a - b).abs() < 2e-5, "{file}");
        }
        let parameters = model::read_mechanical_part(document, feature)
            .unwrap()
            .parameters;
        let model::ResolvedMechanicalPart::HexNut { thread, .. } = parameters
            .resolve(&multicad_core::parameters::evaluate(document).unwrap())
            .unwrap()
        else {
            panic!("Expected catalog nut");
        };
        let r = thread.major_radius_mm - thread.profile.depth_mm / 2.;
        for (point, material) in [
            ([0., 0., 2.], false),
            ([r, 0., 2.], false),
            ([r, 0., 2.5], true),
            ([4.8, 0., 2.], true),
            ([5.2, 0., 2.], false),
        ] {
            assert_eq!(
                nut.rust_thread_contains(point).unwrap(),
                material,
                "{file} {point:?}"
            );
            let native_material = probe(control, point);
            assert!(
                if material {
                    native_material > 0.99
                } else {
                    native_material < 1e-5
                },
                "{file} {point:?}: native occupied fraction {native_material}"
            );
        }

        let rust_topology = nut.topology_snapshot(feature, 0).unwrap();
        let native_topology = control.topology_snapshot(feature, 0).unwrap();
        let native_faces = control.faces().unwrap();
        let rust_faces = nut.faces().unwrap();
        let mut matched = 0;
        for native_face in native_faces.iter().filter(|face| face.plane.is_some()) {
            let element = native_topology
                .elements
                .iter()
                .find(|element| {
                    element.key.kind == TopologyKind::Face
                        && element.key.ordinal == native_face.index
                })
                .unwrap();
            let intent = GeometryIntent::capture(&native_topology, element.key).unwrap();
            let Resolution::Resolved { key, .. } = resolve(&intent, &rust_topology, 0) else {
                panic!(
                    "{file}: native stock face origins {:?} did not resolve",
                    element.origins
                );
            };
            let face = rust_faces
                .iter()
                .find(|face| face.index == key.ordinal)
                .unwrap();
            let (origin, normal) = native_face.plane.unwrap();
            let (point, actual_normal) = face.plane.unwrap();
            let dot =
                |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
            assert!((dot(normal, actual_normal) - 1.).abs() < 1e-10, "{file}");
            assert!(
                dot(std::array::from_fn(|i| point[i] - origin[i]), normal).abs() < 1e-6,
                "{file}"
            );
            matched += 1;
        }
        assert_eq!(matched, 8, "{file}");
    }
}

#[test]
fn rust_hex_nut_retains_outer_face_intent_after_hand_edit_and_reopen() {
    let mut session = session();
    let mut source = nut(ThreadRepresentation::Modeled);
    let MechanicalPartParameters::HexNut { thread, .. } = &mut source else {
        unreachable!()
    };
    thread.fit = ThreadFit::Metric6HInternal;
    thread.frame = PrimitiveFrame {
        origin: [length(10.), length(20.), length(30.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let request = add(&mut session, "Rotated fitted M6 nut", source.clone());
    let build = |document: &Document, revision| {
        compiler()
            .compile_with_backend(document, revision, GeometryBackend::Rust, None)
            .unwrap()
    };
    let before = build(session.document(), 1);
    let solid = before.solid(request.feature).unwrap();
    let bounds = solid.bounds().unwrap();
    assert_eq!(bounds.min[0..2], [10., 15.]);
    assert_eq!(bounds.max[0..2], [15.2, 25.]);
    assert!(!solid.rust_thread_contains([12., 20., 30.]).unwrap());
    assert!(solid.rust_thread_contains([12., 24.9, 30.]).unwrap());
    let original = solid.topology_snapshot(request.feature, 1).unwrap();
    let outer = original
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face
                && element
                    .origins
                    .iter()
                    .any(|origin| origin.role == "hardware.outer/0")
        })
        .unwrap();
    let intent = GeometryIntent::capture(&original, outer.key).unwrap();
    let MechanicalPartParameters::HexNut { thread, .. } = &mut source else {
        unreachable!()
    };
    thread.hand = ThreadHand::Left;
    let edit = model::prepare_mechanical_part_edit(&session, 1, actor(), request.feature, &source)
        .unwrap();
    session.commit(edit.prepared).unwrap();
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
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
    assert_eq!(
        model::read_mechanical_part(&reopened.document, request.feature)
            .unwrap()
            .parameters,
        source
    );
    let rebuilt = build(&reopened.document, 0);
    let changed = rebuilt.solid(request.feature).unwrap();
    assert_eq!(changed.bounds().unwrap(), bounds);
    assert!((changed.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-9);
    let current = changed.topology_snapshot(request.feature, 0).unwrap();
    assert!(
        matches!(resolve(&intent, &current, 0), Resolution::Resolved { key, .. } if key == outer.key)
    );
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
    for turn in [1., 2., 3.] {
        assert!(probe(&readback, [pitch_radius, 0., turn + 0.2]) < 1e-5);
        assert!(probe(&readback, [pitch_radius, 0., turn + 0.3]) > 0.999);
    }
    let before_volume = solid.volume_mm3().unwrap();
    let after_volume = readback.volume_mm3().unwrap();
    let root = 3. - 5. * 3_f64.sqrt() / 16.;
    // Each axial slice has the same annular thread area, even at fractional
    // pitches. Integrate its linear flanks, then subtract from the hex prism.
    let integral = 0.125 * (9. - root * root) / 2.
        + 2. / 3_f64.sqrt() * (3. * (9. - root * root) / 2. - (27. - root.powi(3)) / 3.);
    let expected = 5.2 * (50. * 3_f64.sqrt() - PI * root * root - 2. * PI * integral);
    assert!(
        (before_volume - expected).abs() < 1e-5,
        "native={before_volume:.15}, expected={expected:.15}"
    );
    assert!(
        (after_volume - expected).abs() < 1e-5,
        "STEP={after_volume:.15}, expected={expected:.15}"
    );
    assert!(
        (after_volume - before_volume).abs() < 1e-5,
        "before={before_volume:.15}, after={after_volume:.15}, expected={expected:.15}"
    );
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
