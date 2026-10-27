//! Emits real checked file fixtures; does not implement a production feature compiler.
#[path = "../src/files.rs"]
mod files;
use multicad_core::{document::*, project_file::*};
use multicad_kernel::Solid;
use multicad_ui::storage::{DocumentStorage, SaveMode};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/project-file-review");
    std::fs::create_dir_all(&directory)?;
    let mut store = files::ProjectStore::new(directory.join("recovery"));
    if let Some(path) = args.next() {
        let opened = store
            .open_project(std::path::Path::new(&path))
            .map_err(|e| e.to_string())?;
        let output = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.join("verified-resources.json"));
        report(&opened.file, &output)?;
        return Ok(());
    }
    let parameter = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "600".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let mut edits = vec![Mutation::Insert(parameter)];
    let mut instances = vec![];
    let mut payloads = BTreeMap::new();
    for (index, name, density, color) in [
        (0, "Birch", 700., [190, 150, 105, 255]),
        (1, "Steel", 7850., [130, 150, 175, 255]),
    ] {
        let material = Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: Some(density),
                color_rgba: color,
                section_hatch: None,
            },
        );
        let feature = Entity::new(
            format!("{name} exact source"),
            EntityData::Feature {
                definition: Definition {
                    type_id: "unavailable.review.step".into(),
                    schema_version: 17,
                    data: serde_json::json!({"opaque_provider_field":{"preserve":true}}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let part = Entity::new(
            format!("{name} panel"),
            EntityData::Part {
                features: vec![feature.id],
                material: Some(material.id),
                tags: BTreeSet::from(["maker-space".into()]),
            },
        );
        let instance = Entity::new(
            format!("{name} instance"),
            EntityData::Instance {
                component: part.id,
                placement: Placement {
                    translation_mm: [index as f64 * 50., 0., 0.],
                    ..Default::default()
                },
            },
        );
        instances.push(instance.id);
        let step = directory.join(format!("{name}.step"));
        Solid::box_named([20., 30., 8.], feature.id, Default::default())?.write_step(&step)?;
        let payload = Payload::new(
            "kernel.source.step",
            "model/step",
            Some(feature.id),
            std::fs::read(step)?,
        );
        payloads.insert(payload.descriptor.id, payload);
        edits.extend(
            [material, feature, part, instance]
                .into_iter()
                .map(Mutation::Insert),
        );
    }
    edits.push(Mutation::Insert(Entity::new(
        "Workshop assembly",
        EntityData::Assembly { instances },
    )));
    let document = Document::new("Native file review")?.edited(&edits)?;
    let mut project = ProjectFile::new(document)?;
    project.resources.payloads = payloads;
    let symbol = ProjectFile::new(Document::new("Pinned Door definition")?)?;
    let pin = Payload::pinned_symbol(EntityId::new(), "door-revision-42", &symbol)?;
    project.resources.payloads.insert(pin.descriptor.id, pin);
    project.resources.metadata.insert(
        "future_manifest_extension".into(),
        serde_json::json!({"must_survive":"UI edits and Save As"}),
    );
    let cache = DerivedCache::new(
        Payload::new(
            "display.mesh",
            "application/octet-stream",
            None,
            vec![1, 2, 3],
        ),
        project.cache_key("review.mesh", "occt-7.9.3/review-1", 1, BTreeMap::new())?,
    );
    project.resources.caches.insert(cache.descriptor.id, cache);
    let path = directory.join("mixed-material.multicad");
    let mode = if path.exists() {
        SaveMode::Replace(
            store
                .open_project(&path)
                .map_err(|e| e.to_string())?
                .stamp
                .ok_or("missing stamp")?,
        )
    } else {
        SaveMode::Create
    };
    store
        .save_project(&path, &project, mode)
        .map_err(|e| e.to_string())?;
    store
        .write_recovery(&project, 0, Some(&path))
        .map_err(|e| e.to_string())?;

    report(&project, &directory.join("expected-resources.json"))?;
    println!("Fixture: {}", path.display());
    Ok(())
}
fn report(project: &ProjectFile, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let payloads:Vec<_>=project.resources.payloads.values().map(|p|serde_json::json!({"id":p.descriptor.id,"kind":p.descriptor.kind,"owner":p.descriptor.owner,"sha256":ContentHash::of(&p.bytes).to_string(),"length":p.bytes.len(),"symbol_pin":p.descriptor.symbol_pin})).collect();
    let caches: Vec<_> = project
        .resources
        .caches
        .values()
        .map(|c| serde_json::json!({"id":c.descriptor.id,"key":c.key}))
        .collect();
    std::fs::write(
        path,
        serde_json::to_vec_pretty(
            &serde_json::json!({"document":project.document,"payloads":payloads,"caches":caches,"metadata":project.resources.metadata,"scope":"Real STEP/container preservation fixture. Geometry feature provider unavailable; production application must not pretend to render these feature definitions."}),
        )?,
    )?;
    println!("Resource report: {}", path.display());
    Ok(())
}
