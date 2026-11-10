use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::*,
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, PrimitiveFrame, ScrewDrive,
        ScrewHead, ThreadHand, ThreadRepresentation, length,
    },
    project_file::{self, ProjectFile},
    references::TopologyKind,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, MeshOptions, RigidTransform, Solid};
use multicad_model::{Compiler, EvaluationCache, export::Exporter};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn session() -> Session {
    Session::new(Document::new("Native headed screws").unwrap()).unwrap()
}
fn add(s: &mut Session, parameters: MechanicalPartParameters) -> MechanicalPartRequest {
    let request = MechanicalPartRequest::new("Machine screw", parameters);
    let prepared = model::prepare_mechanical_part(s, s.revision(), actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    request
}
fn probe(solid: &Solid, center: [f64; 3]) -> f64 {
    let size: f64 = 0.003;
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|x| x - size / 2.),
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
fn parameters(hex: bool, partial: bool, left: bool, rotated: bool) -> MechanicalPartParameters {
    let mut p = model::default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        shank_diameter,
        head,
        drive,
        thread,
        ..
    } = &mut p
    else {
        panic!()
    };
    *underhead_length = Some(length(6.));
    *shank_diameter = length(3.);
    thread.length = length(if partial { 2.5 } else { 6. });
    thread.hand = if left {
        ThreadHand::Left
    } else {
        ThreadHand::Right
    };
    if hex {
        *head = ScrewHead::ExternalHex {
            across_flats: length(5.5),
            height: length(2.),
        };
        *drive = ScrewDrive::ExternalHex;
    }
    if rotated {
        thread.frame = PrimitiveFrame {
            origin: [length(10.), length(20.), length(30.)],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        };
    }
    p
}
fn world(p: [f64; 3], rotated: bool) -> [f64; 3] {
    if rotated {
        [10. + p[2], 20. + p[0], 30. + p[1]]
    } else {
        p
    }
}

fn expected_screw_volume(
    head_volume: f64,
    diameter: f64,
    pitch: f64,
    length: f64,
    threaded: f64,
) -> f64 {
    let radius = diameter / 2.;
    let root = radius - 5. * 3_f64.sqrt() / 16. * pitch;
    // Every axial slice rotates the same flat-root metric profile. Integrate
    // the two linear flanks and P/8 crest, including fractional turn lengths.
    let integral = (radius * radius - root * root) / 16.
        + 2. / (3_f64.sqrt() * pitch)
            * (radius * (radius * radius - root * root) / 2.
                - (radius.powi(3) - root.powi(3)) / 3.);
    let thread_area = std::f64::consts::PI * root * root + 2. * std::f64::consts::PI * integral;
    head_volume
        + std::f64::consts::PI * radius * radius * (length - threaded)
        + thread_area * threaded
}

