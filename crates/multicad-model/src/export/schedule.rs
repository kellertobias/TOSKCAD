//! Direct schedule CSV: build one schedule on an exact source snapshot and,
//! when asked, write exactly that CSV. A write names the CSV hash it expects,
//! so a changed source writes nothing.
use super::*;
use multicad_core::schedules::ScheduleSpec;
use std::{fs, io::Write};

/// Where a schedule CSV is written and the CSV hash the caller accepted.
pub struct ScheduleTarget<'a> {
    pub path: &'a Path,
    pub expected: ContentHash,
}

fn io(e: impl ToString) -> ExportError {
    failure(ErrorKind::Io, None, e)
}

/// The written path must be an absolute UTF-8 `.csv` file in an existing
/// directory, and never an existing symlink or directory.
fn check_target(path: &Path) -> Result<(), ExportError> {
    let invalid = |m: &str| failure(ErrorKind::Invalid, None, m);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("Choose a UTF-8 CSV file name"))?;
    if !path.is_absolute() || path.to_str().is_none() {
        return Err(invalid("Choose an absolute UTF-8 CSV path"));
    }
    if !name.to_ascii_lowercase().ends_with(".csv") || name.starts_with('.') || name.len() > 200 {
        return Err(invalid("The schedule file name must end in .csv"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Choose a CSV path in a folder"))?;
    if !fs::metadata(parent).map_err(io)?.is_dir() {
        return Err(invalid("The CSV folder does not exist"));
    }
    if let Ok(meta) = fs::symlink_metadata(path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        return Err(invalid(
            "The CSV path is a folder or symlink; nothing was written",
        ));
    }
    Ok(())
}

/// Write through a sibling partial file, flush it, then rename it over the
/// target, so a reader never sees a half-written CSV.
fn publish(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().expect("checked CSV folder")).map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.as_file().sync_all().map_err(io)?;
    if fs::read(file.path()).map_err(io)? != bytes {
        return Err(io(
            "Written schedule CSV differs from the generated schedule",
        ));
    }
    file.persist(path).map_err(io)?;
    Ok(())
}

impl Exporter {
    /// Build `spec` on the snapshot; with a target, write its CSV only when
    /// it hashes to the accepted value.
    pub fn schedule(
        &self,
        document: &Document,
        revision: u64,
        spec: ScheduleSpec,
        target: Option<ScheduleTarget<'_>>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ScheduleReceipt, ExportError> {
        self.with_backend(GeometryBackend::default(), None)
            .schedule(document, revision, spec, target, interrupt)
    }
}

impl GeometryExporter<'_> {
    pub(super) fn schedule(
        &self,
        document: &Document,
        revision: u64,
        spec: ScheduleSpec,
        target: Option<ScheduleTarget<'_>>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ScheduleReceipt, ExportError> {
        let spec = spec.normalized();
        let source = ExportStamp::capture(document, revision)?;
        if let Some(target) = &target {
            check_target(target.path)?;
        }
        checkpoint(interrupt)?;
        let model = self.compile_geometry(document, revision, interrupt)?;
        checkpoint(interrupt)?;
        if model.snapshot().stamp.source_hash != source.source_hash {
            return Err(failure(
                ErrorKind::Stale,
                None,
                "Model source differs from the schedule request",
            ));
        }
        let schedule = crate::schedules::build(&model, document, revision, spec).map_err(|e| {
            failure(
                ErrorKind::Unsupported,
                e.entity,
                format!("The {} cannot be built: {}", spec.label(), e.message),
            )
        })?;
        let csv = schedule.csv();
        let csv_hash = ContentHash::of(csv.as_bytes());
        let mut written = None;
        if let Some(target) = target {
            if csv_hash != target.expected {
                return Err(failure(
                    ErrorKind::Stale,
                    None,
                    format!(
                        "The {} changed since it was previewed; nothing was written. Preview it again",
                        spec.label()
                    ),
                ));
            }
            checkpoint(interrupt)?;
            publish(target.path, csv.as_bytes())?;
            written = Some(target.path.to_str().expect("checked UTF-8").to_owned());
        }
        Ok(ScheduleReceipt {
            spec,
            source,
            title: schedule.title,
            notes: schedule.notes,
            columns: schedule.columns,
            rows: schedule.rows,
            csv,
            csv_hash,
            written,
        })
    }
}
