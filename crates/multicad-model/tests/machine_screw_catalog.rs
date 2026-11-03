//! Measure the actual shipped source and pinned templates, not regenerated fixtures.
use multicad_core::{
    document::EntityData,
    model::{self, ThreadFamily, ThreadFit, ThreadHand, ThreadRepresentation},
    project_file::{self, ContentHash},
    symbols,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::Compiler;
use serde_json::Value;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug)]
struct Row {
    hex: bool,
    nominal: f64,
    pitch: f64,
    length: f64,
    threaded: f64,
    head: f64,
    height: f64,
    socket: f64,
    depth: f64,
}
// Reviewed source expectations, independent of the generated project and JSON.
const ROWS: [Row; 19] = [
    Row {
        hex: false,
        nominal: 2.,
        pitch: 0.4,
        length: 10.,
        threaded: 10.,
        head: 3.8,
        height: 2.,
        socket: 1.55,
        depth: 1.,
    },
    Row {
        hex: false,
        nominal: 2.5,
        pitch: 0.45,
        length: 12.,
        threaded: 12.,
        head: 4.5,
        height: 2.5,
        socket: 2.05,
        depth: 1.1,
    },
    Row {
        hex: false,
        nominal: 3.,
        pitch: 0.5,
        length: 10.,
        threaded: 10.,
        head: 5.68,
        height: 3.,
        socket: 2.55,
        depth: 1.3,
    },
    Row {
        hex: false,
        nominal: 3.,
        pitch: 0.5,
        length: 30.,
        threaded: 18.,
        head: 5.68,
        height: 3.,
        socket: 2.55,
        depth: 1.3,
    },
    Row {
        hex: false,
        nominal: 4.,
        pitch: 0.7,
        length: 12.,
        threaded: 12.,
        head: 7.22,
        height: 4.,
        socket: 3.05,
        depth: 2.,
    },
    Row {
        hex: false,
        nominal: 4.,
        pitch: 0.7,
        length: 30.,
        threaded: 20.,
        head: 7.22,
        height: 4.,
        socket: 3.05,
        depth: 2.,
    },
    Row {
        hex: false,
        nominal: 5.,
        pitch: 0.8,
        length: 16.,
        threaded: 16.,
        head: 8.72,
        height: 5.,
        socket: 4.0575,
        depth: 2.5,
    },
    Row {
        hex: false,
        nominal: 5.,
        pitch: 0.8,
        length: 30.,
        threaded: 22.,
        head: 8.72,
        height: 5.,
        socket: 4.0575,
        depth: 2.5,
    },
    Row {
        hex: false,
        nominal: 6.,
        pitch: 1.,
        length: 20.,
        threaded: 20.,
        head: 10.22,
        height: 6.,
        socket: 5.08,
        depth: 3.,
    },
    Row {
        hex: false,
        nominal: 6.,
        pitch: 1.,
        length: 40.,
        threaded: 24.,
        head: 10.22,
        height: 6.,
        socket: 5.08,
        depth: 3.,
    },
    Row {
        hex: false,
        nominal: 8.,
        pitch: 1.25,
        length: 25.,
        threaded: 25.,
        head: 13.27,
        height: 8.,
        socket: 6.08,
        depth: 4.,
    },
    Row {
        hex: false,
        nominal: 8.,
        pitch: 1.25,
        length: 40.,
        threaded: 28.,
        head: 13.27,
        height: 8.,
        socket: 6.08,
        depth: 4.,
    },
    Row {
        hex: false,
        nominal: 10.,
        pitch: 1.5,
        length: 30.,
        threaded: 30.,
        head: 16.27,
        height: 10.,
        socket: 8.10,
        depth: 5.,
    },
    Row {
        hex: false,
        nominal: 10.,
        pitch: 1.5,
        length: 50.,
        threaded: 32.,
        head: 16.27,
        height: 10.,
        socket: 8.10,
        depth: 5.,
    },
    Row {
        hex: true,
        nominal: 4.,
        pitch: 0.7,
        length: 16.,
        threaded: 16.,
        head: 7.,
        height: 2.8,
        socket: 0.,
        depth: 0.,
    },
    Row {
        hex: true,
        nominal: 5.,
        pitch: 0.8,
        length: 16.,
        threaded: 16.,
        head: 8.,
        height: 3.5,
        socket: 0.,
        depth: 0.,
    },
    Row {
        hex: true,
        nominal: 6.,
        pitch: 1.,
        length: 20.,
        threaded: 20.,
        head: 10.,
        height: 4.,
        socket: 0.,
        depth: 0.,
    },
    Row {
        hex: true,
        nominal: 8.,
        pitch: 1.25,
        length: 25.,
        threaded: 25.,
        head: 13.,
        height: 5.3,
        socket: 0.,
        depth: 0.,
    },
    Row {
        hex: true,
        nominal: 10.,
        pitch: 1.5,
        length: 30.,
        threaded: 30.,
        head: 17.,
        height: 6.4,
        socket: 0.,
        depth: 0.,
    },
];
impl Row {
    fn id(self) -> String {
        format!(
            "screw.{}.m{}.l{}",
            if self.hex { "hex" } else { "socket" },
            self.nominal.to_string().replace('.', "-"),
            self.length
        )
    }
}
fn close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < 1e-5,
        "{label}: actual={actual:.12}, expected={expected:.12}"
    );
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
fn material(solid: &Solid, point: [f64; 3], label: &str) {
    assert!(
        probe(solid, point) > 0.999,
        "Missing material at {point:?}: {label}"
    );
}
fn void(solid: &Solid, point: [f64; 3], label: &str) {
    assert!(
        probe(solid, point) < 1e-5,
        "Unexpected material at {point:?}: {label}"
    );
}
fn measure(solid: &Solid, row: Row) {
    assert_eq!(solid.solid_count().unwrap(), 1, "{:?}", row);
    let bounds = solid.bounds().unwrap();
    close(bounds.min[2], -row.height, "head top");
    close(bounds.max[2], row.length, "tip / underhead length");
    close(bounds.max[0], row.head / 2., "head positive X");
    close(bounds.min[0], -row.head / 2., "head negative X");
    let yradius = if row.hex {
        row.head / 3_f64.sqrt()
    } else {
        row.head / 2.
    };
    close(bounds.max[1], yradius, "head positive Y");
    close(bounds.min[1], -yradius, "head negative Y");
    let faces = solid.faces().unwrap();
    if row.hex {
        let walls: Vec<_> = faces
            .iter()
            .filter(|face| face.center_mm[2] < -1e-5)
            .filter_map(|face| {
                let (point, normal) = face.plane?;
                (normal[2].abs() < 1e-8).then_some((face, point, normal))
            })
            .collect();
        assert_eq!(walls.len(), 6, "Six true external flats");
        for (face, point, normal) in walls {
            let distance = (point[0] * normal[0] + point[1] * normal[1]).abs();
            close(2. * distance, row.head, "measured external across flats");
            close(
                face.area_mm2 / (row.head / 3_f64.sqrt()),
                row.height,
                "height from flat area",
            );
        }
        material(
            solid,
            [row.head / 2. - 0.02, 0., -row.height / 2.],
            "inside external flat",
        );
        void(
            solid,
            [row.head / 2. + 0.02, 0., -row.height / 2.],
            "outside external flat",
        );
        material(
            solid,
            [0., yradius - 0.02, -row.height / 2.],
            "external corner beyond inscribed circle",
        );
        material(solid, [0., 0., -row.height / 2.], "solid external-hex head");
    } else {
        let cylindrical_area: f64 = faces
            .iter()
            .filter(|face| face.plane.is_none() && face.center_mm[2] < -1e-5)
            .map(|face| face.area_mm2)
            .sum();
        close(
            cylindrical_area / (PI * row.height),
            row.head,
            "head diameter from cylindrical area",
        );
        let walls: Vec<_> = faces
            .iter()
            .filter(|face| face.center_mm[2] < -1e-5)
            .filter_map(|face| {
                let (point, normal) = face.plane?;
                (normal[2].abs() < 1e-8).then_some((face, point, normal))
            })
            .collect();
        assert_eq!(walls.len(), 6, "Six actual socket flats");
        for (face, point, normal) in walls {
            close(
                2. * (point[0] * normal[0] + point[1] * normal[1]).abs(),
                row.socket,
                "actual socket across flats",
            );
            close(
                face.area_mm2 / (row.socket / 3_f64.sqrt()),
                row.depth,
                "socket depth from wall area",
            );
        }
        let floor = -row.height + row.depth;
        let floors: Vec<_> = faces
            .iter()
            .filter(|face| {
                face.plane.is_some_and(|(point, normal)| {
                    normal[2].abs() > 1. - 1e-8 && (point[2] - floor).abs() < 1e-5
                })
            })
            .collect();
        assert_eq!(floors.len(), 1, "One flat socket floor");
        close(
            floors[0].area_mm2,
            3_f64.sqrt() / 2. * row.socket.powi(2),
            "socket floor area",
        );
        let between_flat_and_corner = (row.socket / 2. + row.socket / 3_f64.sqrt()) / 2.;
        let z = -row.height + row.depth / 2.;
        void(
            solid,
            [0., between_flat_and_corner, z],
            "hex socket corner beyond nominal cylindrical bore",
        );
        material(
            solid,
            [between_flat_and_corner, 0., z],
            "material beyond socket flat",
        );
        void(solid, [0., 0., floor - 0.02], "socket above floor");
        material(solid, [0., 0., floor + 0.02], "retained floor below cavity");
        material(solid, [row.head / 2. - 0.02, 0., z], "outer head envelope");
        void(
            solid,
            [row.head / 2. + 0.02, 0., z],
            "outside outer head envelope",
        );
    }
    material(solid, [0., 0., 0.02], "shaft remains joined to head");
    let start = row.length - row.threaded;
    let pitch_radius = row.nominal / 2. - 3. * 3_f64.sqrt() * row.pitch / 16.;
    if start > 0. {
        material(
            solid,
            [row.nominal / 2. - 0.02, 0., start / 2.],
            "plain shank before thread start",
        );
        void(
            solid,
            [row.nominal / 2. + 0.02, 0., start / 2.],
            "outside shank nominal diameter",
        );
    }
    for turn in [1., (row.threaded / row.pitch).floor() - 2.] {
        material(
            solid,
            [pitch_radius, 0., start + (turn + 0.20) * row.pitch],
            "real thread ridge at pitch radius",
        );
        void(
            solid,
            [pitch_radius, 0., start + (turn + 0.30) * row.pitch],
            "real thread groove; shank must not fill it",
        );
    }
}