fn assert_screw_volumes(native: &Solid, step: &Solid, analytic: f64) {
    // New screw metrology uses the actual native linear policy and the
    // unchanged maximum GK relative error in kernel/src/bridge.cpp:volume.
    // Budget each solid by boundary uncertainty epsilon*surface_area plus
    // quadrature uncertainty r*|volume|; a roundtrip gets their sum. Independent
    // dimensional/material probes remain required. This is no physical-fit or
    // manufactured-tolerance claim, and does not change legacy nut assertions.
    let epsilon = GeometryPolicy::default().linear_tolerance_mm;
    let relative_limit = 1e-7;
    let mut measurements = Vec::new();
    for solid in [native, step] {
        let volume = solid.volume_mm3().unwrap();
        let area = solid
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>();
        assert!(area.is_finite() && area > 0.);
        let budget = epsilon * area + relative_limit * volume.abs();
        assert!(
            (volume - analytic).abs() <= budget,
            "volume={volume:.15}, analytic={analytic:.15}, area={area:.15}, budget={budget:.15}"
        );
        measurements.push((volume, budget));
    }
    let delta = (measurements[0].0 - measurements[1].0).abs();
    let budget = measurements[0].1 + measurements[1].1;
    assert!(
        delta <= budget,
        "native/STEP delta={delta:.15}, budget={budget:.15}"
    );
}
fn verify_geometry(solid: &Solid, hex: bool, partial: bool, left: bool, rotated: bool) {
    let point = |p| world(p, rotated);
    let k = if hex { 2. } else { 3. };
    let bounds = solid.bounds().unwrap();
    let axis = if rotated { 0 } else { 2 };
    let origin = if rotated { 10. } else { 0. };
    assert!((bounds.min[axis] - (origin - k)).abs() < 1e-5);
    assert!((bounds.max[axis] - (origin + 6.)).abs() < 1e-5);
    assert_eq!(solid.solid_count().unwrap(), 1);
    assert!(probe(solid, point([0., 0., 0.02])) > 0.999);
    if hex {
        assert!(probe(solid, point([2.70, 0., -1.])) > 0.999);
        assert!(probe(solid, point([2.80, 0., -1.])) < 1e-5);
        assert!(probe(solid, point([0., 3.05, -1.])) > 0.999);
    } else {
        assert!(probe(solid, point([2.80, 0., -1.])) > 0.999);
        assert!(probe(solid, point([2.88, 0., -1.])) < 1e-5);
        assert!(probe(solid, point([0., 0., -2.5])) < 1e-5);
        // A hex socket has a corner beyond AF/2, while retaining material
        // beyond a flat: these distinguish both possible cylindrical bores.
        assert!(probe(solid, point([0., 1.35, -2.5])) < 1e-5);
        assert!(probe(solid, point([1.35, 0., -2.5])) > 0.999);
        assert!(probe(solid, point([0., 0., -1.65])) > 0.999);
        assert!(probe(solid, point([0., 0., -1.75])) < 1e-5);
    }
    let start = if partial { 3.5 } else { 0. };
    let pitch = 0.5;
    let radius = 1.5 - 3. * 3_f64.sqrt() / 16. * pitch;
    if partial {
        assert!(probe(solid, point([radius, 0., 1.])) > 0.999);
    }
    for turn in [1., 2.] {
        assert!(probe(solid, point([radius, 0., start + (turn + 0.20) * pitch])) > 0.999);
        assert!(
            probe(solid, point([radius, 0., start + (turn + 0.30) * pitch])) < 1e-5,
            "a shank cylinder must not fill the thread grooves"
        );
    }
    // At a quarter turn, left-handed geometry is the Y mirror of right-handed.
    let y = if left { -radius } else { radius };
    assert!(probe(solid, point([0., y, start + 1.45 * pitch])) > 0.999);
    assert!(probe(solid, point([0., y, start + 1.55 * pitch])) < 1e-5);
}

#[test]
fn both_head_forms_full_partial_and_left_threads_have_exact_native_sections_and_step_readbacks() {
    let dir = tempfile::tempdir().unwrap();
    for (hex, partial, left, rotated) in [
        (false, false, false, false),
        (false, true, true, true),
        (true, false, true, false),
        (true, true, false, true),
    ] {
        let mut s = session();
        let r = add(&mut s, parameters(hex, partial, left, rotated));
        let model = compiler().compile(s.document(), s.revision()).unwrap();
        let solid = model.solid(r.feature).unwrap();
        verify_geometry(solid, hex, partial, left, rotated);
        let topology = solid.topology_snapshot(r.feature, s.revision()).unwrap();
        let faces: Vec<_> = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .collect();
        assert!(faces.iter().all(|f| !f.origins.is_empty()));
        assert!(
            faces
                .iter()
                .flat_map(|f| &f.origins)
                .any(|o| o.role.starts_with("thread/start/"))
        );
        assert!(
            faces
                .iter()
                .flat_map(|f| &f.origins)
                .any(|o| o.role.starts_with("screw.head/"))
        );
        if !hex {
            assert!(
                faces
                    .iter()
                    .flat_map(|f| &f.origins)
                    .any(|o| o.role == "screw.socket/end")
            );
        }
        assert!(
            faces
                .iter()
                .flat_map(|f| &f.origins)
                .all(|o| o.role != "cap.start" && o.role != "cap.end")
        );
        let mesh = solid
            .mesh(MeshOptions {
                linear_deflection_mm: 0.025,
                ..Default::default()
            })
            .unwrap();
        assert!(!mesh.indices.is_empty());
        let path = dir.path().join(format!("{}.step", r.feature));
        solid.write_step(&path).unwrap();
        let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        verify_geometry(&readback, hex, partial, left, rotated);
        let head_volume = if hex {
            3_f64.sqrt() / 2. * 5.5_f64.powi(2) * 2.
        } else {
            std::f64::consts::PI * 2.84_f64.powi(2) * 3.
                - 3_f64.sqrt() / 2. * 2.55_f64.powi(2) * 1.3
        };
        assert_screw_volumes(
            solid,
            &readback,
            expected_screw_volume(head_volume, 3., 0.5, 6., if partial { 2.5 } else { 6. }),
        );
    }
}

