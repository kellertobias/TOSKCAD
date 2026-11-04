use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{
        self, BodyInput, PrimitiveParameters, PrimitiveRequest, RoundThreadPreset, ThreadHand,
        ThreadMode, ThreadRepresentation, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{
    Compiler,
    export::{Exporter, mesh},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn pair(row: &RoundThreadPreset, hand: ThreadHand) -> (Session, ThreadRequest, ThreadRequest) {
    let mut s = Session::new(Document::new("Nominal Rd mating pair").unwrap()).unwrap();
    let mut p = row.parameters();
    p.hand = hand;
    p.length = length(3.37 * row.pitch_mm);
    p.radial_allowance = length(0.005);
    let male = ThreadRequest::new("Round external", p.clone());
    let q = model::prepare_thread(&s, 0, actor(), &male).unwrap();
    s.commit(q.prepared).unwrap();
    let blank = PrimitiveRequest::new(
        "Round nut blank",
        PrimitiveParameters::Cylinder {
            radius: length(row.diameter_mm / 2. + 2.5),
            height: p.length.clone(),
            frame: Default::default(),
        },
    );
    let q = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(q.prepared).unwrap();
    let female = ThreadRequest::receiving(
        "Round internal",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        p,
    )
    .unwrap();
    let q = model::prepare_thread(&s, s.revision(), actor(), &female).unwrap();
    s.commit(q.prepared).unwrap();
    (s, male, female)
}
fn material(shape: &Solid, point: [f64; 3]) -> f64 {
    let edge = 0.00002;
    let cube = Solid::box_solid([edge; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|v| v - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    shape
        .boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / edge.powi(3)
}
fn mesh_material(raw: &mesh::ReadMesh, [x, y, z]: [f64; 3]) -> bool {
    let mut crossings = 0;
    for &(a, b, c) in &raw.world {
        let t = [a, b, c];
        let mut points = [[0.; 2]; 2];
        let mut count = 0;
        for i in 0..3 {
            let a = t[i];
            let b = t[(i + 1) % 3];
            if (a[2] <= z && z < b[2]) || (b[2] <= z && z < a[2]) {
                let f = (z - a[2]) / (b[2] - a[2]);
                if count < 2 {
                    points[count] = [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1])];
                }
                count += 1;
            }
        }
        if count == 2 {
            let [a, b] = points;
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}
#[test]
fn named_round_native_pairs_preserve_unequal_cutter_circles_flanks_and_actual_exports_for_both_hands()
 {
    let dir = tempfile::tempdir().unwrap();
    let mut index = 0;
    for row in model::round_thread_presets() {
        for hand in [ThreadHand::Right, ThreadHand::Left] {
            let (s, male, female) = pair(&row, hand);
            let built = compiler().compile(s.document(), s.revision()).unwrap();
            let solids = [
                built.solid(male.feature).unwrap(),
                built.solid(female.feature).unwrap(),
            ];
            assert!(
                solids[0]
                    .boolean(solids[1], BooleanOperation::Intersect, Default::default())
                    .unwrap()
                    .is_none()
            );
            let mut volumes = vec![];
            for (i, request) in [&male, &female].into_iter().enumerate() {
                let shape = solids[i];
                assert_eq!(shape.solid_count().unwrap(), 1);
                let r = request
                    .parameters
                    .resolve(s.parameters(), request.mode)
                    .unwrap();
                let q = r.round.as_ref().unwrap();
                let volume = shape.volume_mm3().unwrap();
                volumes.push(volume);
                let path = dir.path().join(format!("{index}-{i}.step"));
                shape.write_step(&path).unwrap();
                let back = Solid::read_step(&path, Default::default()).unwrap();
                let area: f64 = shape.faces().unwrap().iter().map(|f| f.area_mm2).sum();
                assert!(
                    (back.volume_mm3().unwrap() - volume).abs()
                        < GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * volume
                );
                let bounds = shape.bounds().unwrap();
                assert!(bounds.min[2].abs() < 16e-7);
                assert!((bounds.max[2] - r.length_mm).abs() < 16e-7);
                assert!(
                    shape
                        .topology_snapshot(request.feature, 0)
                        .unwrap()
                        .elements
                        .iter()
                        .all(|e| !e.origins.is_empty())
                );
                for (representation, solid) in [("native", shape), ("step", &back)] {
                    for valley in [false, true] {
                        let circle = if valley {
                            &q.actual_cutter_root_circle
                        } else {
                            &q.actual_cutter_crest_circle
                        };
                        for fraction in [-0.65, 0.65] {
                            let dz = fraction * circle.radius_mm * 15_f64.to_radians().cos();
                            let rr = (circle.radius_mm.powi(2) - dz * dz).sqrt();
                            let boundary = circle.center_radius_mm + if valley { -rr } else { rr };
                            let z = 2. * r.pitch_mm + circle.center_axial_mm + dz;
                            for (dr, inside_profile) in [(-0.001, true), (0.001, false)] {
                                let actual = material(solid, [boundary + dr, 0., z]);
                                let wanted = if i == 0 {
                                    inside_profile
                                } else {
                                    !inside_profile
                                };
                                assert!(
                                    if wanted {
                                        actual > 0.999
                                    } else {
                                        actual < 1e-5
                                    },
                                    "circle {representation}/{index}/{i}/{valley}/{fraction}/{dr}: {actual}, point={:?}, boundary={boundary}, circle={circle:?}",
                                    [boundary + dr, 0., z]
                                );
                            }
                        }
                    }
                    let a = 15_f64.to_radians();
                    let k = (1. - a.sin()) / a.cos();
                    for drop in [0.22 * r.pitch_mm, 0.28 * r.pitch_mm] {
                        for direction in [-1., 1.] {
                            let z = 2. * r.pitch_mm
                                + direction * (drop * a.tan() + q.cutter_crest_radius_mm * k);
                            for (dz, inside_profile) in
                                [(-direction * 0.001, true), (direction * 0.001, false)]
                            {
                                let actual =
                                    material(solid, [r.major_radius_mm - drop, 0., z + dz]);
                                let wanted = if i == 0 {
                                    inside_profile
                                } else {
                                    !inside_profile
                                };
                                assert!(
                                    if wanted {
                                        actual > 0.999
                                    } else {
                                        actual < 1e-5
                                    },
                                    "flank {representation}/{index}/{i}/{drop}/{direction}: {actual}"
                                );
                            }
                        }
                    }
                }
            }
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
            .unwrap();
            let rebuilt = compiler().compile(&reopened.project.document, 0).unwrap();
            for (i, feature) in [male.feature, female.feature].into_iter().enumerate() {
                assert!(
                    (rebuilt.solid(feature).unwrap().volume_mm3().unwrap() - volumes[i]).abs()
                        < 1e-8
                );
            }
            let print = PrintRequest {
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
                .print_preview(s.document(), s.revision(), &print, &actor(), None)
                .unwrap();
            assert!(preview.can_write());
            let stem = format!("round-{index}");
            let receipt = exporter
                .write_print(
                    s.document(),
                    s.revision(),
                    &print,
                    &preview.key,
                    &dir.path().join(format!("{stem}.3mf")),
                    &actor(),
                    None,
                )
                .unwrap();
            assert_eq!(receipt.files.len(), 3);
            for file in receipt.files {
                let bytes = std::fs::read(&file.path).unwrap();
                let raw = if file.kind == PrintFileKind::Stl {
                    mesh::read_stl(&bytes).unwrap()
                } else {
                    mesh::read_three_mf(&bytes).unwrap()
                };
                assert_eq!(raw.triangles, file.triangles);
                assert!((raw.volume_mm3() - file.mesh_volume_mm3).abs() < 1e-6);
                if file.kind == PrintFileKind::Stl {
                    let plan = preview
                        .files
                        .iter()
                        .find(|p| {
                            std::path::Path::new(&file.path).file_name().unwrap()
                                == p.filename(&stem).as_str()
                        })
                        .unwrap();
                    let external = plan.part == Some(male.body);
                    let request = if external { &male } else { &female };
                    let r = request
                        .parameters
                        .resolve(s.parameters(), request.mode)
                        .unwrap();
                    let q = r.round.as_ref().unwrap();
                    for valley in [false, true] {
                        let circle = if valley {
                            &q.actual_cutter_root_circle
                        } else {
                            &q.actual_cutter_crest_circle
                        };
                        for fraction in [-0.65, 0.65] {
                            let dz = fraction * circle.radius_mm * 15_f64.to_radians().cos();
                            let rr = (circle.radius_mm.powi(2) - dz * dz).sqrt();
                            let boundary = circle.center_radius_mm + if valley { -rr } else { rr };
                            let z = 2. * r.pitch_mm + circle.center_axial_mm + dz;
                            for (dr, inside_profile) in [(-0.01, true), (0.01, false)] {
                                for phi in [0_f64, 0.43] {
                                    let hand = if r.hand == ThreadHand::Right { 1. } else { -1. };
                                    let radial = boundary + dr;
                                    let point = [
                                        radial * phi.cos(),
                                        radial * phi.sin(),
                                        z + hand * phi * r.pitch_mm / std::f64::consts::TAU,
                                    ];
                                    assert_eq!(
                                        mesh_material(&raw, point),
                                        if external {
                                            inside_profile
                                        } else {
                                            !inside_profile
                                        }
                                    );
                                }
                            }
                        }
                    }
                }
            }
            index += 1;
        }
    }
}
#[test]
fn round_receiving_wall_and_cosmetic_manufacturing_refusals_are_explicit() {
    let row = model::round_thread_presets().remove(0);
    let (mut s, male, female) = pair(&row, ThreadHand::Right);
    let mut p = female.parameters;
    p.minimum_wall = length(3.);
    let edit = model::prepare_thread_edit(&s, s.revision(), actor(), female.feature, &p).unwrap();
    assert!(
        compiler()
            .compile(edit.prepared.document(), s.revision() + 1)
            .err()
            .unwrap()
            .to_string()
            .contains("minimum wall")
    );
    let mut p = male.parameters;
    p.representation = ThreadRepresentation::CosmeticEnvelope;
    let edit = model::prepare_thread_edit(&s, s.revision(), actor(), male.feature, &p).unwrap();
    s.commit(edit.prepared).unwrap();
    let print = PrintRequest {
        source: PrintSource::Parts {
            parts: vec![male.body],
        },
        aids: AidPolicy::IncludeAll,
        quality: Default::default(),
        stl_per_part: false,
    };
    assert!(
        Exporter::default()
            .print_preview(s.document(), s.revision(), &print, &actor(), None)
            .err()
            .unwrap()
            .to_string()
            .contains("cosmetic")
    );
}
