//! Developer-only trusted native cache generation from actual shipped canonical recipes.
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{
        self, MetricFitEngagement, ResolvedMechanicalPart, ResolvedThread, ThreadFit, ThreadHand,
        ThreadMode, ThreadParameters, ThreadRepresentation,
    },
    parameters, project_file, symbols,
};
use multicad_kernel::{GeometryPolicy, HelicalThread, RootRoundedHelicalThread, thread_prototypes};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Registry {
    Catalog,
    FittedFlat,
    SymmetricRoot,
}
#[derive(Clone, Copy)]
enum Spec {
    Flat(HelicalThread),
    Root(RootRoundedHelicalThread),
}
impl Spec {
    fn key(
        self,
        policy: GeometryPolicy,
    ) -> std::result::Result<String, multicad_kernel::GeometryError> {
        match self {
            Self::Flat(s) => thread_prototypes::key(s, policy),
            Self::Root(s) => thread_prototypes::root_rounded_key(s, policy),
        }
    }
    fn generate(
        self,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> std::result::Result<thread_prototypes::GeneratedPrototype, multicad_kernel::GeometryError>
    {
        match self {
            Self::Flat(s) => thread_prototypes::generate(s, feature, policy),
            Self::Root(s) => thread_prototypes::generate_root_rounded(s, feature, policy),
        }
    }
}
#[derive(Clone)]
struct Job {
    key: String,
    spec: Spec,
    registry: Registry,
    canonical: Vec<serde_json::Value>,
    sources: Vec<String>,
}
fn native_spec(thread: &ResolvedThread) -> Result<Spec> {
    if thread.representation != ThreadRepresentation::Modeled
        || thread.buttress.is_some()
        || thread.round.is_some()
        || thread.npt.is_some()
        || thread.pipe.is_some()
    {
        return Err("Prototype job has an unsupported thread profile or representation".into());
    }
    let profile = HelicalThread {
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
    };
    Ok(
        if let Some(root_radius_mm) = thread
            .metric_fit
            .as_ref()
            .and_then(|fit| fit.external_root_radius_mm)
        {
            Spec::Root(RootRoundedHelicalThread {
                profile,
                root_radius_mm,
            })
        } else {
            Spec::Flat(profile)
        },
    )
}
fn authored_spec(document: &Document) -> Result<Spec> {
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
    native_spec(&thread)
}
fn jobs(library: &Path) -> Result<(Vec<Job>, String, usize)> {
    let policy = GeometryPolicy::default();
    let evaluated = parameters::evaluate(&Document::new("Canonical default")?)?;
    let ResolvedMechanicalPart::MachineScrew { thread, .. } =
        model::default_machine_screw().resolve(&evaluated)?
    else {
        unreachable!()
    };
    let spec = native_spec(&thread)?;
    let default_key = spec.key(policy)?;
    let mut unique = BTreeMap::from([(
        default_key.clone(),
        Job {
            key: default_key.clone(),
            spec,
            registry: Registry::Catalog,
            canonical: vec![],
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
        let key = spec.key(policy)?;
        let definitions = symbols::definitions(&project.document)?;
        if definitions.len() != 1 || authored_spec(&definitions[0].1.template)?.key(policy)? != key
        {
            return Err(format!("Author/template thread mismatch for {file}").into());
        }
        unique
            .entry(key.clone())
            .and_modify(|job| job.sources.push(file.into()))
            .or_insert(Job {
                key,
                spec,
                registry: Registry::Catalog,
                canonical: vec![],
                sources: vec![file.into()],
            });
    }
    // Resolve real canonical fitted source, never reinterpret a flat nominal
    // profile as a circular-root fit. Physical span and fit-group intent remain distinct.
    for (label, fit, length, mode) in [
        (
            "fitted-M3-6g-physical3",
            ThreadFit::Metric6gExternal,
            3.,
            ThreadMode::NewExternal,
        ),
        (
            "fitted-M3-6g-physical8",
            ThreadFit::Metric6gExternal,
            8.,
            ThreadMode::NewExternal,
        ),
        (
            "fitted-M3-6g-physical12",
            ThreadFit::Metric6gExternal,
            12.,
            ThreadMode::NewExternal,
        ),
        (
            "fitted-M3-6H-physical2.4",
            ThreadFit::Metric6HInternal,
            2.4,
            ThreadMode::CutInternal,
        ),
        (
            "fitted-M3-6H-physical2.5",
            ThreadFit::Metric6HInternal,
            2.5,
            ThreadMode::CutInternal,
        ),
    ] {
        let source = ThreadParameters {
            major_diameter: model::length(3.),
            pitch: model::length(0.5),
            length: model::length(length),
            fit,
            metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
            ..Default::default()
        };
        let resolved = source.resolve(&evaluated, mode)?;
        let spec = native_spec(&resolved)?;
        let key = spec.key(policy)?;
        let canonical = serde_json::json!({"source":source,"mode":mode,"resolved_major_radius_mm":resolved.major_radius_mm,"resolved_depth_mm":resolved.profile.depth_mm,"root_radius_mm":resolved.metric_fit.as_ref().and_then(|m|m.external_root_radius_mm)});
        if let Some(job) = unique.get_mut(&key) {
            job.sources.push(label.into());
            job.canonical.push(canonical);
        } else {
            unique.insert(
                key.clone(),
                Job {
                    key,
                    spec,
                    registry: match spec {
                        Spec::Flat(_) => Registry::FittedFlat,
                        Spec::Root(_) => Registry::SymmetricRoot,
                    },
                    sources: vec![label.into()],
                    canonical: vec![canonical],
                },
            );
        }
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
                    let result = job
                        .spec
                        .generate(EntityId::new(), GeometryPolicy::default())
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
    let mut fitted_flat = String::from(
        "// Generated derived fitted flat-thread registrations.\nconst FITTED_FLAT_REGISTRY: &[Registration] = &[\n",
    );
    let mut root_registry = String::from(
        "// Generated derived symmetric circular-root registrations.\nconst ROOT_REGISTRY: &[RootRegistration] = &[\n",
    );
    let mut report = Vec::new();
    // All source recipes have completed native roundtrip validation before any
    // registered output changes. No STEP work runs concurrently (CAD-115).
    std::fs::create_dir_all(&output)?;
    for (key, (job, artifact)) in completed {
        let filename = if key == default_key {
            "default-m3.brep".to_string()
        } else if job.registry == Registry::SymmetricRoot {
            format!("metric-root-rounded-{key}.brep")
        } else {
            format!("metric-{key}.brep")
        };
        std::fs::write(output.join(&filename), &artifact.brep)?;
        let table = match job.registry {
            Registry::Catalog => &mut registry,
            Registry::FittedFlat => &mut fitted_flat,
            Registry::SymmetricRoot => &mut root_registry,
        };
        table.push_str(&artifact.registration.replace("__BREP_FILE__", &filename));
        table.push_str(",\n");
        report.push(serde_json::json!({"key":key,"sources":job.sources,"canonical_fitted_sources":job.canonical,"registry":format!("{:?}",job.registry),"file":filename,
            "fresh_seconds":artifact.fresh_seconds,"load_seconds":artifact.load_seconds,
            "brep_bytes":artifact.brep.len(),"volume_mm3":artifact.volume_mm3,"faces":artifact.face_count,"raw_seed_count":artifact.raw_seed_count,"seed_count":artifact.seed_count}));
    }
    registry.push_str("];\n");
    std::fs::write(output.join("metric.rs"), registry)?;
    fitted_flat.push_str("];\n");
    root_registry.push_str("];\n");
    std::fs::write(output.join("fitted_flat.rs"), fitted_flat)?;
    std::fs::write(output.join("root_rounded.rs"), root_registry)?;
    println!(
        "{}",
        serde_json::json!({"format":"multicad.thread-prototype-generation/3",
        "source_fingerprint":thread_prototypes::source_fingerprint(),"catalog_entries":catalog_entries,
        "unique_prototypes":count,"workers":workers.min(count),"elapsed_seconds":elapsed,
        "entries":report,"output":output})
    );
    Ok(())
}