#[test]
fn screw_edits_invalidate_cache_keep_identity_and_reopen_in_an_independent_session() {
    let mut s = session();
    let r = add(&mut s, parameters(false, true, false, false));
    let mut cache = EvaluationCache::default();
    let initial = compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    compiler()
        .compile_cached(&mut cache, s.document(), s.revision(), None)
        .unwrap();
    assert!(cache.last_compile().reused.contains(&r.feature));
    let original = s.document().clone();
    let mut changed = r.parameters.clone();
    let MechanicalPartParameters::MachineScrew { drive, .. } = &mut changed else {
        panic!()
    };
    *drive = ScrewDrive::HexSocket {
        across_flats: length(2.55),
        depth: length(1.5),
    };
    let prepared =
        model::prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &changed)
            .unwrap();
    let candidate = compiler()
        .compile(prepared.prepared.document(), s.revision() + 1)
        .unwrap();
    assert_eq!(s.document(), &original);
    assert!(
        candidate.solid(r.feature).unwrap().volume_mm3().unwrap()
            < initial.solid(r.feature).unwrap().volume_mm3().unwrap()
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
    let saved = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &saved);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved).unwrap(),
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
    let model = compiler()
        .compile(fresh.document(), fresh.revision())
        .unwrap();
    assert!(
        (model.solid(r.feature).unwrap().volume_mm3().unwrap()
            - candidate.solid(r.feature).unwrap().volume_mm3().unwrap())
        .abs()
            < 1e-6
    );
}

#[test]
fn cosmetic_screw_threads_are_refused_by_step_mesh_and_print_manufacturing_paths() {
    let mut s = session();
    let mut p = parameters(false, false, false, false);
    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut p else {
        panic!()
    };
    thread.representation = ThreadRepresentation::CosmeticEnvelope;
    let r = add(&mut s, p);
    assert!(model::has_cosmetic_threads(s.document()).unwrap());
    assert_eq!(
        model::cosmetic_thread_in_history(s.document(), r.feature).unwrap(),
        Some(r.feature)
    );
    let set = EntityId::new();
    let prepared = prepare_set(
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
    s.commit(prepared).unwrap();
    let exporter = Exporter::default();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(!preview.can_run());
    assert!(preview.artifacts.is_empty());
    assert!(preview.recipes.iter().all(
        |r| matches!(&r.status,RecipeStatus::Unsupported(reason) if reason.contains("cosmetic"))
    ));
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
    assert!(error.message.contains("cosmetic"));
    let mut modeled = model::read_mechanical_part(s.document(), r.feature)
        .unwrap()
        .parameters;
    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut modeled else {
        panic!()
    };
    thread.representation = ThreadRepresentation::Modeled;
    let prepared =
        model::prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &modeled)
            .unwrap();
    s.commit(prepared.prepared).unwrap();
    let preview = exporter
        .preview(s.document(), s.revision(), set, &actor(), None)
        .unwrap();
    assert!(preview.can_run());
    assert!(!preview.artifacts.is_empty());
    let package = exporter
        .print_preview(s.document(), s.revision(), &request, &actor(), None)
        .unwrap();
    assert!(package.can_write());
    assert!(!package.objects.is_empty());
    assert!(package.objects.iter().all(|o| o.triangles > 0));
}

