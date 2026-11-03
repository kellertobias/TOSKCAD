//! Developer-only trusted native cache generation from actual shipped canonical recipes.
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, ResolvedMechanicalPart, ResolvedThread, ThreadHand},
    parameters, project_file, symbols,
};
use multicad_kernel::{GeometryPolicy, HelicalThread, thread_prototypes};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Clone)]
struct Job {
    key: String,
    spec: HelicalThread,
    sources: Vec<String>,
}
fn native_spec(thread: &ResolvedThread) -> HelicalThread {
    HelicalThread {
        major_radius_mm: thread.major_radius_mm,
        pitch_mm: thread.pitch_mm,
        length_mm: thread.length_mm,
        depth_mm: thread.profile.depth_mm,
        crest_width_mm: thread.profile.crest_width_mm,
        lower_flank_deg: thread.profile.lower_flank_deg,
        upper_flank_deg: thread.profile.upper_flank_deg,
        radial_taper: thread.profile.radial_taper,
        starts: thread.starts,
        left_hand: thread.hand == ThreadHand::Left,
    }
}
fn authored_spec(document: &Document) -> Result<HelicalThread> {
    let features: Vec<_> = document.entities().filter(|e| matches!(&e.data,EntityData::Feature {definition,..} if definition.type_id==model::MECHANICAL_PART_TYPE)).collect();
    if features.len() != 1 {
        return Err("Catalog source must have exactly one mechanical recipe".into());
    }
    let source = model::read_mechanical_part(document, features[0].id)?;
    let evaluated = parameters::evaluate(document)?;
    let ResolvedMechanicalPart::MachineScrew { thread, .. } =
        source.parameters.resolve(&evaluated)?
    else {
        return Err("Catalog source is not a machine screw".into());
    };
    Ok(native_spec(&thread))
}
fn jobs(library: &Path) -> Result<(Vec<Job>, String, usize)> {
    let policy = GeometryPolicy::default();
    let evaluated = parameters::evaluate(&Document::new("Canonical default")?)?;
    let ResolvedMechanicalPart::MachineScrew { thread, .. } =
        model::default_machine_screw().resolve(&evaluated)?
    else {
        unreachable!()
    };
    let spec = native_spec(&thread);
    let default_key = thread_prototypes::key(spec, policy)?;
    let mut unique = BTreeMap::from([(
        default_key.clone(),
        Job {
            key: default_key.clone(),
            spec,
            sources: vec!["canonical-default-M3".into()],
        },
    )]);
    let catalog: serde_json::Value = serde_json::from_slice(&std::fs::read(
        library.join("machine-screws-curation.json"),
    )?)?;
    let entries = catalog["entries"]
        .as_array()
        .ok_or("Missing catalog entries")?;
    if entries.len() != 19 {
        return Err("Expected nineteen reviewed shipped catalog recipes".into());
    }
    for entry in entries {
        let file = entry["file"].as_str().ok_or("Missing catalog filename")?;
        if Path::new(file).components().count() != 1 {
            return Err("Catalog filename must be a bare name".into());
        }
        let bytes = std::fs::read(library.join(file))?;
        let project = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )?
        .project;
        let spec = authored_spec(&project.document)?;
        let key = thread_prototypes::key(spec, policy)?;
        let definitions = symbols::definitions(&project.document)?;
        if definitions.len() != 1
            || thread_prototypes::key(authored_spec(&definitions[0].1.template)?, policy)? != key
        {
            return Err(format!("Author/template thread mismatch for {file}").into());
        }
        unique
            .entry(key.clone())
            .and_modify(|job| job.sources.push(file.into()))
            .or_insert(Job {
                key,
                spec,
                sources: vec![file.into()],
            });
    }
    Ok((unique.into_values().collect(), default_key, entries.len()))
}
fn main() -> Result<()> {
    let mut output = PathBuf::from("crates/multicad-kernel/assets/thread-prototypes");
    let mut workers = 4usize;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--jobs" {
            workers = args
                .next()
                .ok_or("--jobs needs a count")?
                .to_str()
                .ok_or("Invalid job count")?
                .parse()?;
        } else {
            output = PathBuf::from(arg);
        }
    }
    if !(1..=8).contains(&workers) {
        return Err("Generation allows 1–8 native owner workers".into());
    }
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let (jobs, default_key, catalog_entries) = jobs(&library)?;
    let count = jobs.len();
    let pending = Arc::new(Mutex::new(VecDeque::from(jobs)));
    let (send, receive) = mpsc::channel();
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..workers.min(count) {
            let pending = Arc::clone(&pending);
            let send = send.clone();
            scope.spawn(move || {
                loop {
                    let Some(job) = pending.lock().unwrap().pop_front() else {
                        break;
                    };
                    // Every native handle is created and dropped on this worker.
                    // Only owned bytes, numeric receipts and error text cross back.
                    let result = thread_prototypes::generate(
                        job.spec,
                        EntityId::new(),
                        GeometryPolicy::default(),
                    )
                    .map_err(|e| e.to_string());
                    if send.send((job, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(send);
    });
    let mut completed = BTreeMap::new();
    for (job, result) in receive {
        completed.insert(
            job.key.clone(),
            (
                job,
                result.map_err(|e| format!("Native prototype generation failed: {e}"))?,
            ),
        );
    }
    if completed.len() != count {
        return Err("A native prototype worker exited without completing its receipt".into());
    }
    let elapsed = started.elapsed().as_secs_f64();
    let mut registry = String::from(
        "// Generated derived native registry; rebuild from canonical catalog recipes.\nconst REGISTRY: &[Registration] = &[\n",
    );
    let mut report = Vec::new();
    // All source recipes have completed native roundtrip validation before any
    // registered output changes. No STEP work runs concurrently (CAD-115).
    std::fs::create_dir_all(&output)?;
    for (key, (job, artifact)) in completed {
        let filename = if key == default_key {
            "default-m3.brep".to_string()
        } else {
            format!("metric-{key}.brep")
        };
        std::fs::write(output.join(&filename), &artifact.brep)?;
        registry.push_str(&artifact.registration.replace("__BREP_FILE__", &filename));
        registry.push_str(",\n");
        report.push(serde_json::json!({"key":key,"sources":job.sources,"file":filename,
            "fresh_seconds":artifact.fresh_seconds,"load_seconds":artifact.load_seconds,
            "brep_bytes":artifact.brep.len(),"volume_mm3":artifact.volume_mm3,"faces":artifact.face_count}));
    }
    registry.push_str("];\n");
    std::fs::write(output.join("metric.rs"), registry)?;
    println!(
        "{}",
        serde_json::json!({"format":"multicad.thread-prototype-generation/2",
        "source_fingerprint":thread_prototypes::source_fingerprint(),"catalog_entries":catalog_entries,
        "unique_prototypes":count,"workers":workers.min(count),"elapsed_seconds":elapsed,
        "entries":report,"output":output})
    );
    Ok(())
}
