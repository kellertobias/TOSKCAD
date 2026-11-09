use super::*;
use multicad_core::{
    commands::Actor,
    exchange::{WrittenFile, filename_token},
    project_file::ContentHash,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{Solid, rust_kernel::step_read};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
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
fn verify(
    exporter: &Exporter,
    built: &Built,
    artifact: &ExportArtifact,
    path: &Path,
) -> Result<(), ExportError> {
    if let Some(evidence) = &artifact.schedule {
        let expected = built
            .generated
            .get(&artifact.output.filename)
            .ok_or_else(|| io("Current schedule output is missing"))?;
        let written = fs::read(path).map_err(io)?;
        if written != *expected || ContentHash::of(&written) != evidence.csv_hash {
            return Err(io(
                "Written schedule CSV differs from the generated schedule",
            ));
        }
        return verify_csv(&written, evidence)
            .map_err(|e| failure(ErrorKind::Invalid, Some(artifact.producer), e));
    }
    if let Some(evidence) = &artifact.drawing {
        let expected = built
            .generated
            .get(&artifact.output.filename)
            .ok_or_else(|| io("Current drawing output is missing"))?;
        let written = fs::read(path).map_err(io)?;
        if written != *expected {
            return Err(io("Written drawing bytes differ from the generated output"));
        }
        let text = String::from_utf8_lossy(&written);
        let structure_ok = if artifact.output.filename.ends_with(".pdf") {
            written.starts_with(b"%PDF-1.4")
                && text.ends_with("%%EOF\n")
                && text.matches("/Type /Page ").count() == evidence.pages.len()
        } else {
            text.starts_with("0\nSECTION\n2\nHEADER")
                && text.ends_with("0\nEOF\n")
                && evidence.pages.len() == 1
        };
        if !structure_ok {
            return Err(failure(
                ErrorKind::Invalid,
                Some(artifact.producer),
                "Drawing readback structure or page count differs from the plan",
            ));
        }
        return Ok(());
    }
    if let Some(evidence) = &artifact.mesh {
        let expected = built
            .generated
            .get(&artifact.output.filename)
            .ok_or_else(|| io("Current exact mesh output is missing"))?;
        return verify_mesh(&fs::read(path).map_err(io)?, expected, artifact, evidence);
    }
    if artifact.planar.is_some() {
        let expected = built
            .generated
            .get(&artifact.output.filename)
            .ok_or_else(|| io("Current exact planar output is missing"))?;
        let (_, _) = hash_file(path)?;
        return dxf::verify(&fs::read(path).map_err(io)?, expected).map_err(io);
    }
    let tolerance = exporter.compiler.policy.linear_tolerance_mm;
    let (count, volume, bounds) = if built.backend == GeometryBackend::Rust {
        let readback = verify_rust_step(built, artifact, path, tolerance)?;
        (
            readback.solid_count,
            readback.volume_mm3,
            readback.bounds_mm,
        )
    } else {
        let read = Solid::read_step(path, exporter.compiler.policy).map_err(io)?;
        let bounds = read.bounds().map_err(io)?;
        (
            read.solid_count().map_err(io)?,
            read.volume_mm3().map_err(io)?,
            [bounds.min, bounds.max],
        )
    };
    let [a, b] = artifact.bounds_mm;
    let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
    let volume_tolerance =
        (artifact.volume_mm3 * 1e-8).max(tolerance * 2. * (x * y + y * z + z * x));
    if count != artifact.solid_count
        || (volume - artifact.volume_mm3).abs() > volume_tolerance
        || bounds
            .into_iter()
            .flatten()
            .zip(a.into_iter().chain(b))
            .any(|(x, y)| (x - y).abs() > tolerance * 2.)
    {
        return Err(failure(
            ErrorKind::Invalid,
            Some(artifact.producer),
            "STEP readback differs from the current solid's count, volume or world bounds",
        ));
    }
    Ok(())
}

type Segment = [[f64; 3]; 2];

fn verify_rust_step(
    built: &Built,
    artifact: &ExportArtifact,
    path: &Path,
    tolerance: f64,
) -> Result<step_read::Readback, ExportError> {
    let invalid = |message: String| failure(ErrorKind::Invalid, Some(artifact.producer), message);
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(io)?
        .take(step_read::MAX_PLANAR_STEP_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > step_read::MAX_PLANAR_STEP_BYTES {
        return Err(failure(
            ErrorKind::Budget,
            Some(artifact.producer),
            "STEP readback exceeds the file budget",
        ));
    }
    let source = std::str::from_utf8(&bytes).map_err(|e| invalid(e.to_string()))?;
    let readback = step_read::read_planar_solid(source, tolerance).map_err(|e| {
        let kind = match e {
            step_read::ReadError::Limit => ErrorKind::Budget,
            _ => ErrorKind::Invalid,
        };
        failure(
            kind,
            Some(artifact.producer),
            format!("Rust STEP readback rejected the solid: {e:?}"),
        )
    })?;
    let shape = built
        .model
        .as_ref()
        .and_then(|model| model.snapshot().shapes.get(&artifact.solid))
        .ok_or_else(|| {
            invalid("Rust STEP verification requires the exact source topology".into())
        })?;
    let mut faces = 0;
    let mut edges = Vec::new();
    for element in &shape.topology.elements {
        match element.key.kind {
            TopologyKind::Face => faces += 1,
            TopologyKind::Edge => match element.geometry {
                Some(AnalyticGeometry::Line { a_mm, b_mm }) => edges.push([a_mm, b_mm]),
                _ => return Err(invalid("Rust STEP source contains a nonline edge".into())),
            },
            TopologyKind::Vertex => {}
        }
    }
    if readback.face_count != faces
        || !same_segments(
            &readback.edges_mm,
            &edges,
            readback.bounds_mm,
            tolerance * 2.,
        )
    {
        return Err(invalid(
            "STEP readback differs from the source face count or analytic edges".into(),
        ));
    }
    Ok(readback)
}

fn same_segments(
    actual: &[Segment],
    expected: &[Segment],
    bounds: [[f64; 3]; 2],
    tolerance: f64,
) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    // Index both directions by the largest spatial extent. This avoids an
    // all-pairs comparison without relying on input order or edge winding.
    let axis = (0..3)
        .max_by(|&a, &b| (bounds[1][a] - bounds[0][a]).total_cmp(&(bounds[1][b] - bounds[0][b])))
        .expect("three axes");
    let mut ordered: Vec<_> = actual
        .iter()
        .copied()
        .enumerate()
        .flat_map(|(i, [a, b])| [(i, [a, b]), (i, [b, a])])
        .collect();
    ordered.sort_unstable_by(|a, b| a.1[0][axis].total_cmp(&b.1[0][axis]));
    let mut used = vec![false; actual.len()];
    expected.iter().all(|edge| {
        let x = edge[0][axis];
        let first = ordered.partition_point(|(_, p)| p[0][axis] < x - tolerance);
        let hit = ordered[first..]
            .iter()
            .take_while(|(_, p)| p[0][axis] <= x + tolerance)
            .filter(|(i, _)| !used[*i])
            .filter_map(|(i, p)| {
                let error = p
                    .iter()
                    .flatten()
                    .zip(edge.iter().flatten())
                    .map(|(a, b)| (a - b).abs())
                    .fold(0., f64::max);
                (error <= tolerance).then_some((*i, error))
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b));
        if let Some((i, _)) = hit {
            used[i] = true;
            true
        } else {
            false
        }
    })
}
/// Read a written OBJ, 3MF, STL or GLB back and compare it with the generated bytes,
/// the exact solid's world bounds and the tessellated volume.
fn verify_mesh(
    written: &[u8],
    expected: &[u8],
    artifact: &ExportArtifact,
    evidence: &MeshEvidence,
) -> Result<(), ExportError> {
    if written != expected {
        return Err(io("Written mesh bytes differ from the generated output"));
    }
    let name = &artifact.output.filename;
    // STL and GLB store single-precision coordinates; STL keeps no objects.
    let (read, single, flat) = if name.ends_with(".obj") {
        (super::mesh::read_obj(written), false, false)
    } else if name.ends_with(".stl") {
        (super::mesh::read_stl(written), true, true)
    } else if name.ends_with(".glb") {
        (super::mesh::read_glb(written), true, false)
    } else {
        (super::mesh::read_three_mf(written), false, false)
    };
    let read = read.map_err(io)?;
    let bounds = read
        .bounds_mm()
        .ok_or_else(|| io("Mesh readback is empty"))?;
    let [a, b] = artifact.bounds_mm;
    let extent = a.iter().chain(&b).fold(1f64, |m, v| m.max(v.abs()));
    let precision = if single { extent * 1e-6 } else { 0. };
    let slack = evidence.chord_deviation_mm * 2. + 1e-6 + precision;
    let relative = if single { 1e-5 } else { 1e-6 };
    let volume_ok = (read.volume_mm3() - evidence.mesh_volume_mm3).abs()
        <= evidence.mesh_volume_mm3.abs() * relative + 1e-6;
    // The tessellation must also enclose the exact solid's volume, up to the
    // chord deviation over its surface; a mis-oriented or open mesh fails.
    let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
    let chord_allowance = 2. * (x * y + y * z + z * x) * evidence.chord_deviation_mm * 1.5
        + artifact.volume_mm3 * 1e-6;
    let exact_ok = (read.volume_mm3() - artifact.volume_mm3).abs() <= chord_allowance;
    if (!flat && (read.objects != evidence.objects || read.items != evidence.build_items))
        || read.triangles != evidence.triangles
        || !volume_ok
        || !exact_ok
        || bounds[0].iter().zip(a).any(|(x, y)| (x - y).abs() > slack)
        || bounds[1].iter().zip(b).any(|(x, y)| (x - y).abs() > slack)
    {
        return Err(failure(
            ErrorKind::Invalid,
            Some(artifact.producer),
            "Mesh readback differs from the generated objects, placements, volume or exact world bounds",
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
    actor: &Actor,
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
        verify(exporter, built, current, &path)?;
        stored.artifact = current.clone();
    }
    Ok(RunReceipt {
        actor: actor.clone(),
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
    actor: &Actor,
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
        return reuse(exporter, built, &directory, actor, interrupt);
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
        if let Some(bytes) = built.generated.get(&artifact.output.filename) {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(io)?;
            file.write_all(bytes).map_err(io)?;
        } else {
            artifact_solid(model, artifact)?
                .write_step(&path)
                .map_err(io)?;
        }
        checkpoint(interrupt)?;
        verify(exporter, built, artifact, &path)?;
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
        actor: actor.clone(),
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
        actor: actor.clone(),
        key: preview.key.clone(),
        directory: directory
            .to_str()
            .ok_or_else(|| io("Export path must be UTF-8"))?
            .into(),
        files,
        reused: false,
    })
}
/// Read a written schedule CSV back: its kind and revision comments, a
/// header and one record per row, each with every column.
fn verify_csv(written: &[u8], evidence: &ScheduleEvidence) -> Result<(), String> {
    let text = std::str::from_utf8(written).map_err(|_| "Schedule CSV is not UTF-8")?;
    let (comments, records) = crate::schedules::read_csv(text)?;
    let kind = format!("MultiCAD {} schedule", evidence.spec.kind.token());
    let revision = format!("revision {}", evidence.revision);
    if comments.first() != Some(&kind)
        || comments.get(1) != Some(&revision)
        || records.len() != evidence.rows + 1
        || records.iter().any(|r| r.len() != evidence.columns)
    {
        return Err("Schedule CSV readback differs from the planned kind, revision or size".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::same_segments;

    #[test]
    fn step_edge_match_is_unordered_and_cannot_reuse_or_move_a_boundary() {
        let bounds = [[0., 0., 0.], [8., 4., 2.]];
        let a = [[0., 0., 0.], [8., 0., 0.]];
        let b = [[8., 0., 0.], [8., 4., 0.]];
        let reversed_with_noise = [[8. + 1e-8, 0., 0.], [0., 0., 0.]];
        assert!(same_segments(
            &[b, reversed_with_noise],
            &[a, b],
            bounds,
            1e-7
        ));
        assert!(!same_segments(
            &[b, reversed_with_noise],
            &[a, a],
            bounds,
            1e-7
        ));
        assert!(!same_segments(&[b], &[a, b], bounds, 1e-7));
        let moved = [[8., 0., 0.], [8., 3., 0.]];
        assert!(!same_segments(&[a, moved], &[a, b], bounds, 1e-7));
    }

    #[test]
    fn step_edge_match_prefers_the_closest_of_nearby_boundaries() {
        let bounds = [[0., 0., 0.], [8., 4., 2.]];
        let edge = |x| [[x, 0., 0.], [x, 4., 0.]];
        let [a, b, c] = [edge(1.), edge(1.0075), edge(1.015)];
        assert!(same_segments(&[a, b, c], &[b, c, a], bounds, 0.01));
    }
}