#[test]
fn impossible_socket_floor_corner_wall_thread_extent_and_pitch_fail_atomically() {
    let mut s = session();
    let r = add(&mut s, parameters(false, true, false, false));
    let before = s.document().clone();
    for case in 0..4 {
        let mut p = r.parameters.clone();
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            drive,
            thread,
            ..
        } = &mut p
        else {
            panic!()
        };
        match case {
            0 => {
                *drive = ScrewDrive::HexSocket {
                    across_flats: length(2.55),
                    depth: length(3.),
                }
            }
            1 => {
                *drive = ScrewDrive::HexSocket {
                    across_flats: length(5.5),
                    depth: length(1.3),
                }
            }
            2 => *underhead_length = Some(length(2.)),
            _ => thread.pitch = length(5.),
        }
        assert!(
            model::prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &p).is_err(),
            "invalid case {case}"
        );
        assert_eq!(s.document(), &before);
    }
}

#[test]
fn positive_but_unresolvable_socket_walls_and_floors_fail_native_preflight_without_commit() {
    let s = session();
    let before = s.document().clone();
    let minimum = GeometryPolicy::default().minimum_feature_mm;
    for floor_case in [true, false] {
        let mut p = model::default_machine_screw();
        let MechanicalPartParameters::MachineScrew { head, drive, .. } = &mut p else {
            panic!()
        };
        let ScrewHead::SocketCap { diameter, height } = head else {
            panic!()
        };
        let ScrewDrive::HexSocket {
            across_flats,
            depth,
        } = drive
        else {
            panic!()
        };
        *diameter = length(5.68);
        *height = length(3.);
        if floor_case {
            *depth = length(3. - minimum / 2.);
        } else {
            *across_flats = length((5.68 / 2. - minimum / 2.) * 3_f64.sqrt());
        }
        let request = MechanicalPartRequest::new("Unresolvable socket", p);
        let candidate =
            model::prepare_mechanical_part(&s, s.revision(), actor(), &request).unwrap();
        let error = compiler()
            .compile(candidate.prepared.document(), s.revision() + 1)
            .err()
            .expect("native preflight must reject the unresolved socket");
        assert!(error.message.contains("minimum feature size"));
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
    }
}

#[test]
fn sourced_stock_length_representatives_preserve_fractional_turns_and_step_volumes() {
    let directory = tempfile::tempdir().unwrap();
    for (d, p, l, b, dk, k, af, depth) in [
        (3., 0.5, 30., 18., 5.68, 3., 2.55, 1.3),
        (2.5, 0.45, 12., 12., 4.5, 2.5, 2.05, 1.1),
        (10., 1.5, 50., 32., 16.27, 10., 8.10, 5.),
    ] {
        let mut parameters = model::default_machine_screw();
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            shank_diameter,
            head,
            drive,
            thread,
            ..
        } = &mut parameters
        else {
            panic!()
        };
        *underhead_length = Some(length(l));
        *shank_diameter = length(d);
        *head = ScrewHead::SocketCap {
            diameter: length(dk),
            height: length(k),
        };
        *drive = ScrewDrive::HexSocket {
            across_flats: length(af),
            depth: length(depth),
        };
        thread.major_diameter = length(d);
        thread.pitch = length(p);
        thread.length = length(b);
        let mut s = session();
        let request = add(&mut s, parameters);
        let built = compiler().compile(s.document(), s.revision()).unwrap();
        let solid = built.solid(request.feature).unwrap();
        let path = directory.path().join(format!("m{d}.step"));
        solid.write_step(&path).unwrap();
        let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        let head_volume =
            std::f64::consts::PI * (dk / 2_f64).powi(2) * k - 3_f64.sqrt() / 2. * af * af * depth;
        assert_screw_volumes(
            solid,
            &readback,
            expected_screw_volume(head_volume, d, p, l, b),
        );
        let radius = d / 2. - 3. * 3_f64.sqrt() / 16. * p;
        for geometry in [solid, &readback] {
            assert_eq!(geometry.solid_count().unwrap(), 1);
            assert!((geometry.bounds().unwrap().max[2] - l).abs() < 1e-5);
            assert!(probe(geometry, [radius, 0., l - b + 1.20 * p]) > 0.999);
            assert!(probe(geometry, [radius, 0., l - b + 1.30 * p]) < 1e-5);
        }
    }
}

