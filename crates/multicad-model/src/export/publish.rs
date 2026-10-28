use super::*;
use multicad_core::{
    commands::Actor,
    exchange::{WrittenFile, filename_token},
    project_file::ContentHash,
};
use multicad_kernel::Solid;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
const MANIFEST: &str = "multicad-export.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    provider: String,
    set: EntityId,
    source_hash: ContentHash,
    plan_hash: ContentHash,
    actor: Actor,
    files: Vec<WrittenFile>,
}
fn io(e: impl ToString) -> ExportError {
    failure(ErrorKind::Io, None, e)
}
fn verify(exporter: &Exporter, artifact: &ExportArtifact, path: &Path) -> Result<(), ExportError> {
    let read = Solid::read_step(path, exporter.compiler.policy).map_err(io)?;
    let count = read.solid_count().map_err(io)?;
    let volume = read.volume_mm3().map_err(io)?;
    let bounds = read.bounds().map_err(io)?;
    let tolerance = exporter.compiler.policy.linear_tolerance_mm;
    let [a, b] = artifact.bounds_mm;
    let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
    let volume_tolerance =
        (artifact.volume_mm3 * 1e-8).max(tolerance * 2. * (x * y + y * z + z * x));
    if count != artifact.solid_count
        || (volume - artifact.volume_mm3).abs() > volume_tolerance
        || bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip(a.into_iter().chain(b))
            .any(|(x, y)| (x - y).abs() > tolerance * 2.)
    {
        return Err(failure(
            ErrorKind::Invalid,
            Some(artifact.producer),
            "STEP readback differs from the current native solid's count, volume or world bounds",
        ));
    }
    Ok(())
}
fn hash_file(path: &Path) -> Result<(ContentHash, u64), ExportError> {
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_STEP_BYTES {
        return Err(io("Output is not a regular bounded file"));
    }
    let bytes = fs::read(path).map_err(io)?;
    Ok((ContentHash::of(&bytes), bytes.len() as u64))
}
fn reuse(
    exporter: &Exporter,
    built: &Built,
    directory: &Path,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<RunReceipt, ExportError> {
    let metadata = fs::symlink_metadata(directory).map_err(io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io("Existing export destination is not an owned directory"));
    }
    let manifest_path = directory.join(MANIFEST);
    let metadata = fs::symlink_metadata(&manifest_path).map_err(|_| {
        io("Existing export destination has no ownership manifest; choose another destination")
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 2 * 1024 * 1024
    {
        return Err(io("Invalid export manifest file"));
    }
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(manifest_path).map_err(io)?).map_err(io)?;
    let preview = &built.preview;
    if manifest.schema != 1
        || manifest.provider != preview.provider
        || manifest.set != preview.key.set
        || manifest.source_hash != preview.key.source.source_hash
        || manifest.plan_hash != preview.key.plan_hash
        || manifest.files.len() != preview.artifacts.len()
    {
        return Err(io(
            "Existing export bundle belongs to different source or provider; nothing was overwritten",
        ));
    }
    let expected: BTreeSet<_> = preview
        .artifacts
        .iter()
        .map(|a| a.output.filename.as_str())
        .chain(std::iter::once(MANIFEST))
        .collect();
    let entries: Vec<_> = fs::read_dir(directory)
        .map_err(io)?
        .take(expected.len() + 1)
        .collect::<Result<_, _>>()
        .map_err(io)?;
    if entries.len() != expected.len() {
        return Err(io(
            "Export bundle contains additional or missing files; nothing was overwritten",
        ));
    }
    for entry in entries {
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| io("Unexpected filename in export bundle"))?;
        if !expected.contains(name) || !entry.file_type().map_err(io)?.is_file() {
            return Err(io(
                "Export bundle contains unowned files or symlinks; nothing was overwritten",
            ));
        }
    }
    for (stored, current) in manifest.files.iter_mut().zip(&preview.artifacts) {
        checkpoint(interrupt)?;
        if stored.artifact.output != current.output
            || stored.artifact.producer != current.producer
            || stored.artifact.solid != current.solid
        {
            return Err(io("Export bundle output bindings differ from current plan"));
        }
        let path = directory.join(&current.output.filename);
        let (hash, bytes) = hash_file(&path)?;
        if hash != stored.content_hash || bytes != stored.bytes {
            return Err(io("Export bundle was modified; nothing was overwritten"));
        }
        verify(exporter, current, &path)?;
        stored.artifact = current.clone();
    }
    Ok(RunReceipt {
        actor: preview.requested_by.clone(),
        key: preview.key.clone(),
        directory: directory
            .to_str()
            .ok_or_else(|| io("Export path must be UTF-8"))?
            .into(),
        files: manifest.files,
        reused: true,
    })
}
pub(super) fn run(
    exporter: &Exporter,
    built: &Built,
    root: &Path,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<RunReceipt, ExportError> {
    checkpoint(interrupt)?;
    if !root.is_absolute() || root.to_str().is_none() || !fs::metadata(root).map_err(io)?.is_dir() {
        return Err(failure(
            ErrorKind::Invalid,
            None,
            "Choose an existing absolute UTF-8 export directory",
        ));
    }
    let preview = &built.preview;
    let prefix = filename_token(&preview.selection.name);
    if prefix.len() > 160 {
        return Err(failure(
            ErrorKind::Invalid,
            Some(preview.key.set),
            "Export-set name is too long for its bundle directory",
        ));
    }
    let name = format!(
        "{}_{}_{}_{}",
        prefix,
        preview.key.set,
        &preview.key.source.source_hash.to_string()[..12],
        &ContentHash::of(preview.provider.as_bytes()).to_string()[..8]
    );
    let directory = root.join(&name);
    let lock_path = root.join(format!(".multicad-{}.lock", preview.key.set));
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(io("Export lock cannot be a symlink"));
    }
    // A persistent OS lock releases on crash; deleting lock files would allow
    // different inode locks to race across application instances.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(io)?;
    lock.try_lock().map_err(|e| {
        io(format!(
            "Export destination is busy or cannot be locked: {e}"
        ))
    })?;
    if fs::symlink_metadata(&directory).is_ok() {
        return reuse(exporter, built, &directory, interrupt);
    }
    let staging = tempfile::Builder::new()
        .prefix(".multicad-export-")
        .tempdir_in(root)
        .map_err(io)?;
    let model = built.model.as_ref().ok_or_else(|| {
        failure(
            ErrorKind::Invalid,
            None,
            "Export has no owned current model",
        )
    })?;
    let mut files = Vec::new();
    for artifact in &preview.artifacts {
        checkpoint(interrupt)?;
        let path = staging.path().join(&artifact.output.filename);
        model
            .solid(artifact.solid)
            .ok_or_else(|| {
                failure(
                    ErrorKind::Invalid,
                    Some(artifact.solid),
                    "Current native solid was lost",
                )
            })?
            .write_step(&path)
            .map_err(io)?;
        checkpoint(interrupt)?;
        verify(exporter, artifact, &path)?;
        let (content_hash, bytes) = hash_file(&path)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(io)?
            .sync_all()
            .map_err(io)?;
        files.push(WrittenFile {
            artifact: artifact.clone(),
            content_hash,
            bytes,
        });
    }
    let manifest = Manifest {
        schema: 1,
        provider: preview.provider.clone(),
        set: preview.key.set,
        source_hash: preview.key.source.source_hash,
        plan_hash: preview.key.plan_hash,
        actor: preview.requested_by.clone(),
        files: files.clone(),
    };
    let mut manifest_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging.path().join(MANIFEST))
        .map_err(io)?;
    manifest_file
        .write_all(&serde_json::to_vec_pretty(&manifest).map_err(io)?)
        .map_err(io)?;
    manifest_file.sync_all().map_err(io)?;
    drop(manifest_file);
    checkpoint(interrupt)?;
    fs::rename(staging.path(), &directory).map_err(io)?;
    let _ = staging.keep();
    // Publication succeeded. A later cancellation cannot relabel a completed run.
    Ok(RunReceipt {
        actor: preview.requested_by.clone(),
        key: preview.key.clone(),
        directory: directory
            .to_str()
            .ok_or_else(|| io("Export path must be UTF-8"))?
            .into(),
        files,
        reused: false,
    })
}
