//! Exercise the desktop query adapters against the same canonical project.
#[path = "../src/assembly_inspection.rs"]
mod assembly_inspection;
#[path = "../src/inspection.rs"]
mod inspection;

use multicad_core::{
    assembly_inspection::{
        AssemblyInspectionQuery, AssemblyInspectionReport, InspectionStamp, OccurrencePair,
        PairQuery, PairRelation,
    },
    commands::{Actor, Channel, Session},
    document::Document,
    inspection::{GeometryReference, MeasurementQuery, MeasurementStamp},
    model::{self, OccurrencePath, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    references::{AnalyticGeometry, GeometryIntent},
};
use multicad_model::{Compiler, GeometryBackend};
use multicad_ui::{assembly::AssemblyInspectionBackend, inspection::MeasurementBackend};
use std::time::{Duration, Instant};

fn reply<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(reply) = poll() {
            return reply;
        }
        assert!(Instant::now() < deadline, "Inspection worker did not reply");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn add(session: &mut Session, primitive: &PrimitiveRequest) {
    let prepared = model::prepare_primitive(
        session,
        session.revision(),
        Actor::local(Channel::Ui),
        primitive,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
}

fn box_at(x: f64) -> PrimitiveRequest {
    PrimitiveRequest::new(
        "Square stock",
        PrimitiveParameters::Box {
            size: [4.; 3].map(length),
            frame: PrimitiveFrame {
                origin: [length(x), length(0.), length(0.)],
                ..Default::default()
            },
        },
    )
}

fn pair_query(session: &Session, a: &PrimitiveRequest, b: &PrimitiveRequest) -> PairQuery {
    let path = |root| OccurrencePath {
        root,
        members: vec![],
    };
    PairQuery {
        expected: InspectionStamp::capture(session.document(), session.revision()).unwrap(),
        pairs: vec![OccurrencePair {
            a: path(a.body),
            b: path(b.body),
        }],
        contact_tolerance_mm: 1e-6,
    }
}

#[test]
fn desktop_inspection_workers_keep_the_selected_backend_for_the_same_source() {
    let mut session = Session::new(Document::new("Inspection backend").unwrap()).unwrap();
    let sphere = PrimitiveRequest::new(
        "Round stock",
        PrimitiveParameters::Sphere {
            radius: length(3.),
            frame: Default::default(),
        },
    );
    let box_ = box_at(10.);
    for primitive in [&sphere, &box_] {
        add(&mut session, primitive);
    }
    let document = session.document();
    let revision = session.revision();
    let measurement = MeasurementStamp::capture(document, revision).unwrap();
    let assembly = AssemblyInspectionQuery::Current(pair_query(&session, &sphere, &box_));

    let mut native_measure =
        inspection::Backend::with_geometry_backend(GeometryBackend::OpenCascade);
    native_measure.request(document, measurement, None).unwrap();
    assert!(reply(|| native_measure.poll()).result.is_ok());
    let mut native_assembly =
        assembly_inspection::Backend::with_geometry_backend(GeometryBackend::OpenCascade);
    native_assembly.request(document, &assembly).unwrap();
    let AssemblyInspectionReport::Current(report) =
        reply(|| native_assembly.poll()).result.unwrap()
    else {
        panic!("Expected current assembly report");
    };
    let pair = report.pairs[0].result.as_ref().unwrap();
    assert_eq!(pair.relation, PairRelation::Separated);
    assert!((pair.minimum_clearance_mm - 7.).abs() < 1e-6);

    let mut rust_measure = inspection::Backend::with_geometry_backend(GeometryBackend::Rust);
    rust_measure.request(document, measurement, None).unwrap();
    let mut rust_assembly =
        assembly_inspection::Backend::with_geometry_backend(GeometryBackend::Rust);
    rust_assembly.request(document, &assembly).unwrap();
    assert!(reply(|| rust_measure.poll()).result.is_ok());
    let AssemblyInspectionReport::Current(report) = reply(|| rust_assembly.poll()).result.unwrap()
    else {
        panic!("Expected current Rust assembly report");
    };
    let failure = report.pairs[0].result.as_ref().unwrap_err();
    assert_eq!(
        failure.kind,
        multicad_core::assembly_inspection::InspectionFailureKind::Unsupported
    );
    assert_eq!(session.revision(), revision);
}

#[test]
fn rust_desktop_workers_measure_real_finite_vertices_and_assembly_clearance() {
    let mut session = Session::new(Document::new("Rust assembly clearance").unwrap()).unwrap();
    let [a, b] = [0., 10.].map(box_at);
    add(&mut session, &a);
    add(&mut session, &b);
    let document = session.document();
    let revision = session.revision();
    let backend = GeometryBackend::Rust;
    let model = Compiler::default()
        .compile_with_backend(document, revision, backend, None)
        .unwrap();
    let vertex = |source, x| {
        let topology = model
            .solid(source)
            .unwrap()
            .topology_snapshot(source, revision)
            .unwrap();
        let element = topology
            .elements
            .iter()
            .find(|element| {
                element.geometry
                    == Some(AnalyticGeometry::Point {
                        position_mm: [x, 0., 0.],
                    })
            })
            .unwrap();
        GeometryReference {
            source,
            intent: GeometryIntent::capture(&topology, element.key).unwrap(),
        }
    };
    let query = MeasurementQuery {
        a: vertex(a.body, 4.),
        b: vertex(b.body, 10.),
    };
    let mut measure = inspection::Backend::with_geometry_backend(backend);
    let stamp = MeasurementStamp::capture(document, revision).unwrap();
    measure.request(document, stamp, Some(&query)).unwrap();
    let measured = reply(|| measure.poll())
        .result
        .unwrap()
        .transient
        .unwrap()
        .result
        .unwrap();
    assert_eq!(measured.minimum_mm, 6.);
    assert_eq!(measured.axis_components_mm, Some([6., 0., 0.]));

    let mut assembly = assembly_inspection::Backend::with_geometry_backend(backend);
    let query = AssemblyInspectionQuery::Current(pair_query(&session, &a, &b));
    assembly.request(document, &query).unwrap();
    let AssemblyInspectionReport::Current(report) = reply(|| assembly.poll()).result.unwrap()
    else {
        panic!("Expected current assembly report");
    };
    let result = report.pairs[0].result.as_ref().unwrap();
    assert_eq!(result.relation, PairRelation::Separated);
    assert_eq!(result.minimum_clearance_mm, 6.);
    assert_eq!(result.overlap_volume_mm3, 0.);
}