#[test]
fn rust_headed_screw_rebuilds_original_socket_and_hex_catalog_projects() {
    use multicad_core::references::{GeometryIntent, Resolution, resolve};
    use multicad_model::GeometryBackend;

    for (file, socket) in [
        ("screw-socket-m6-l20.tcd", true),
        ("screw-hex-m6-l20.tcd", false),
    ] {
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
        let source = project.document.clone();
        let exact = compiler()
            .compile_with_backend(&source, 0, GeometryBackend::Rust, None)
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let native = compiler().compile(&source, 0).unwrap();
        let (&part, &feature) = exact.snapshot().bodies.iter().next().unwrap();
        assert_eq!(exact.snapshot().bodies.len(), 1);
        assert_eq!(native.snapshot().bodies[&part], feature);
        let screw = exact.solid(feature).unwrap();
        let control = native.solid(feature).unwrap();
        assert!(screw.uses_rust_geometry());
        assert_eq!(screw.solid_count().unwrap(), 1);
        let resolved = model::read_mechanical_part(&source, feature).unwrap();
        let model::ResolvedMechanicalPart::MachineScrew { head, drive, .. } = resolved
            .parameters
            .resolve(&multicad_core::parameters::evaluate(&source).unwrap())
            .unwrap()
        else {
            panic!("Expected catalog machine screw");
        };
        let h = head.height_mm();
        if socket {
            let model::ResolvedScrewDrive::HexSocket {
                across_flats_mm,
                depth_mm,
            } = drive
            else {
                panic!("Expected catalog hex socket");
            };
            let between_flat_and_corner = (0.5 + 1. / 3_f64.sqrt()) * across_flats_mm / 2.;
            assert!(
                !screw
                    .rust_thread_contains([0., between_flat_and_corner, -h + depth_mm / 2.])
                    .unwrap()
            );
            assert!(
                screw
                    .rust_thread_contains([between_flat_and_corner, 0., -h + depth_mm / 2.])
                    .unwrap()
            );
            assert!(
                screw
                    .rust_thread_contains([0., 0., -h + depth_mm + 0.1])
                    .unwrap()
            );
        } else {
            let model::ResolvedScrewHead::ExternalHex {
                across_flats_mm, ..
            } = head
            else {
                panic!("Expected catalog external hex");
            };
            let between_flat_and_corner = (0.5 + 1. / 3_f64.sqrt()) * across_flats_mm / 2.;
            assert!(
                screw
                    .rust_thread_contains([0., between_flat_and_corner, -h / 2.])
                    .unwrap()
            );
            assert!(
                !screw
                    .rust_thread_contains([between_flat_and_corner, 0., -h / 2.])
                    .unwrap()
            );
        }
        let volume = control.volume_mm3().unwrap();
        let area: f64 = control
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum();
        let budget = GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * volume;
        assert!(
            (screw.volume_mm3().unwrap() - volume).abs() <= budget,
            "{file}: Rust {}, native {volume}, budget {budget}",
            screw.volume_mm3().unwrap()
        );
        let actual = screw.bounds().unwrap();
        let expected = control.bounds().unwrap();
        for (value, control) in actual
            .min
            .into_iter()
            .chain(actual.max)
            .zip(expected.min.into_iter().chain(expected.max))
        {
            assert!(
                (value - control).abs() < 2e-5,
                "{file}: {actual:?} vs {expected:?}"
            );
        }
        let native_topology = control.topology_snapshot(feature, 0).unwrap();
        let rust_topology = screw.topology_snapshot(feature, 0).unwrap();
        for role in ["screw.thread/end", "screw.head/start"]
            .into_iter()
            .chain(socket.then_some("screw.socket/end"))
        {
            let native_face = native_topology
                .elements
                .iter()
                .find(|element| {
                    element.key.kind == TopologyKind::Face
                        && element.origins.iter().any(|origin| origin.role == role)
                })
                .unwrap();
            let intent = GeometryIntent::capture(&native_topology, native_face.key).unwrap();
            assert!(
                matches!(
                    resolve(&intent, &rust_topology, 0),
                    Resolution::Resolved { key, .. }
                        if rust_topology.elements.iter().any(|element| element.key == key && element.origins.iter().any(|origin| origin.role == role))
                ),
                "{file}: retained face intent {role} did not resolve"
            );
        }
        assert_eq!(project.document, source);
    }
}