fn assert_volume_agreement(native: &Solid, step: &Solid, label: &str) {
    let before = native.volume_mm3().unwrap();
    let after = step.volume_mm3().unwrap();
    let native_area = native
        .faces()
        .unwrap()
        .iter()
        .map(|f| f.area_mm2)
        .sum::<f64>();
    let step_area = step
        .faces()
        .unwrap()
        .iter()
        .map(|f| f.area_mm2)
        .sum::<f64>();
    // Linear B-rep uncertainty times surface area, plus the existing 1e-7
    // relative numerical volume-integration accuracy for each representation.
    let budget = GeometryPolicy::default().linear_tolerance_mm * (native_area + step_area)
        + 1e-7 * (before.abs() + after.abs());
    assert!(
        (before - after).abs() <= budget,
        "{label}: native={before:.15}, STEP={after:.15}, geometric/integration budget={budget:.15}"
    );
}

#[test]
fn all_nineteen_shipped_screws_and_pinned_templates_have_source_dimensions_hex_drives_real_threads_and_step_readbacks()
 {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../library/machine-screws-curation.json"
    ))
    .unwrap();
    assert_eq!(catalog["format"], "toskcad.machine-screw-curation");
    assert_eq!(catalog["version"], 1);
    let entries = catalog["entries"].as_array().unwrap();
    assert_eq!(entries.len(), ROWS.len());
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(library.join("library.json")).unwrap()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    for row in ROWS {
        let id = row.id();
        let found: Vec<_> = entries.iter().filter(|entry| entry["id"] == id).collect();
        assert_eq!(found.len(), 1, "Unique source entry {id}");
        let entry = found[0];
        for (key, value) in [
            ("nominal_mm", row.nominal),
            ("pitch_mm", row.pitch),
            ("underhead_length_mm", row.length),
            ("thread_length_mm", row.threaded),
            ("shank_diameter_mm", row.nominal),
            ("head_height_mm", row.height),
            (
                if row.hex {
                    "head_across_flats_mm"
                } else {
                    "head_diameter_mm"
                },
                row.head,
            ),
        ] {
            assert_eq!(entry[key].as_f64(), Some(value), "{id}: {key}");
        }
        assert_eq!(
            entry["family"],
            if row.hex {
                "external_hex"
            } else {
                "socket_cap"
            }
        );
        if !row.hex {
            assert_eq!(entry["drive_across_flats_mm"].as_f64(), Some(row.socket));
            assert_eq!(entry["drive_depth_mm"].as_f64(), Some(row.depth));
        }
        let filename = entry["file"].as_str().unwrap();
        let records: Vec<_> = manifest["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["file"] == filename)
            .collect();
        assert_eq!(records.len(), 1, "Unique manifest entry for {id}");
        let bytes = std::fs::read(library.join(filename)).unwrap();
        assert_eq!(records[0]["sha256"], ContentHash::of(&bytes).to_string());
        let project = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(records[0]["document"], project.document.id().to_string());
        assert!(
            project
                .capabilities()
                .contains(model::MACHINE_SCREW_CAPABILITY)
        );
        let definitions = symbols::definitions(&project.document).unwrap();
        assert_eq!(definitions.len(), 1);
        let definition = &definitions[0].1;
        for (label, document) in [
            ("author", &project.document),
            ("template", &definition.template),
        ] {
            let features: Vec<_>=document.entities().filter(|e|matches!(&e.data,EntityData::Feature{definition,..} if definition.type_id==model::MECHANICAL_PART_TYPE)).collect();
            assert_eq!(features.len(), 1, "{id} {label}");
            let feature = features[0].id;
            let source = model::read_mechanical_part(document, feature).unwrap();
            assert_eq!(source.output.body, definition.root);
            let evaluated = multicad_core::parameters::evaluate(document).unwrap();
            assert!(source.matches_reference_dimensions(&evaluated).unwrap());
            let model::MechanicalPartParameters::MachineScrew { thread, .. } = &source.parameters
            else {
                panic!("Shipped source must be a canonical screw")
            };
            assert_eq!(thread.family, ThreadFamily::IsoMetric);
            assert_eq!(thread.fit, ThreadFit::NominalBasic);
            assert_eq!(thread.hand, ThreadHand::Right);
            assert_eq!(thread.starts, 1);
            assert_eq!(thread.representation, ThreadRepresentation::Modeled);
            let facts = source
                .provenance
                .as_ref()
                .unwrap()
                .catalog_facts
                .as_ref()
                .unwrap();
            assert_eq!(facts.nominal_fastener_diameter_mm, Some(row.nominal));
            assert!(facts.material.is_none() && facts.grade.is_none());
            let built = Compiler {
                omit_display: true,
                ..Default::default()
            }
            .compile(document, 0)
            .unwrap();
            let solid = built.solid(feature).unwrap();
            measure(solid, row);
            let path = directory.path().join(format!("{id}-{label}.step"));
            solid.write_step(&path).unwrap();
            let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            measure(&readback, row);
            assert_volume_agreement(solid, &readback, &format!("{id} {label}"));
        }
    }
}

