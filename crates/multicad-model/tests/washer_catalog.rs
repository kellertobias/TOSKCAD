use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData},
    model::{self, MechanicalPartParameters, MechanicalPartRequest, length},
    project_file::{self, ContentHash},
    symbols,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::Compiler;
use serde_json::Value;
use std::f64::consts::PI;

// Independent reviewed expectations, not computed from the catalog under test.
const ROWS: [(f64, f64, f64, f64); 10] = [
    (2., 2.2, 5., 0.3),
    (2.5, 2.7, 6., 0.5),
    (3., 3.2, 7., 0.5),
    (3.5, 3.7, 8., 0.5),
    (4., 4.3, 9., 0.8),
    (5., 5.3, 10., 1.),
    (6., 6.4, 12., 1.6),
    (7., 7.4, 14., 1.6),
    (8., 8.4, 16., 1.6),
    (10., 10.5, 20., 2.),
];

fn assert_ring(solid: &Solid, bore: f64, outside: f64, thickness: f64) {
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    for axis in [0, 1] {
        assert!((bounds.min[axis] + outside / 2.).abs() < 1e-6);
        assert!((bounds.max[axis] - outside / 2.).abs() < 1e-6);
    }
    assert!(bounds.min[2].abs() < 1e-6);
    assert!((bounds.max[2] - thickness).abs() < 1e-6);
    // Measure both cylindrical surfaces independently of the volume and bounds.
    let mut diameters: Vec<_> = solid
        .faces()
        .unwrap()
        .into_iter()
        .filter(|face| face.plane.is_none())
        .map(|face| face.area_mm2 / (PI * thickness))
        .collect();
    diameters.sort_by(f64::total_cmp);
    assert_eq!(diameters.len(), 2);
    assert!((diameters[0] - bore).abs() < 1e-6);
    assert!((diameters[1] - outside).abs() < 1e-6);
    let expected = PI * (outside.powi(2) - bore.powi(2)) * thickness / 4.;
    assert!((solid.volume_mm3().unwrap() - expected).abs() < 1e-6);
}

#[test]
fn all_reviewed_washer_rows_have_independently_measured_bores_outsides_thicknesses_and_step_geometry()
 {
    let catalog: Value =
        serde_json::from_str(include_str!("../../../library/washers-curation.json")).unwrap();
    assert_eq!(catalog["format"], "toskcad.washer-curation");
    assert_eq!(catalog["version"], 1);
    let entries = catalog["entries"].as_array().unwrap();
    assert_eq!(entries.len(), ROWS.len());
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(library.join("library.json")).unwrap()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    for (nominal, bore, outside, thickness) in ROWS {
        let entry = entries
            .iter()
            .find(|row| row["nominal_mm"].as_f64() == Some(nominal))
            .unwrap();
        assert_eq!(
            entry["bore_mm"], bore,
            "M{nominal} bore is clearance, not fastener nominal"
        );
        assert_eq!(entry["outside_mm"], outside);
        assert_eq!(entry["thickness_mm"], thickness);
        // Measure the actual packaged authoring source AND the immutable
        // template that consumers import; do not regenerate a matching fixture.
        let filename = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(library.join(filename)).unwrap();
        let records: Vec<_> = manifest["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|record| record["file"] == filename)
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["sha256"], ContentHash::of(&bytes).to_string());
        let project = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let definitions = symbols::definitions(&project.document).unwrap();
        assert_eq!(definitions.len(), 1);
        let definition = &definitions[0].1;
        for (label, document) in [
            ("author", &project.document),
            ("template", &definition.template),
        ] {
            let features: Vec<_> = document.entities().filter(|entity| matches!(&entity.data, EntityData::Feature { definition, .. } if definition.type_id == model::MECHANICAL_PART_TYPE)).collect();
            assert_eq!(features.len(), 1);
            let feature = features[0].id;
            let source = model::read_mechanical_part(document, feature).unwrap();
            assert_eq!(source.output.body, definition.root);
            let facts = source
                .provenance
                .as_ref()
                .unwrap()
                .catalog_facts
                .as_ref()
                .unwrap();
            assert_eq!(facts.nominal_fastener_diameter_mm, Some(nominal));
            assert!(facts.material.is_none() && facts.grade.is_none());
            let evaluated = multicad_core::parameters::evaluate(document).unwrap();
            assert!(source.matches_reference_dimensions(&evaluated).unwrap());
            let built = Compiler {
                omit_display: true,
                ..Default::default()
            }
            .compile(document, 0)
            .unwrap();
            let solid = built.solid(feature).unwrap();
            assert_ring(solid, bore, outside, thickness);
            let path = directory
                .path()
                .join(format!("washer-{nominal}-{label}.step"));
            solid.write_step(&path).unwrap();
            let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            assert_ring(&readback, bore, outside, thickness);
        }
    }
    let coverage = catalog["coverage"].as_array().unwrap();
    assert_eq!(coverage.len(), 12);
    for (nominal, ..) in ROWS {
        let rows: Vec<_> = coverage
            .iter()
            .filter(|row| row["nominal_mm"].as_f64() == Some(nominal))
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["status"], "supported");
        let entry = entries
            .iter()
            .find(|entry| entry["nominal_mm"].as_f64() == Some(nominal))
            .unwrap();
        assert_eq!(rows[0]["entry_id"], entry["id"]);
    }
    for nominal in [4.5, 9.] {
        let row = coverage
            .iter()
            .find(|row| row["nominal_mm"].as_f64() == Some(nominal))
            .unwrap();
        assert_eq!(row["status"], "unavailable");
        assert!(row["entry_id"].is_null());
        assert!(!row["reason"].as_str().unwrap().is_empty());
    }
}

#[test]
fn impossible_washer_dimensions_never_prepare_or_mutate_source() {
    let session = Session::new(Document::new("Invalid washers").unwrap()).unwrap();
    let before = session.document().clone();
    for (bore, outside, thickness) in [
        (0., 7., 0.5),
        (3.2, 3.2, 0.5),
        (8., 7., 0.5),
        (3.2, 7., 0.),
        (3.2, 7., -1.),
    ] {
        let request = MechanicalPartRequest::new(
            "Invalid washer",
            MechanicalPartParameters::Washer {
                bore_diameter: length(bore),
                outside_diameter: length(outside),
                thickness: length(thickness),
                frame: Default::default(),
            },
        );
        assert!(
            model::prepare_mechanical_part(&session, 0, Actor::local(Channel::Mcp), &request)
                .is_err()
        );
        assert_eq!(session.document(), &before);
    }
}
