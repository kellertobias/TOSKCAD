use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{
        self, BodyInput, LeadScrewPreset, PrimitiveParameters, PrimitiveRequest, ThreadHand,
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
fn pair(
    row: &LeadScrewPreset,
    hand: ThreadHand,
    starts: u32,
) -> (Session, ThreadRequest, ThreadRequest) {
    let mut s = Session::new(Document::new("Lead screw mating geometry").unwrap()).unwrap();
    let mut p = row.parameters();
    p.hand = hand;
    p.starts = starts;
    p.length = length(4.37 * row.pitch_mm);
    p.radial_allowance = length(0.005);
    let male = ThreadRequest::new("Lead external", p.clone());
    let prepared = model::prepare_thread(&s, 0, actor(), &male).unwrap();
    s.commit(prepared.prepared).unwrap();
    let blank = PrimitiveRequest::new(
        "Lead nut blank",
        PrimitiveParameters::Cylinder {
            radius: length(row.internal_major_diameter_mm / 2. + 2.),
            height: p.length.clone(),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(prepared.prepared).unwrap();
    let female = ThreadRequest::receiving(
        "Lead internal",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        p,
    )
    .unwrap();
    let prepared = model::prepare_thread(&s, s.revision(), actor(), &female).unwrap();
    s.commit(prepared.prepared).unwrap();
    (s, male, female)
}
fn material(solid: &Solid, point: [f64; 3]) -> f64 {
    let edge = 0.0002;
    let cube = Solid::box_solid([edge; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|p| p - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / edge.powi(3)
}
fn mesh_material(raw: &mesh::ReadMesh, [x, y, z]: [f64; 3]) -> bool {
    let mut crossings = 0;
    for &(a, b, c) in &raw.world {
        let triangle = [a, b, c];
        let mut points = [[0.; 2]; 2];
        let mut count = 0;
        for i in 0..3 {
            let a = triangle[i];
            let b = triangle[(i + 1) % 3];
            if (a[2] <= z && z < b[2]) || (b[2] <= z && z < a[2]) {
                let t = (z - a[2]) / (b[2] - a[2]);
                if count < 2 {
                    points[count] = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
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
fn measured_crest_center(
    raw: &mesh::ReadMesh,
    radius: f64,
    phi: f64,
    around: f64,
    pitch: f64,
) -> f64 {
    let point = |z| [radius * phi.cos(), radius * phi.sin(), z];
    let start = around - 0.45 * pitch;
    let step = 0.9 * pitch / 64.;
    let mut changes = vec![];
    let mut previous = mesh_material(raw, point(start));
    for i in 1..=64 {
        let z = start + f64::from(i) * step;
        let inside = mesh_material(raw, point(z));
        if inside != previous {
            let mut lo = z - step;
            let mut hi = z;
            for _ in 0..20 {
                let mid = (lo + hi) / 2.;
                if mesh_material(raw, point(mid)) == previous {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            changes.push((lo + hi) / 2.);
        }
        previous = inside;
    }
    assert_eq!(
        changes.len(),
        2,
        "one bounded crest interval required: {changes:?}"
    );
    (changes[0] + changes[1]) / 2.
}
#[test]
fn sourced_pairs_and_left_hand_multi_start_have_measured_flanks_lead_step_print_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let rows = model::lead_screw_presets();
    let mut cases: Vec<_> = rows.iter().map(|r| (r, ThreadHand::Right, 1)).collect();
    cases.push((&rows[1], ThreadHand::Left, 2));
    for (index, (row, hand, starts)) in cases.into_iter().enumerate() {
        let (s, male, female) = pair(row, hand, starts);
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
        let volumes: Vec<_> = solids.iter().map(|s| s.volume_mm3().unwrap()).collect();
        for (i, (solid, request)) in solids.iter().zip([&male, &female]).enumerate() {
            assert_eq!(solid.solid_count().unwrap(), 1);
            let r = request
                .parameters
                .resolve(s.parameters(), request.mode)
                .unwrap();
            let center = 2. * row.pitch_mm;
            for sign in [-1., 1.] {
                for fraction in [0.2, 0.6] {
                    let drop = fraction * r.profile.depth_mm;
                    let z = center
                        + sign
                            * (r.profile.crest_width_mm / 2.
                                + drop * (row.included_angle_deg / 2.).to_radians().tan());
                    let boundary = r.major_radius_mm - drop;
                    for (offset, below) in [(-0.001, true), (0.001, false)] {
                        let wanted = if i == 0 { below } else { !below };
                        let actual = material(solid, [boundary + offset, 0., z]);
                        assert!(
                            if wanted {
                                actual > 0.999
                            } else {
                                actual < 1e-5
                            },
                            "{index}/{i}/{sign}/{fraction}/{offset}: {actual}"
                        );
                    }
                }
            }
            let topology = solid.topology_snapshot(request.feature, 0).unwrap();
            assert!(topology.elements.iter().all(|e| !e.origins.is_empty()));
            if i == 0 {
                for start in 0..starts {
                    for side in ["lower", "crest", "upper"] {
                        let role = format!("thread/start/{start}/{side}");
                        assert!(topology.elements.iter().any(|e| {
                            e.origins
                                .iter()
                                .any(|o| o.source == request.feature && o.role == role)
                        }));
                    }
                }
            }
            let bounds = solid.bounds().unwrap();
            assert!(bounds.min[2].abs() < 16e-7);
            assert!((bounds.max[2] - r.length_mm).abs() < 16e-7);
            let path = dir.path().join(format!("{index}-{i}.step"));
            solid.write_step(&path).unwrap();
            let back = Solid::read_step(&path, Default::default()).unwrap();
            let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            assert!(
                (back.volume_mm3().unwrap() - volumes[i]).abs()
                    < GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * volumes[i]
            );
        }
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(s.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let read = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let reopened = compiler().compile(&read.project.document, 0).unwrap();
        for (i, feature) in [male.feature, female.feature].into_iter().enumerate() {
            assert!(
                (reopened.solid(feature).unwrap().volume_mm3().unwrap() - volumes[i]).abs() < 1e-8
            );
        }
        let chord = 0.002;
        let print = PrintRequest {
            source: PrintSource::Parts {
                parts: vec![male.body, female.body],
            },
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality {
                chord_deviation_mm: chord,
                angular_deviation_deg: 5.,
            },
            stl_per_part: true,
        };
        let exporter = Exporter::default();
        let preview = exporter
            .print_preview(s.document(), s.revision(), &print, &actor(), None)
            .unwrap();
        assert!(preview.can_write(), "{:?}", preview.blocked);
        let stem = format!("lead-{index}");
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
                for (z, wanted) in [(2. * row.pitch_mm, true), (2.5 * row.pitch_mm, false)] {
                    assert_eq!(
                        mesh_material(&raw, [radius, 0., z]),
                        if external { wanted } else { !wanted }
                    );
                }
                if external && starts == 2 {
                    let radius = r.major_radius_mm - 0.1 * r.profile.depth_mm;
                    let z0 =
                        measured_crest_center(&raw, radius, 0., 2. * row.pitch_mm, row.pitch_mm);
                    let z1 =
                        measured_crest_center(&raw, radius, 0., 3. * row.pitch_mm, row.pitch_mm);
                    let phi = 0.43;
                    let shifted =
                        measured_crest_center(&raw, radius, phi, 2. * row.pitch_mm, row.pitch_mm);
                    let measured_lead = (shifted - z0) * std::f64::consts::TAU / phi;
                    let axial_error = 2. * chord / (row.included_angle_deg / 2.).to_radians().cos();
                    assert!((z1 - z0 - row.pitch_mm).abs() < axial_error);
                    assert!(
                        (measured_lead + row.pitch_mm * 2.).abs()
                            < axial_error * std::f64::consts::TAU / phi,
                        "measured signed LH lead {measured_lead}"
                    );
                    println!(
                        "measured LH multi-start: pitch={} mm, lead={measured_lead} mm/revolution, expected lead={} mm/revolution",
                        z1 - z0,
                        -2. * row.pitch_mm
                    );
                }
            }
        }
    }
}
#[test]
fn lead_screw_receiving_wall_and_cosmetic_manufacturing_refusal_remain_enforced() {
    let row = model::lead_screw_presets().remove(0);
    let (mut s, male, female) = pair(&row, ThreadHand::Right, 1);
    let mut p = female.parameters.clone();
    p.minimum_wall = length(3.);
    let change = model::prepare_thread_edit(&s, s.revision(), actor(), female.feature, &p).unwrap();
    assert!(
        compiler()
            .compile(change.prepared.document(), s.revision() + 1)
            .err()
            .unwrap()
            .to_string()
            .contains("minimum wall")
    );
    let mut p = male.parameters;
    p.representation = ThreadRepresentation::CosmeticEnvelope;
    let change = model::prepare_thread_edit(&s, s.revision(), actor(), male.feature, &p).unwrap();
    s.commit(change.prepared).unwrap();
    let request = PrintRequest {
        source: PrintSource::Parts {
            parts: vec![male.body],
        },
        aids: AidPolicy::IncludeAll,
        quality: Default::default(),
        stl_per_part: false,
    };
    assert!(
        Exporter::default()
            .print_preview(s.document(), s.revision(), &request, &actor(), None)
            .unwrap_err()
            .to_string()
            .contains("cosmetic")
    );
}