#[test]
fn every_requested_nominal_has_exact_per_family_supported_or_custom_coverage() {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../library/machine-screws-curation.json"
    ))
    .unwrap();
    let coverage = catalog["coverage"].as_array().unwrap();
    assert_eq!(coverage.len(), 12);
    for nominal in [2., 2.5, 3., 3.5, 4., 4.5, 5., 6., 7., 8., 9., 10.] {
        let rows: Vec<_> = coverage
            .iter()
            .filter(|r| r["nominal_mm"].as_f64() == Some(nominal))
            .collect();
        assert_eq!(rows.len(), 1);
        for hex in [false, true] {
            let family = if hex { "external_hex" } else { "socket_cap" };
            let mut expected: Vec<_> = ROWS
                .iter()
                .filter(|r| r.nominal == nominal && r.hex == hex)
                .map(|r| r.id())
                .collect();
            expected.sort();
            let actual = &rows[0][family];
            assert_eq!(
                actual["status"],
                if expected.is_empty() {
                    "unavailable"
                } else {
                    "supported"
                }
            );
            assert!(!actual["reason"].as_str().unwrap().trim().is_empty());
            let mut ids: Vec<_> = actual["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect();
            ids.sort();
            assert_eq!(ids, expected, "M{nominal} {family}");
        }
    }
}
