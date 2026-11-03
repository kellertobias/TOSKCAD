use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{
        self, BodyInput, ButtressLoadFlankSide, PrimitiveParameters, PrimitiveRequest, ThreadHand,
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
fn pair(side: ButtressLoadFlankSide, hand: ThreadHand) -> (Session, ThreadRequest, ThreadRequest) {
    let mut s = Session::new(Document::new("Named buttress pair").unwrap()).unwrap();
    let mut p = model::buttress_thread_presets().remove(0).parameters(side);
    p.hand = hand;
    p.length = length(13.48);
    p.radial_allowance = length(0.005);
    let male = ThreadRequest::new("Buttress external", p.clone());
    let q = model::prepare_thread(&s, 0, actor(), &male).unwrap();
    s.commit(q.prepared).unwrap();
    let blank = PrimitiveRequest::new(
        "Buttress nut blank",
        PrimitiveParameters::Cylinder {
            radius: length(12.),
            height: p.length.clone(),
            frame: Default::default(),
        },
    );
    let q = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(q.prepared).unwrap();
    let female = ThreadRequest::receiving(
        "Buttress internal",
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
            translation_mm: point.map(|x| x - edge / 2.),
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
fn named_buttress_native_pairs_preserve_load_flank_phase_root_and_flat_cutter_for_both_hands() {
    let dir = tempfile::tempdir().unwrap();
    for (index, (side, hand)) in [
        (ButtressLoadFlankSide::NegativeAxisSide, ThreadHand::Right),
        (ButtressLoadFlankSide::NegativeAxisSide, ThreadHand::Left),
        (ButtressLoadFlankSide::PositiveAxisSide, ThreadHand::Right),
        (ButtressLoadFlankSide::PositiveAxisSide, ThreadHand::Left),
    ]
    .into_iter()
    .enumerate()
    {
        let (s, male, female) = pair(side, hand);
        let built = compiler()
            .compile(s.document(), s.revision())
            .unwrap_or_else(|e| panic!("{index}: {e}"));
        let solids = [
            built.solid(male.feature).unwrap(),
            built.solid(female.feature).unwrap(),
        ];
        assert!(
            solids[0]
                .boolean(solids[1], BooleanOperation::Intersect, Default::default())
                .unwrap()
                .is_none(),
            "mating {index}"
        );
        let volumes: Vec<_> = solids.iter().map(|s| s.volume_mm3().unwrap()).collect();
        let mut stepped = vec![];
        for (i, (shape, request)) in solids.iter().zip([&male, &female]).enumerate() {
            assert_eq!(shape.solid_count().unwrap(), 1);
            let before_mesh = shape.to_brep().unwrap();
            let coarse = shape
                .mesh(Default::default())
                .unwrap_or_else(|e| panic!("default mesh {index}/{i}: {e}"));
            assert_eq!(coarse, shape.mesh(Default::default()).unwrap());
            assert_eq!(
                coarse
                    .face_indices
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                shape.faces().unwrap().len()
            );
            assert!(
                shape
                    .mesh(multicad_kernel::MeshOptions {
                        maximum_triangles: 1,
                        ..Default::default()
                    })
                    .is_err()
            );
            shape
                .mesh(multicad_kernel::MeshOptions {
                    linear_deflection_mm: 0.003,
                    angular_deflection_rad: 5_f64.to_radians(),
                    ..Default::default()
                })
                .unwrap_or_else(|e| panic!("mesh {index}/{i}: {e}"));
            assert_eq!(shape.to_brep().unwrap(), before_mesh);
            let path = dir.path().join(format!("{index}-{i}.step"));
            shape.write_step(&path).unwrap();
            let back = Solid::read_step(&path, Default::default()).unwrap();
            let area: f64 = shape.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            assert!(
                (back.volume_mm3().unwrap() - volumes[i]).abs()
                    < GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * volumes[i]
            );
            let bounds = shape.bounds().unwrap();
            assert!(bounds.min[2].abs() < 16e-7);
            assert!((bounds.max[2] - 13.48).abs() < 16e-7);
            let names = shape.topology_snapshot(request.feature, 0).unwrap();
            assert!(names.elements.iter().all(|e| !e.origins.is_empty()));
            stepped.push(back);
        }
        for (i, request) in [&male, &female].into_iter().enumerate() {
            let resolved = request
                .parameters
                .resolve(s.parameters(), request.mode)
                .unwrap();
            let b = resolved.buttress.as_ref().unwrap();
            assert_eq!(b.intent.load_flank_side, side);
            assert_eq!(resolved.hand, hand);
            for shape in [solids[i], &stepped[i]] {
                // Independently check both asymmetric lines at two radii in
                // the fixed axial meridian. Wrong hand-dependent phase fails.
                for (direction, angle) in [(-1., b.lower_flank_deg), (1., b.upper_flank_deg)] {
                    for radius in [8., 9.] {
                        let z = 8.
                            + b.crest_phase_axial_mm
                            + direction
                                * (b.crest_width_mm / 2.
                                    + (resolved.major_radius_mm - radius)
                                        * angle.to_radians().tan());
                        for (offset, inside_positive_profile) in
                            [(-direction * 0.001, true), (direction * 0.001, false)]
                        {
                            let fraction = material(shape, [radius, 0., z + offset]);
                            let wanted = if i == 0 {
                                inside_positive_profile
                            } else {
                                !inside_positive_profile
                            };
                            assert!(
                                if wanted {
                                    fraction > 0.999
                                } else {
                                    fraction < 1e-5
                                },
                                "flank {index}/{i}/{direction}/{radius}/{offset}: {fraction}"
                            );
                        }
                    }
                }
                if let Some(circle) = &b.root_circle {
                    for angle in [-0.5_f64, 0.5] {
                        let z = 8. + circle.center_axial_mm + circle.radius_mm * angle.sin();
                        let boundary = circle.center_radius_mm - circle.radius_mm * angle.cos();
                        assert!(material(shape, [boundary - 0.001, 0., z]) > 0.999);
                        assert!(material(shape, [boundary + 0.001, 0., z]) < 1e-5);
                    }
                } else {
                    let root_z = 8. + b.crest_phase_axial_mm;
                    assert!(material(shape, [resolved.major_radius_mm - 0.001, 0., root_z]) < 1e-5);
                    assert!(
                        material(shape, [resolved.major_radius_mm + 0.001, 0., root_z]) > 0.999
                    );
                    let lower = 4.
                        - b.crest_width_mm / 2.
                        - b.depth_mm * b.lower_flank_deg.to_radians().tan();
                    let upper =
                        b.crest_width_mm / 2. + b.depth_mm * b.upper_flank_deg.to_radians().tan();
                    let z = 8. + b.crest_phase_axial_mm + (lower + upper) / 2.;
                    let radius = resolved.major_radius_mm - b.depth_mm;
                    assert!(material(shape, [radius - 0.001, 0., z]) < 1e-5);
                    assert!(material(shape, [radius + 0.001, 0., z]) > 0.999);
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
                (rebuilt.solid(feature).unwrap().volume_mm3().unwrap() - volumes[i]).abs() < 1e-8
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
        assert!(preview.can_write(), "{:?}", preview.blocked);
        let stem = format!("buttress-{index}");
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
                let r = male.parameters.resolve(s.parameters(), male.mode).unwrap();
                let radius = r.major_radius_mm - r.profile.depth_mm / 2.;
                for (z, wanted) in [(8., true), (10., false)] {
                    assert_eq!(
                        mesh_material(&raw, [radius, 0., z]),
                        if external { wanted } else { !wanted }
                    );
                }
                if external {
                    let circle = r.buttress.unwrap().root_circle.unwrap();
                    for angle in [-0.5_f64, 0.5] {
                        let z = 8. + circle.center_axial_mm + circle.radius_mm * angle.sin();
                        let boundary = circle.center_radius_mm - circle.radius_mm * angle.cos();
                        assert!(mesh_material(&raw, [boundary - 0.01, 0., z]));
                        assert!(!mesh_material(&raw, [boundary + 0.01, 0., z]));
                    }
                }
            }
        }
    }
}
#[test]
fn buttress_wall_and_cosmetic_manufacturing_failures_remain_explicit() {
    let (mut s, male, female) = pair(ButtressLoadFlankSide::NegativeAxisSide, ThreadHand::Right);
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
            .unwrap_err()
            .to_string()
            .contains("cosmetic")
    );
}
