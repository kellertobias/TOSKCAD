use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    exchange::{AidPolicy, MeshQuality, PrintFileKind, PrintRequest, PrintSource},
    model::{
        self, BodyInput, PipeGaugeConvention, PipeThreadForm, PipeThreadIntent, PipeThreadSize,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, ThreadMode, ThreadRepresentation,
        ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{BooleanOperation, Solid};
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
fn mesh_material(raw: &mesh::ReadMesh, point: [f64; 3]) -> bool {
    let [x, y, z] = point;
    let mut crossings = 0;
    for &(a, b, c) in &raw.world {
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
            let (a, b) = (points[0], points[1]);
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}
fn pair(size: PipeThreadSize) -> (Session, ThreadRequest, ThreadRequest) {
    let mut s = Session::new(Document::new("R/Rc gauge-aligned pair").unwrap()).unwrap();
    let intent = |form| PipeThreadIntent {
        form,
        size,
        gauge: PipeGaugeConvention::Iso7Nominal1994,
    };
    let mut external = intent(PipeThreadForm::R).parameters();
    external.radial_allowance = length(0.005);
    external.length = length(intent(PipeThreadForm::R).default_length_mm() + 0.005 * 32.);
    let male = ThreadRequest::new("R external", external);
    let p = model::prepare_thread(&s, s.revision(), actor(), &male).unwrap();
    s.commit(p.prepared).unwrap();
    let mut internal = intent(PipeThreadForm::Rc).parameters();
    internal.radial_allowance = length(0.005);
    // Opposing axes: align gauge planes and the same helical phase, rather
    // than merely intersecting two coincident parallel-thread fixtures.
    let origin =
        intent(PipeThreadForm::R).gauge_offset_mm() + intent(PipeThreadForm::Rc).gauge_offset_mm();
    let phi = std::f64::consts::TAU * origin / intent(PipeThreadForm::R).preset().pitch_mm;
    internal.frame = PrimitiveFrame {
        origin: [length(0.), length(0.), length(origin)],
        u: [phi.cos(), phi.sin(), 0.],
        v: [phi.sin(), -phi.cos(), 0.],
    };
    let blank = PrimitiveRequest::new(
        "Rc fitting blank",
        PrimitiveParameters::Cylinder {
            radius: length(intent(PipeThreadForm::Rc).base_major_mm() / 2. + 2.),
            height: length(intent(PipeThreadForm::Rc).default_length_mm()),
            frame: internal.frame.clone(),
        },
    );
    let p = model::prepare_primitive(&s, s.revision(), actor(), &blank).unwrap();
    s.commit(p.prepared).unwrap();
    let female = ThreadRequest::receiving(
        "Rc internal",
        ThreadMode::CutInternal,
        BodyInput {
            body: blank.body,
            producer: blank.feature,
        },
        internal,
    )
    .unwrap();
    let p = model::prepare_thread(&s, s.revision(), actor(), &female).unwrap();
    s.commit(p.prepared).unwrap();
    (s, male, female)
}
#[test]
fn sourced_r_rc_pairs_have_native_clearance_step_print_and_independent_reopen() {
    let dir = tempfile::tempdir().unwrap();
    for size in [PipeThreadSize::OneEighth, PipeThreadSize::OneQuarter] {
        let (s, male, female) = pair(size);
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
        for (i, solid) in solids.iter().enumerate() {
            assert_eq!(solid.solid_count().unwrap(), 1);
            let path = dir.path().join(format!("{size:?}-{i}.step"));
            solid.write_step(&path).unwrap();
            let back = Solid::read_step(&path, Default::default()).unwrap();
            let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            assert!(
                (back.volume_mm3().unwrap() - volumes[i]).abs()
                    < multicad_kernel::GeometryPolicy::default().linear_tolerance_mm * area
                        + 1e-7 * volumes[i]
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
        let receipt = exporter
            .write_print(
                s.document(),
                s.revision(),
                &request,
                &preview.key,
                &dir.path().join(format!("{size:?}.3mf")),
                &actor(),
                None,
            )
            .unwrap();
        assert_eq!(receipt.files.len(), 3);
        for file in receipt.files {
            let bytes = std::fs::read(&file.path).unwrap();
            let raw = match file.kind {
                PrintFileKind::ThreeMf => mesh::read_three_mf(&bytes).unwrap(),
                PrintFileKind::Stl => mesh::read_stl(&bytes).unwrap(),
            };
            assert_eq!(raw.triangles, file.triangles);
            assert!(raw.triangles > 100);
            assert!((raw.volume_mm3() - file.mesh_volume_mm3).abs() < 1e-6);
            if file.kind == PrintFileKind::Stl {
                let plan = preview
                    .files
                    .iter()
                    .find(|p| {
                        std::path::Path::new(&file.path).file_name().unwrap()
                            == p.filename(&format!("{size:?}")).as_str()
                    })
                    .unwrap();
                let is_male = plan.part == Some(male.body);
                let pitch = male.parameters.pipe.unwrap().preset().pitch_mm;
                let resolved = male
                    .parameters
                    .resolve(s.parameters(), ThreadMode::NewExternal)
                    .unwrap();
                let major = resolved.major_radius_mm;
                let depth = resolved.profile.depth_mm;
                // Independently section actual exported triangles. On the
                // mid-depth radius the crest and valley phases must alternate;
                // a cosmetic cone or a cylindrical bore fails this check.
                for (z, male_material) in [(2. * pitch, true), (2.5 * pitch, false)] {
                    let radius = major + z / 32. - depth / 2.;
                    assert_eq!(
                        mesh_material(&raw, [radius, 0., z]),
                        if is_male {
                            male_material
                        } else {
                            !male_material
                        }
                    );
                }
            }
        }
    }
}
#[test]
fn cosmetic_pipe_cannot_print() {
    let (mut s, male, _) = pair(PipeThreadSize::OneEighth);
    let mut edit = male.clone();
    edit.parameters.representation = ThreadRepresentation::CosmeticEnvelope;
    let p = model::prepare_thread_edit(&s, s.revision(), actor(), male.feature, &edit.parameters)
        .unwrap();
    s.commit(p.prepared).unwrap();
    let request = PrintRequest {
        source: PrintSource::Parts {
            parts: vec![male.body],
        },
        aids: AidPolicy::IncludeAll,
        quality: Default::default(),
        stl_per_part: false,
    };
    let error = Exporter::default()
        .print_preview(s.document(), s.revision(), &request, &actor(), None)
        .unwrap_err();
    assert!(error.to_string().contains("cosmetic"));
}
#[test]
fn tapered_receiving_wall_is_checked_at_the_entrance_and_along_the_full_depth() {
    let (s, _, female) = pair(PipeThreadSize::OneEighth);
    let before = s.document().clone();
    let mut parameters = female.parameters;
    parameters.minimum_wall = length(3.);
    let p =
        model::prepare_thread_edit(&s, s.revision(), actor(), female.feature, &parameters).unwrap();
    let error = compiler()
        .compile(p.prepared.document(), s.revision() + 1)
        .err()
        .expect("insufficient wall must fail native compilation");
    assert!(error.to_string().contains("minimum wall"), "{error}");
    assert_eq!(s.document(), &before);
}
