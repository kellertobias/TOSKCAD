//! Headless exact-geometry report for a saved native project. This verifies
//! source regeneration; it is not the production Export workspace.
#[path = "../src/files.rs"]
mod files;
use multicad_core::{
    document::{Document, EntityData},
    model,
};
use multicad_model::Compiler;
use multicad_ui::storage::DocumentStorage;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
type Error = Box<dyn std::error::Error>;
fn report(document: &Document) -> Result<serde_json::Value, Error> {
    let model = Compiler::default().compile(document, 0)?;
    let snapshot = model.snapshot();
    let bodies=snapshot.bodies.iter().map(|(body,producer)| {
        let shape=&snapshot.shapes[producer];
        serde_json::json!({"body":body,"producer":producer,"name":document.get(*body).map(|e|&e.name),"volume_mm3":shape.volume_mm3,"solid_count":shape.solid_count,"bounds":{"min":shape.bounds.min,"max":shape.bounds.max},"triangles":shape.mesh.indices.len()/3})
    }).collect::<Vec<_>>();
    let occurrences=snapshot.occurrences.values().map(|occurrence|->Result<serde_json::Value,Error>{
        let solid=model.solid(occurrence.instance).ok_or("Evaluated occurrence has no exact world solid")?;
        let bounds=solid.bounds()?;
        Ok(serde_json::json!({"instance":occurrence.instance,"component":occurrence.component,"producer":occurrence.producer,"name":document.get(occurrence.instance).map(|e|&e.name),"pose":occurrence.pose,"volume_mm3":solid.volume_mm3()?,"solid_count":solid.solid_count()?,"bounds":{"min":bounds.min,"max":bounds.max},"canonical_source":document.get(occurrence.instance),"topology":occurrence.topology}))
    }).collect::<Result<Vec<_>,Error>>()?;
    let parameters = document
        .entities()
        .filter(|entity| matches!(entity.data, EntityData::Parameter { .. }))
        .collect::<Vec<_>>();
    let leaf_occurrences=snapshot.leaf_occurrences.values().map(|leaf|->Result<serde_json::Value,Error>{
        let solid=model.leaf_solid(&leaf.path).ok_or("Physical leaf has no exact native solid")?;
        Ok(serde_json::json!({"path":leaf.path,"component":leaf.component,"producer":leaf.producer,"pose":leaf.pose,"bounds":{"min":leaf.bounds.min,"max":leaf.bounds.max},"volume_mm3":solid.volume_mm3()?,"solid_count":solid.solid_count()?}))
    }).collect::<Result<Vec<_>,Error>>()?;
    let placement_drivers=document.entities().filter(|entity|matches!(&entity.data,EntityData::Extension{definition,..} if matches!(definition.type_id.as_str(),model::INSTANCE_PLACEMENT_TYPE|model::INSTANCE_ALIGNMENT_TYPE))).collect::<Vec<_>>();
    let producer_sources = document
        .entities()
        .filter(|entity| snapshot.shapes.contains_key(&entity.id))
        .collect::<Vec<_>>();
    let context = multicad_solver::SketchEvaluationContext::new(document, 0)?;
    let sketches = document
        .entities()
        .filter(|entity| {
            matches!(&entity.data, EntityData::Sketch { definition, .. }
                if definition.type_id == multicad_core::constraint_sketch::source::TYPE)
        })
        .map(|entity| -> Result<serde_json::Value, Error> {
            let evaluated = context.evaluate(entity.id, Some(&snapshot.references), None)?;
            let solved = evaluated
                .solved()
                .ok_or("Saved sketch has no current solved geometry")?;
            let bindings = multicad_core::sketch_external::bindings(document, entity.id)?
                .into_iter()
                .map(|binding| {
                    serde_json::json!({
                        "reference": binding.reference,
                        "source": binding.source,
                        "intent": binding.intent,
                        "definition": binding.definition,
                    })
                })
                .collect::<Vec<_>>();
            let frame = evaluated.frame;
            Ok(serde_json::json!({
                "id": entity.id,
                "name": entity.name,
                "plane": evaluated.plane,
                "frame": {"origin":frame.origin,"u":frame.u,"v":frame.v,"n":frame.n},
                "canonical": evaluated.canonical(),
                "solved": solved,
                "external_bindings": bindings,
            }))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(
        serde_json::json!({"document":document.id(),"source_hash":snapshot.stamp.source_hash,"bodies":bodies,"body_sources":snapshot.body_sources,"occurrences":occurrences,"leaf_occurrences":leaf_occurrences,"parameters":parameters,"placement_drivers":placement_drivers,"producer_sources":producer_sources,"sketches":sketches,"consumed_bodies":snapshot.consumed_bodies,"references":snapshot.references,"references_resolved":snapshot.references.all_resolved(),"scope":"Saved canonical source regenerated with the production native model compiler and checked sketch solver. Occurrence bounds, volume and topology come from exact world B-reps; display meshes are not manufacturing input."}),
    )
}
fn write_new(path: &Path, report: &serde_json::Value) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}
fn main() -> Result<(), Error> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [project, report_path] = args.as_slice() else {
        return Err("Usage: model_file_review PROJECT.multicad NEW_REPORT.json".into());
    };
    let file = files::ProjectStore::default()
        .open_project(&PathBuf::from(project))
        .map_err(|e| e.to_string())?
        .file;
    write_new(&PathBuf::from(report_path), &report(&file.document)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Request, Session},
        constraint_sketch::Dimension,
        document::{Entity, Mutation},
        parameters::BoundExpression,
    };
    fn fixture() -> (Document, model::InstanceCopyRequest, Entity) {
        let mut session =
            Session::new(Document::new("Saved exact occurrence report").unwrap()).unwrap();
        let parameter = Entity::new(
            "distance",
            EntityData::Parameter {
                expression: "30".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::CommandLine),
                label: "Parameter".into(),
                edits: vec![Mutation::Insert(parameter.clone())],
            })
            .unwrap();
        let primitive = model::PrimitiveRequest::new(
            "Source",
            model::PrimitiveParameters::Box {
                size: [model::length(5.), model::length(3.), model::length(2.)],
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::CommandLine),
            &primitive,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        let mut copy = model::InstanceCopyRequest::new(
            model::InstanceCopySource::Part {
                source: model::BodyInput {
                    body: primitive.body,
                    producer: primitive.feature,
                },
            },
            Default::default(),
        );
        copy.parameters.translation[0] = Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=$distance + 20", "mm", None)
                .unwrap(),
        };
        let prepared = model::prepare_instance_copies(
            &session,
            session.revision(),
            Actor::local(Channel::CommandLine),
            std::slice::from_ref(&copy),
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        (session.document().clone(), copy, parameter)
    }
    #[test]
    fn reports_real_world_occurrences_and_canonical_formula_binding_identities() -> Result<(), Error>
    {
        let (document, copy, parameter) = fixture();
        let original = document.clone();
        let result = report(&document)?;
        assert_eq!(result["bodies"].as_array().unwrap().len(), 1);
        assert_eq!(result["occurrences"].as_array().unwrap().len(), 1);
        let occurrence = &result["occurrences"][0];
        assert_eq!(occurrence["instance"], serde_json::json!(copy.instance));
        assert_eq!(
            occurrence["pose"]["translation_mm"],
            serde_json::json!([50., 0., 0.])
        );
        assert_eq!(
            occurrence["bounds"]["min"],
            serde_json::json!([50., 0., 0.])
        );
        assert_eq!(
            occurrence["bounds"]["max"],
            serde_json::json!([55., 3., 2.])
        );
        assert!((occurrence["volume_mm3"].as_f64().unwrap() - 30.).abs() < 1e-9);
        assert_eq!(occurrence["solid_count"], 1);
        assert_eq!(
            result["placement_drivers"][0]["id"],
            serde_json::json!(copy.driver)
        );
        let dimension = &result["placement_drivers"][0]["data"]["definition"]["data"]["parameters"]
            ["translation"][0];
        assert_eq!(
            dimension,
            &serde_json::to_value(&copy.parameters.translation[0])?
        );
        assert_eq!(
            dimension["expression"]["bindings"][0]["target"],
            serde_json::json!(parameter.id)
        );
        assert_eq!(result["references_resolved"], true);
        assert_eq!(document, original);
        Ok(())
    }
    #[test]
    fn report_creation_refuses_existing_paths_and_keeps_previous_bytes() -> Result<(), Error> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("report.json");
        let report = serde_json::json!({"review":"original"});
        write_new(&path, &report)?;
        let bytes = std::fs::read(&path)?;
        assert!(write_new(&path, &serde_json::json!({"review":"replacement"})).is_err());
        assert_eq!(std::fs::read(&path)?, bytes);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes)?, report);
        Ok(())
    }
}
