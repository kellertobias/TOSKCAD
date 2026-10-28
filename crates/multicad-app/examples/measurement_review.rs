//! Real native equipment envelopes and associative measurement review projects.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    inspection::{self, GeometryReference, MeasurementQuery, MeasurementStamp, PinRequest},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, TopologyKind},
};
use multicad_model::{Compiler, Model, inspection::measure_model};
use std::{io::Write, path::Path};
type Error = Box<dyn std::error::Error>;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn primitive(
    s: &mut Session,
    name: &str,
    size: [f64; 3],
    origin: [Dimension; 3],
) -> Result<PrimitiveRequest, Error> {
    let r = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: size.map(length),
            frame: PrimitiveFrame {
                origin,
                ..Default::default()
            },
        },
    );
    s.commit(model::prepare_primitive(s, s.revision(), actor(), &r)?.prepared)?;
    Ok(r)
}
fn face(m: &Model, body: EntityId, normal: [f64; 3]) -> Result<GeometryReference, Error> {
    let producer = m.snapshot().body_sources[&body];
    let mut t = m.snapshot().shapes[&producer].topology.clone();
    t.owner = body;
    let key=t.elements.iter().find(|e|e.key.kind==TopologyKind::Face && matches!(e.geometry,Some(AnalyticGeometry::Plane{frame}) if frame.normal().iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-9))).ok_or("Expected exact named planar face")?.key;
    Ok(GeometryReference {
        source: body,
        intent: GeometryIntent::capture(&t, key)?,
    })
}
fn save(s: &Session, path: &Path) -> Result<serde_json::Value, Error> {
    let m = Compiler::default().compile(s.document(), s.revision())?;
    let file = ProjectFile::new(s.document().clone())?;
    let mut bytes = Vec::new();
    project_file::write(&file, &mut bytes, Default::default())?;
    let decoded = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    if decoded.document != *s.document() {
        return Err("Native project changed source while reopening".into());
    }
    let reopened = Compiler::default().compile(&decoded.document, 0)?;
    let mut values = Vec::new();
    for id in inspection::pins(&decoded.document) {
        let pin = inspection::read(&decoded.document, id)?;
        let evaluation = measure_model(&reopened, &pin.query);
        values.push(serde_json::json!({"pin":{"id":pin.id,"name":pin.name,"query":pin.query,"visible":pin.visible,"label_offset_mm":pin.label_offset_mm},"evaluation":evaluation}));
    }
    let bodies=m.snapshot().bodies.iter().map(|(id,producer)|{let shape=&m.snapshot().shapes[producer];serde_json::json!({"id":id,"name":s.document().get(*id).map(|e|&e.name),"producer":producer,"bounds_min_mm":shape.bounds.min,"bounds_max_mm":shape.bounds.max,"volume_mm3":shape.volume_mm3,"solid_count":shape.solid_count})}).collect::<Vec<_>>();
    let mut f = tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing parent")?)?;
    f.write_all(&bytes)?;
    f.as_file().sync_all()?;
    f.persist_noclobber(path)?;
    Ok(
        serde_json::json!({"file":path.file_name().ok_or("No filename")?.to_string_lossy(),"source_stamp":MeasurementStamp::capture(s.document(),s.revision())?,"bodies":bodies,"reopened_pins":values}),
    )
}
fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let mut s = Session::new(Document::new("Bench and equipment clearance")?)?;
    let parameter = Entity::new(
        "clearance",
        EntityData::Parameter {
            expression: "400".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Clearance parameter".into(),
        edits: vec![Mutation::Insert(parameter)],
    })?;
    let bench = primitive(
        &mut s,
        "Bench envelope",
        [1200., 600., 900.],
        [length(0.), length(0.), length(0.)],
    )?;
    let x = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=1200 + $clearance", "mm", None)?,
    };
    let equipment = primitive(
        &mut s,
        "Saw envelope",
        [600., 600., 1500.],
        [x, length(0.), length(0.)],
    )?;
    let mut reports = vec![save(&s, &directory.join("01-ready.multicad"))?];
    let m = Compiler::default().compile(s.document(), s.revision())?;
    let query = MeasurementQuery {
        a: face(&m, bench.body, [1., 0., 0.])?,
        b: face(&m, equipment.body, [-1., 0., 0.])?,
    };
    let pin = PinRequest::new("Bench to saw clearance", query);
    if (measure_model(&m, &pin.query).result?.minimum_mm - 400.).abs() > 1e-6 {
        return Err("Initial clearance mismatch".into());
    }
    s.commit(inspection::prepare_pin(&s, s.revision(), actor(), &pin)?)?;
    reports.push(save(&s, &directory.join("02-pinned.multicad"))?);
    let mut changed = s
        .document()
        .get(parameter_id)
        .ok_or("Missing parameter")?
        .clone();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        return Err("Not a parameter".into());
    };
    *expression = "650".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Move equipment through parameter".into(),
        edits: vec![Mutation::Replace(changed)],
    })?;
    let m = Compiler::default().compile(s.document(), s.revision())?;
    if (measure_model(&m, &pin.query).result?.minimum_mm - 650.).abs() > 1e-6 {
        return Err("Updated clearance mismatch".into());
    }
    reports.push(save(&s, &directory.join("03-clearance650.multicad"))?);
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Remove measurement source".into(),
        edits: vec![
            Mutation::Remove(equipment.body),
            Mutation::Remove(equipment.feature),
        ],
    })?;
    let m = Compiler::default().compile(s.document(), s.revision())?;
    if measure_model(&m, &pin.query).result.is_ok() {
        return Err("Removed source silently retained a value".into());
    }
    reports.push(save(&s, &directory.join("04-source-missing.multicad"))?);
    let report = serde_json::json!({"scope":"Real compiled B-rep equipment envelopes and metadata-only measurement pins; this fixture does not implement Room tools or establish physical equipment safety clearance","bench":bench,"equipment":equipment,"parameter":parameter_id,"pin":pin.id,"fixtures":reports,"native_gui_steps":["Open 01-ready, click global Measure beside Library, choose actual bench and saw faces. Face selection can use camera axis changes between A and B.","Open 02-pinned, click Measure, select Bench to saw clearance in retained pins. Native minimum is400mm; parallel supporting-plane separation is independently labeled.","Change clearance400to650 in Parameters, Apply, return to Measure: retained pin evaluates650mm.","Open 04-source-missing: remaining Bench geometry remains usable, pin is explicitly unresolved with no stale number. ReplaceB with a current face or remove pin.","Select two different faces on Bench to measure its actual dimensions; pin/Update/Remove are metadata-only Undo steps."]});
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("review.json"))?;
    f.write_all(&serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}
fn main() -> Result<(), Error> {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/measurement-review")
        });
    generate(&directory)?;
    println!("Native review projects: {}", directory.display());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_fixture_roundtrips_current_values_missing_sources_and_refuses_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("review");
        let report = generate(&dir).unwrap();
        assert_eq!(report["fixtures"].as_array().unwrap().len(), 4);
        assert!(generate(&dir).is_err());
        assert_eq!(
            report["fixtures"][1]["reopened_pins"][0]["evaluation"]["result"]["Ok"]["minimum_mm"],
            400.
        );
        assert_eq!(
            report["fixtures"][2]["reopened_pins"][0]["evaluation"]["result"]["Ok"]["minimum_mm"],
            650.
        );
        assert!(
            report["fixtures"][3]["reopened_pins"][0]["evaluation"]["result"]
                .get("Err")
                .is_some()
        );
    }
}
