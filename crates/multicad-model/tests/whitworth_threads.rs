use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    exchange::*,
    model::{
        self, BodyInput, PrimitiveParameters, PrimitiveRequest, ResolvedThread, ThreadFamily,
        ThreadFit, ThreadHand, ThreadMode, ThreadParameters, ThreadRepresentation, ThreadRequest,
        length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{
    Compiler,
    export::{Exporter, mesh},
};
use std::{f64::consts::PI, path::Path};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Rounded thread metrology").unwrap()).unwrap()
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
// Independently selected physical basic diameters/TPI and sourced diameter
// limits; G1/4 is a pipe designation, never a 1/4-inch physical diameter.
struct Case {
    family: ThreadFamily,
    diameter: f64,
    tpi: u32,
    external: [[Option<f64>; 2]; 3],
    internal: [[Option<f64>; 2]; 3],
    scale: f64,
}
fn cases() -> [Case; 3] {
    [
        Case {
            family: ThreadFamily::Bsw,
            diameter: 6.35,
            tpi: 20,
            scale: 25.4,
            external: [
                [Some(0.2427), Some(0.2488)],
                [Some(0.2129), Some(0.2168)],
                [Some(0.1764), Some(0.1848)],
            ],
            internal: [
                [Some(0.25), None],
                [Some(0.218), Some(0.2219)],
                [Some(0.186), Some(0.203)],
            ],
        },
        Case {
            family: ThreadFamily::Bsf,
            diameter: 6.35,
            tpi: 26,
            scale: 25.4,
            external: [
                [Some(0.2432), Some(0.2489)],
                [Some(0.2206), Some(0.2243)],
                [Some(0.1921), Some(0.1997)],
            ],
            internal: [
                [Some(0.25), None],
                [Some(0.2254), Some(0.2291)],
                [Some(0.2008), Some(0.2125)],
            ],
        },
        Case {
            family: ThreadFamily::Bspp,
            diameter: 13.157,
            tpi: 19,
            scale: 1.,
            external: [
                [Some(12.907), Some(13.157)],
                [Some(12.051), Some(12.301)],
                [Some(11.133), Some(11.445)],
            ],
            internal: [
                [Some(13.157), None],
                [Some(12.301), Some(12.426)],
                [Some(11.445), Some(11.89)],
            ],
        },
    ]
}
fn parameters(case: &Case, internal: bool) -> ThreadParameters {
    let pitch = 25.4 / f64::from(case.tpi);
    ThreadParameters {
        family: case.family,
        fit: match (case.family, internal) {
            (ThreadFamily::Bspp, false) => ThreadFit::BsppClassBExternal,
            (ThreadFamily::Bspp, true) => ThreadFit::BsppInternal,
            (_, false) => ThreadFit::WhitworthMediumExternal,
            (_, true) => ThreadFit::WhitworthMediumInternal,
        },
        major_diameter: length(case.diameter),
        pitch: length(pitch),
        length: length(3. * pitch),
        ..Default::default()
    }
}
fn commit_thread(s: &mut Session, request: &ThreadRequest) {
    let prepared = model::prepare_thread(s, s.revision(), actor(), request).unwrap();
    s.commit(prepared.prepared).unwrap();
}
fn cube_material(solid: &Solid, point: [f64; 3], offset: f64) -> bool {
    let size = offset * 0.2;
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    let fraction = solid
        .boolean(
            &cube,
            BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .map_or(0., |solid| solid.volume_mm3().unwrap())
        / size.powi(3);
    assert!(
        fraction < 1e-5 || fraction > 0.999,
        "Probe must lie wholly on one side, got {fraction}"
    );
    fraction > 0.999
}
fn material_at(solid: &Solid, radius: f64, z: f64, inside: bool, offset: f64) {
    assert_eq!(
        cube_material(solid, [radius, 0., z], offset),
        inside,
        "r={radius}, z={z}"
    );
}
fn independent_form(pitch: f64) -> (f64, f64) {
    let theta = 27.5_f64.to_radians();
    (
        pitch / (3. * theta.tan()),
        pitch * theta.cos() / (12. * (1. - theta.sin())),
    )
}
fn verify_true_circular_crest_and_valley(solid: &Solid, resolved: &ResolvedThread, internal: bool) {
    let p = resolved.pitch_mm;
    let major = resolved.major_radius_mm;
    let (depth, r) = independent_form(p);
    let a = r * 27.5_f64.to_radians().cos();
    let offset = (0.02 * r).min(0.002);
    assert!((resolved.profile.rounding_radius_mm.unwrap() - r).abs() < 1e-12);
    assert!((resolved.profile.depth_mm - depth).abs() < 1e-12);
    // Fixed-azimuth axial sections have actual circle centers (R-r,P) and
    // (R-h+r,1.5P). Probe both sides of three nontrivial circular points at
    // each crest AND valley. A polygon, straight flank or flat valley fails.
    for fraction in [0.4, 0.65, 0.85] {
        let dz = fraction * a;
        let crest = major - r + (r * r - dz * dz).sqrt();
        let valley = major - depth + r - (r * r - dz * dz).sqrt();
        for sign in [-1., 1.] {
            material_at(solid, crest - offset, p + sign * dz, !internal, offset);
            material_at(solid, crest + offset, p + sign * dz, internal, offset);
            material_at(
                solid,
                valley - offset,
                1.5 * p + sign * dz,
                !internal,
                offset,
            );
            material_at(
                solid,
                valley + offset,
                1.5 * p + sign * dz,
                internal,
                offset,
            );
        }
    }
    // Independently measure both actual major and minor extrema as well.
    material_at(solid, major - offset, p, !internal, offset);
    material_at(solid, major + offset, p, internal, offset);
    material_at(solid, major - depth - offset, 1.5 * p, !internal, offset);
    material_at(solid, major - depth + offset, 1.5 * p, internal, offset);
    for z in [1.25 * p, 1.75 * p] {
        material_at(solid, major - depth / 2. - offset, z, !internal, offset);
        material_at(solid, major - depth / 2. + offset, z, internal, offset);
    }
    // Quarter-turn phase distinguishes handedness, independently of radii.
    let pd_radius = major - depth / 2.;
    let y = if resolved.hand == ThreadHand::Left {
        -pd_radius
    } else {
        pd_radius
    };
    assert_eq!(cube_material(solid, [0., y, 1.25 * p], offset), !internal);
    assert_eq!(cube_material(solid, [0., -y, 1.25 * p], offset), internal);
}
fn verify_fit(case: &Case, resolved: &ResolvedThread, internal: bool) {
    let fit = resolved.fit.as_ref().unwrap();
    let limits = if internal {
        case.internal
    } else {
        case.external
    };
    for (actual, [min, max]) in [
        fit.major_diameter_mm,
        fit.pitch_diameter_mm,
        fit.minor_diameter_mm,
    ]
    .into_iter()
    .zip(limits)
    {
        if let Some(min) = min {
            assert!(actual >= min * case.scale - 1e-10);
        }
        if let Some(max) = max {
            assert!(actual <= max * case.scale + 1e-10);
        }
    }
    assert!((fit.major_diameter_mm - 2. * resolved.major_radius_mm).abs() < 1e-12);
    assert!(
        (fit.minor_diameter_mm - 2. * (resolved.major_radius_mm - resolved.profile.depth_mm)).abs()
            < 1e-12
    );
    assert_eq!(
        fit.pitch_diameter_mm,
        resolved.actual_pitch_diameter_mm.unwrap()
    );
    if case.family == ThreadFamily::Bspp {
        assert_eq!(fit.engagement_limit_mm, None);
    } else {
        assert!((fit.engagement_limit_mm.unwrap() - 15. * resolved.pitch_mm).abs() < 1e-12);
    }
}
fn radial_section(z: f64, p: f64, major: f64) -> f64 {
    let (h, r) = independent_form(p);
    let theta = 27.5_f64.to_radians();
    let a = r * theta.cos();
    let q = z.rem_euclid(p).min(p - z.rem_euclid(p));
    if q <= a {
        major - r + (r * r - q * q).sqrt()
    } else if q >= p / 2. - a {
        let dz = p / 2. - q;
        major - h + r - (r * r - dz * dz).sqrt()
    } else {
        major - r * (1. - theta.sin()) - (q - a) / theta.tan()
    }
}
fn independent_area(p: f64, major: f64) -> f64 {
    // Composite Simpson integration of the independent circle/line section.
    let n = 4096;
    let f = |i: usize| radial_section(i as f64 * p / n as f64, p, major).powi(2);
    let weighted = f(0)
        + f(n)
        + (1..n)
            .map(|i| if i % 2 == 0 { 2. * f(i) } else { 4. * f(i) })
            .sum::<f64>();
    PI * weighted / (3. * n as f64)
}
fn assert_volume(solid: &Solid, expected: f64) {
    let measured = solid.volume_mm3().unwrap();
    let area = solid
        .faces()
        .unwrap()
        .iter()
        .map(|f| f.area_mm2)
        .sum::<f64>();
    let budget = GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * expected.abs();
    assert!(
        (measured - expected).abs() <= budget,
        "{measured} != {expected}, native-policy budget {budget}"
    );
}
fn assert_mesh_bounds(mesh: &mesh::ReadMesh, shapes: &[&Solid], chord: f64) {
    let mut expected = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for shape in shapes {
        let bounds = shape.bounds().unwrap();
        for (minimum, value) in expected[0].iter_mut().zip(bounds.min) {
            *minimum = minimum.min(value);
        }
        for (maximum, value) in expected[1].iter_mut().zip(bounds.max) {
            *maximum = maximum.max(value);
        }
    }
    let actual = mesh
        .bounds_mm()
        .expect("Actual exported mesh must have bounds");
    let scale = expected
        .into_iter()
        .flatten()
        .map(f64::abs)
        .fold(1., f64::max);
    // The same explicit dimensional budget covers f32 STL and higher-precision
    // 3MF serialization; volume agreement alone cannot detect a misplaced build.
    let budget = chord
        + 2. * f64::from(f32::EPSILON) * scale
        + 16. * GeometryPolicy::default().linear_tolerance_mm;
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        assert!(
            (actual - expected).abs() <= budget,
            "Mesh bound {actual} differs from native {expected}; chord/serialization budget {budget}"
        );
    }
}

fn native_pair(case: &Case) -> (Session, ThreadRequest, ThreadRequest, f64) {
    let mut s = session();
    let external = ThreadRequest::new("Rounded male", parameters(case, false));
    commit_thread(&mut s, &external);
    let receiving = parameters(case, true);
    let resolved = receiving
        .resolve(s.parameters(), ThreadMode::CutInternal)
        .unwrap();
    let outside = resolved.major_radius_mm + 2.;
    let blank = PrimitiveRequest::new(
        "Rounded receiver",
        PrimitiveParameters::Cylinder {
            radius: length(outside),
            height: length(resolved.length_mm),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(prepared.prepared).unwrap();
    let internal = ThreadRequest::receiving(
        "Rounded female",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        receiving,
    )
    .unwrap();
    commit_thread(&mut s, &internal);
    (s, external, internal, outside)
}
fn check_step(solid: &Solid, path: &Path) -> Solid {
    solid.write_step(path).unwrap();
    let back = Solid::read_step(path, GeometryPolicy::default()).unwrap();
    assert_eq!(back.solid_count().unwrap(), 1);
    back
}
fn mesh_material(mesh: &mesh::ReadMesh, radius: f64, angle: f64, z: f64) -> bool {
    let x = radius * angle.cos();
    let y = radius * angle.sin();
    let mut crossings = 0;
    // Section actual exported triangles and classify by 2D crossing parity.
    // Neither the native classifier nor receipts supply material membership.
    for &(a, b, c) in &mesh.world {
        let triangle = [a, b, c];
        let mut points = Vec::new();
        for i in 0..3 {
            let a = triangle[i];
            let b = triangle[(i + 1) % 3];
            if (a[2] <= z && z < b[2]) || (b[2] <= z && z < a[2]) {
                let t = (z - a[2]) / (b[2] - a[2]);
                points.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
            }
        }
        if points.len() == 2 {
            let [a, b] = [points[0], points[1]];
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}

#[test]
fn each_rounded_series_has_real_circular_crests_and_valleys_fit_clearance_and_step_mesh_exports() {
    let dir = tempfile::tempdir().unwrap();
    for (index, case) in cases().into_iter().enumerate() {
        let (s, male, female, outside) = native_pair(&case);
        let built = compiler().compile(s.document(), s.revision()).unwrap();
        let male_solid = built.solid(male.feature).unwrap();
        let female_solid = built.solid(female.feature).unwrap();
        assert!(
            male_solid
                .boolean(
                    female_solid,
                    BooleanOperation::Intersect,
                    GeometryPolicy::default()
                )
                .unwrap()
                .is_none(),
            "Sourced full-form pair must have geometric clearance"
        );
        let male_resolved = male
            .parameters
            .resolve(s.parameters(), ThreadMode::NewExternal)
            .unwrap();
        let female_resolved = female
            .parameters
            .resolve(s.parameters(), ThreadMode::CutInternal)
            .unwrap();
        for (internal, solid, resolved) in [
            (false, male_solid, &male_resolved),
            (true, female_solid, &female_resolved),
        ] {
            assert_eq!(solid.solid_count().unwrap(), 1);
            verify_fit(&case, resolved, internal);
            let topology = solid
                .topology_snapshot(
                    if internal {
                        female.feature
                    } else {
                        male.feature
                    },
                    s.revision(),
                )
                .unwrap();
            assert!(
                topology
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
                    .all(|e| !e.origins.is_empty())
            );
            let path = dir.path().join(format!("series{index}-{internal}.step"));
            let back = check_step(solid, &path);
            let thread_volume =
                independent_area(resolved.pitch_mm, resolved.major_radius_mm) * resolved.length_mm;
            let expected = if internal {
                PI * outside * outside * resolved.length_mm - thread_volume
            } else {
                thread_volume
            };
            for shape in [solid, &back] {
                verify_true_circular_crest_and_valley(shape, resolved, internal);
                assert_volume(shape, expected);
            }
        }
        let r = (male_resolved.major_radius_mm + female_resolved.major_radius_mm) / 2.;
        let epsilon = 0.001;
        assert!(!cube_material(
            male_solid,
            [r, 0., male_resolved.pitch_mm],
            epsilon
        ));
        assert!(!cube_material(
            female_solid,
            [r, 0., male_resolved.pitch_mm],
            epsilon
        ));
        let request = PrintRequest {
            source: PrintSource::Parts {
                parts: vec![male.body, female.body],
            },
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality {
                chord_deviation_mm: 0.003,
                angular_deviation_deg: 5.,
            },
            stl_per_part: true,
        };
        let exporter = Exporter::default();
        let preview = exporter
            .print_preview(s.document(), s.revision(), &request, &actor(), None)
            .unwrap();
        assert!(preview.can_write(), "{:?}", preview.blocked);
        let stem = format!("rounded-{index}");
        let receipt = exporter
            .write_print(
                s.document(),
                s.revision(),
                &request,
                &preview.key,
                &dir.path().join(format!("{stem}.3mf")),
                &actor(),
                None,
            )
            .unwrap();
        assert_eq!(receipt.files.len(), 3);
        for file in receipt.files {
            let bytes = std::fs::read(&file.path).unwrap();
            let mesh = match file.kind {
                PrintFileKind::ThreeMf => mesh::read_three_mf(&bytes).unwrap(),
                PrintFileKind::Stl => mesh::read_stl(&bytes).unwrap(),
            };
            assert_eq!(mesh.unit_scale, 1.);
            assert_eq!(mesh.triangles, file.triangles);
            assert!(mesh.triangles > 0);
            assert!(
                mesh.world
                    .iter()
                    .flat_map(|(a, b, c)| [a, b, c])
                    .flat_map(|p| p.iter())
                    .all(|v| v.is_finite())
            );
            assert!((mesh.volume_mm3() - file.mesh_volume_mm3).abs() < 1e-6);
            let plan = preview
                .files
                .iter()
                .find(|plan| {
                    Path::new(&file.path).file_name().unwrap() == plan.filename(&stem).as_str()
                })
                .unwrap();
            let shapes = if file.kind == PrintFileKind::ThreeMf {
                vec![male_solid, female_solid]
            } else if plan.part == Some(female.body) {
                vec![female_solid]
            } else {
                assert_eq!(plan.part, Some(male.body));
                vec![male_solid]
            };
            assert_mesh_bounds(&mesh, &shapes, request.quality.chord_deviation_mm);
            let (expected, area) = if file.kind == PrintFileKind::ThreeMf {
                assert_eq!((mesh.objects, mesh.items), (2, 2));
                let area = male_solid
                    .faces()
                    .unwrap()
                    .iter()
                    .map(|f| f.area_mm2)
                    .sum::<f64>()
                    + female_solid
                        .faces()
                        .unwrap()
                        .iter()
                        .map(|f| f.area_mm2)
                        .sum::<f64>();
                (
                    male_solid.volume_mm3().unwrap() + female_solid.volume_mm3().unwrap(),
                    area,
                )
            } else {
                let internal = plan.part == Some(female.body);
                assert!(internal || plan.part == Some(male.body));
                let (shape, resolved) = if internal {
                    (female_solid, &female_resolved)
                } else {
                    (male_solid, &male_resolved)
                };
                let z = 1.137 * resolved.pitch_mm;
                let angle = 2. * PI * z / resolved.pitch_mm;
                let radius = resolved.actual_pitch_diameter_mm.unwrap() / 2.;
                assert_eq!(mesh_material(&mesh, radius, angle, z), !internal);
                assert_eq!(mesh_material(&mesh, radius, angle + PI, z), internal);
                (
                    shape.volume_mm3().unwrap(),
                    shape
                        .faces()
                        .unwrap()
                        .iter()
                        .map(|f| f.area_mm2)
                        .sum::<f64>(),
                )
            };
            assert!(
                (mesh.volume_mm3() - expected).abs()
                    <= request.quality.chord_deviation_mm * area + 1e-5 * expected.abs()
            );
        }
    }
}

#[test]
fn rounded_edits_history_and_independent_reopen_retain_fit_phase_and_curvature() {
    let case = cases().into_iter().nth(1).unwrap();
    let mut s = session();
    let request = ThreadRequest::new("Editable BSF", parameters(&case, false));
    commit_thread(&mut s, &request);
    let original = s.document().clone();
    let mut changed = request.parameters.clone();
    changed.length = length(3.25 * 25.4 / f64::from(case.tpi));
    changed.hand = ThreadHand::Left;
    let prepared =
        model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &changed).unwrap();
    let built = compiler()
        .compile(prepared.prepared.document(), s.revision() + 1)
        .unwrap();
    assert_eq!(s.document(), &original);
    let resolved = changed
        .resolve(s.parameters(), ThreadMode::NewExternal)
        .unwrap();
    verify_true_circular_crest_and_valley(built.solid(request.feature).unwrap(), &resolved, false);
    s.commit(prepared.prepared).unwrap();
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
    let project = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let fresh = Session::new(project.document).unwrap();
    assert_eq!(fresh.revision(), 0);
    assert_eq!(
        model::read_thread(fresh.document(), request.feature)
            .unwrap()
            .parameters,
        changed
    );
    let reopened = compiler().compile(fresh.document(), 0).unwrap();
    assert_eq!(reopened.snapshot().bodies[&request.body], request.feature);
    verify_true_circular_crest_and_valley(
        reopened.solid(request.feature).unwrap(),
        &resolved,
        false,
    );
}

#[test]
fn unsupported_rounded_recipes_and_insufficient_receiving_wall_refuse_without_partial_mutation() {
    for case in cases() {
        let s = session();
        let before = s.document().clone();
        for change in 0..7 {
            let mut p = parameters(&case, false);
            match change {
                0 => p.starts = 2,
                1 => p.axial_allowance = length(0.01),
                2 => p.fit = ThreadFit::NominalBasic,
                3 => p.pitch = length(25.4 / f64::from(case.tpi) + 0.01),
                4 => p.radial_allowance = length(1.),
                5 => p.family = ThreadFamily::Whitworth,
                _ => p.fit = parameters(&case, true).fit,
            }
            let request = ThreadRequest::new("Unavailable rounded recipe", p);
            assert!(
                model::prepare_thread(&s, 0, actor(), &request).is_err(),
                "accepted invalid case {change}"
            );
            assert_eq!(s.document(), &before);
        }
        let mut s = session();
        let p = parameters(&case, true);
        let resolved = p.resolve(s.parameters(), ThreadMode::CutInternal).unwrap();
        let blank = PrimitiveRequest::new(
            "Insufficient receiver",
            PrimitiveParameters::Cylinder {
                radius: length(resolved.major_radius_mm + 0.5),
                height: length(resolved.length_mm),
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&s, 0, actor(), &blank).unwrap();
        s.commit(prepared.prepared).unwrap();
        let before = s.document().clone();
        let request = ThreadRequest::receiving(
            "Rejected wall",
            ThreadMode::CutInternal,
            BodyInput {
                body: blank.body,
                producer: blank.feature,
            },
            p,
        )
        .unwrap();
        if let Ok(prepared) = model::prepare_thread(&s, s.revision(), actor(), &request) {
            let error = compiler()
                .compile(prepared.prepared.document(), s.revision() + 1)
                .err()
                .expect("Native receiving-wall preflight must fail");
            assert!(error.message.contains("minimum wall"), "{error}");
        }
        assert_eq!(s.document(), &before);
    }
}

#[test]
fn cosmetic_rounded_families_are_refused_by_manufacturing_and_print_writers() {
    for case in cases() {
        let mut s = session();
        let mut p = parameters(&case, false);
        p.representation = ThreadRepresentation::CosmeticEnvelope;
        let request = ThreadRequest::new("Cosmetic rounded thread", p);
        commit_thread(&mut s, &request);
        let id = EntityId::new();
        let prepared = prepare_set(
            &s,
            s.revision(),
            actor(),
            id,
            "Manufacturing",
            &SetDefinition {
                recipes: vec![
                    Recipe::step("Rounded STEP"),
                    Recipe {
                        id: EntityId::new(),
                        name: "Rounded STL".into(),
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
            .preview(s.document(), s.revision(), id, &actor(), None)
            .unwrap();
        assert!(!preview.can_run());
        assert!(preview.artifacts.is_empty());
        assert!(preview.recipes.iter().all(
            |r| matches!(&r.status,RecipeStatus::Unsupported(reason) if reason.contains("cosmetic"))
        ));
        let directory = tempfile::tempdir().unwrap();
        assert!(
            exporter
                .run(
                    s.document(),
                    s.revision(),
                    &preview.key,
                    directory.path(),
                    &actor(),
                    None
                )
                .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        let print = PrintRequest {
            source: PrintSource::Parts {
                parts: vec![request.body],
            },
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality::default(),
            stl_per_part: true,
        };
        assert!(
            exporter
                .print_preview(s.document(), s.revision(), &print, &actor(), None)
                .unwrap_err()
                .message
                .contains("cosmetic")
        );
    }
}