#[test]
fn rust_headed_screw_fitted_partial_rotated_profile_retains_intent_through_edit_and_reopen() {
    use multicad_core::{
        model::ThreadFit,
        references::{GeometryIntent, Resolution, resolve},
    };
    use multicad_model::GeometryBackend;

    let mut session = session();
    let mut source = parameters(false, true, true, true);
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        shank_diameter,
        head,
        drive,
        thread,
        ..
    } = &mut source
    else {
        unreachable!()
    };
    *underhead_length = Some(length(12.));
    *shank_diameter = length(6.);
    *head = ScrewHead::SocketCap {
        diameter: length(10.),
        height: length(6.),
    };
    *drive = ScrewDrive::HexSocket {
        across_flats: length(5.),
        depth: length(3.),
    };
    thread.major_diameter = length(6.);
    thread.pitch = length(1.);
    thread.length = length(6.5);
    thread.fit = ThreadFit::Metric6gExternal;

    let request = add(&mut session, source.clone());
    let build = |document: &Document, revision| {
        compiler()
            .compile_with_backend(document, revision, GeometryBackend::Rust, None)
            .unwrap()
    };
    let original = build(session.document(), 1);
    let solid = original.solid(request.feature).unwrap();
    let model::ResolvedMechanicalPart::MachineScrew { thread, .. } =
        source.resolve(session.parameters()).unwrap()
    else {
        unreachable!()
    };
    let radius = thread.major_radius_mm - thread.profile.depth_mm / 2.;
    for (p, is_material) in [
        ([2.95, 0., 4.], true),
        ([0., -radius, 6.75], true),
        ([0., -radius, 7.25], false),
        ([0., 2.65, -4.5], false),
        ([2.65, 0., -4.5], true),
    ] {
        assert_eq!(
            solid.rust_thread_contains(world(p, true)).unwrap(),
            is_material,
            "{p:?}"
        );
    }
    assert_eq!(solid.bounds().unwrap().min[0], 4.);
    assert_eq!(solid.bounds().unwrap().max[0], 22.);
    let snapshot = solid.topology_snapshot(request.feature, 1).unwrap();
    let tip = snapshot
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face
                && element.origins.iter().any(|o| o.role == "screw.thread/end")
        })
        .unwrap();
    let intent = GeometryIntent::capture(&snapshot, tip.key).unwrap();

    let MechanicalPartParameters::MachineScrew { thread, .. } = &mut source else {
        unreachable!()
    };
    thread.hand = ThreadHand::Right;
    let prepared =
        model::prepare_mechanical_part_edit(&session, 1, actor(), request.feature, &source)
            .unwrap();
    session.commit(prepared.prepared).unwrap();
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
    assert_eq!(rebuilt.snapshot().bodies[&request.body], request.feature);
    let current = rebuilt.solid(request.feature).unwrap();
    assert!(
        matches!(resolve(&intent, &current.topology_snapshot(request.feature, 0).unwrap(), 0), Resolution::Resolved { key, .. } if key == tip.key)
    );
    assert_eq!(current.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
    assert!(
        current
            .rust_thread_contains(world([0., radius, 6.75], true))
            .unwrap()
    );
    assert!(
        !current
            .rust_thread_contains(world([0., -radius, 6.75], true))
            .unwrap()
    );
}
