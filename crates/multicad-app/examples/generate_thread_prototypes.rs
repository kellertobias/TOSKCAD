//! Developer-only trusted native cache generation from the canonical default M3 recipe.
use multicad_core::{
    document::{Document, EntityId},
    model::{self, MechanicalPartParameters, ThreadHand, ThreadMode},
    parameters,
};
use multicad_kernel::{GeometryPolicy, HelicalThread, thread_prototypes};
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates/multicad-kernel/assets/thread-prototypes"));
    let MechanicalPartParameters::MachineScrew { thread, .. } = model::default_machine_screw()
    else {
        return Err("Canonical default is no longer a machine screw".into());
    };
    let document = Document::new("Trusted default M3 derivation")?;
    let evaluated = parameters::evaluate(&document)?;
    let resolved = thread.resolve(&evaluated, ThreadMode::NewExternal)?;
    if resolved.major_radius_mm != 1.5 || resolved.pitch_mm != 0.5 || resolved.length_mm != 10. {
        return Err(
            "Default M3 prototype generator needs explicit review for changed canonical dimensions"
                .into(),
        );
    }
    let profile = resolved.profile;
    let spec = HelicalThread {
        major_radius_mm: resolved.major_radius_mm,
        pitch_mm: resolved.pitch_mm,
        length_mm: resolved.length_mm,
        depth_mm: profile.depth_mm,
        crest_width_mm: profile.crest_width_mm,
        lower_flank_deg: profile.lower_flank_deg,
        upper_flank_deg: profile.upper_flank_deg,
        radial_taper: profile.radial_taper,
        starts: resolved.starts,
        left_hand: resolved.hand == ThreadHand::Left,
    };
    let artifact = thread_prototypes::generate(spec, EntityId::new(), GeometryPolicy::default())?;
    // Generation has already roundtripped actual native geometry and every
    // neutral role/face witness before any output replaces a registration.
    std::fs::create_dir_all(&output)?;
    std::fs::write(output.join("default-m3.brep"), &artifact.brep)?;
    std::fs::write(output.join("default-m3.rs"), &artifact.registration)?;
    println!(
        "{}",
        serde_json::json!({
            "format":"multicad.thread-prototype-generation/1",
            "canonical_recipe":"default_machine_screw M3 external thread",
            "source_fingerprint":thread_prototypes::source_fingerprint(),
            "fresh_seconds":artifact.fresh_seconds,
            "load_seconds":artifact.load_seconds,
            "brep_bytes":artifact.brep.len(),
            "volume_mm3":artifact.volume_mm3,
            "faces":artifact.face_count,
            "output":output,
        })
    );
    Ok(())
}
