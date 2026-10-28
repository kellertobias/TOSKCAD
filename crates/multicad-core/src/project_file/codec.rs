use super::model::*;
use crate::document::{Document, EntityId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    sync::Arc,
};
const MAGIC: &[u8; 8] = b"MCADPRJ\0";
const HEADER_BYTES: u64 = 52;
pub const CURRENT_FORMAT: u32 = 2;
#[derive(Clone, Copy)]
pub struct FileLimits {
    pub maximum_file_bytes: u64,
    pub maximum_manifest_bytes: u64,
    pub maximum_source_bytes: u64,
    pub maximum_payload_bytes: u64,
    pub maximum_payloads: usize,
}
impl Default for FileLimits {
    fn default() -> Self {
        Self {
            maximum_file_bytes: 256 * 1024 * 1024,
            maximum_manifest_bytes: 4 * 1024 * 1024,
            maximum_source_bytes: 16 * 1024 * 1024,
            maximum_payload_bytes: 64 * 1024 * 1024,
            maximum_payloads: 4096,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectDiagnostic {
    Migrated { from: u32, to: u32 },
    CacheNeedsValidation { cache: EntityId },
    CacheSourceChanged { cache: EntityId },
    CacheCorrupt { cache: EntityId },
}
#[derive(Clone, Debug)]
pub struct OpenedProject {
    pub project: ProjectFile,
    pub diagnostics: Vec<ProjectDiagnostic>,
    pub original_format: u32,
}
#[derive(Clone, Serialize, Deserialize)]
struct Chunk {
    length: u64,
    sha256: ContentHash,
}
impl Chunk {
    fn of(bytes: &[u8]) -> Self {
        Self {
            length: bytes.len() as u64,
            sha256: ContentHash::of(bytes),
        }
    }
}
#[derive(Serialize, Deserialize)]
struct PayloadRecord {
    descriptor: PayloadDescriptor,
    chunk: Chunk,
}
#[derive(Serialize, Deserialize)]
struct CacheRecord {
    descriptor: PayloadDescriptor,
    key: CacheKey,
    chunk: Chunk,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    identity: SourceIdentity,
    source: Chunk,
    required_capabilities: BTreeSet<String>,
    #[serde(default)]
    payloads: Vec<PayloadRecord>,
    #[serde(default)]
    caches: Vec<CacheRecord>,
    recovery: Option<RecoveryInfo>,
    #[serde(default, flatten)]
    metadata: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SourceIdentity {
    id: EntityId,
    name: String,
}
#[derive(Clone, Debug)]
pub struct ProjectSummary {
    pub document: EntityId,
    pub name: String,
    pub recovery: Option<RecoveryInfo>,
    pub format_version: u32,
    pub source_hash: ContentHash,
    pub expected_file_bytes: u64,
}
/// The supported first format stored canonical source in the manifest. Opening
/// it is a checked migration; subsequent explicit saves always write format 2.
#[derive(Deserialize)]
struct ManifestV1 {
    document: Document,
    #[serde(default)]
    required_capabilities: BTreeSet<String>,
    #[serde(default)]
    payloads: Vec<PayloadRecord>,
    #[serde(default)]
    caches: Vec<CacheRecord>,
    recovery: Option<RecoveryInfo>,
    #[serde(default, flatten)]
    metadata: BTreeMap<String, Value>,
}
fn fail(message: impl Into<String>) -> String {
    message.into()
}
fn bytes(reader: &mut impl Read, length: u64, limit: u64) -> Result<Vec<u8>, String> {
    if length > limit {
        return Err("Project entry exceeds its size limit".into());
    }
    let length = usize::try_from(length)
        .map_err(|_| fail("Project entry is too large for this platform"))?;
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("Project data is incomplete: {e}"))?;
    Ok(bytes)
}
fn checked_chunk(reader: &mut impl Read, chunk: &Chunk, limit: u64) -> Result<Vec<u8>, String> {
    let bytes = bytes(reader, chunk.length, limit)?;
    if ContentHash::of(&bytes) != chunk.sha256 {
        return Err("Project content checksum does not match".into());
    }
    Ok(bytes)
}
fn validate_budget(
    manifest: &Manifest,
    base: u64,
    limits: FileLimits,
    source_in_body: bool,
) -> Result<u64, String> {
    if manifest.payloads.len() + manifest.caches.len() > limits.maximum_payloads {
        return Err("Project payload count exceeds its limit".into());
    }
    let mut total = base;
    if manifest.source.length > limits.maximum_source_bytes {
        return Err("Canonical source exceeds its size limit".into());
    }
    if source_in_body {
        total = total
            .checked_add(manifest.source.length)
            .ok_or("Project size overflow")?;
    }
    for chunk in manifest
        .payloads
        .iter()
        .map(|p| &p.chunk)
        .chain(manifest.caches.iter().map(|c| &c.chunk))
    {
        if chunk.length > limits.maximum_payload_bytes {
            return Err("Project payload exceeds its size limit".into());
        }
        total = total
            .checked_add(chunk.length)
            .ok_or("Project size overflow")?;
    }
    if total > limits.maximum_file_bytes {
        return Err("Project exceeds its total size limit".into());
    }
    Ok(total)
}
pub fn write(
    project: &ProjectFile,
    writer: &mut impl Write,
    limits: FileLimits,
) -> Result<(), String> {
    project.validate()?;
    let source = serde_json::to_vec(&project.document).map_err(|e| e.to_string())?;
    let manifest = Manifest {
        identity: SourceIdentity {
            id: project.document.id(),
            name: project.document.name().into(),
        },
        source: Chunk::of(&source),
        required_capabilities: project.capabilities(),
        payloads: project
            .resources
            .payloads
            .values()
            .map(|p| PayloadRecord {
                descriptor: p.descriptor.clone(),
                chunk: Chunk::of(&p.bytes),
            })
            .collect(),
        caches: project
            .resources
            .caches
            .values()
            .map(|c| CacheRecord {
                descriptor: c.descriptor.clone(),
                key: c.key.clone(),
                chunk: Chunk::of(&c.bytes),
            })
            .collect(),
        recovery: project.recovery.clone(),
        metadata: project.resources.metadata.clone(),
    };
    let encoded = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    if encoded.len() as u64 > limits.maximum_manifest_bytes {
        return Err("Project manifest exceeds its size limit".into());
    }
    validate_budget(&manifest, HEADER_BYTES + encoded.len() as u64, limits, true)?;
    let io = |e: std::io::Error| e.to_string();
    writer.write_all(MAGIC).map_err(io)?;
    writer
        .write_all(&CURRENT_FORMAT.to_le_bytes())
        .map_err(io)?;
    writer
        .write_all(&(encoded.len() as u64).to_le_bytes())
        .map_err(io)?;
    writer.write_all(&ContentHash::of(&encoded).0).map_err(io)?;
    writer.write_all(&encoded).map_err(io)?;
    writer.write_all(&source).map_err(io)?;
    for payload in project.resources.payloads.values() {
        writer.write_all(&payload.bytes).map_err(io)?;
    }
    for cache in project.resources.caches.values() {
        writer.write_all(&cache.bytes).map_err(io)?;
    }
    Ok(())
}
struct Envelope {
    manifest: Manifest,
    inline_source: Option<Vec<u8>>,
    version: u32,
    base_bytes: u64,
}
fn envelope(
    reader: &mut impl Read,
    limits: FileLimits,
    supported: &BTreeSet<String>,
) -> Result<Envelope, String> {
    let header = bytes(reader, HEADER_BYTES, limits.maximum_file_bytes)?;
    if &header[..8] != MAGIC {
        return Err(
            "This is not a native ToskCAD project. Use Import source snapshot for legacy JSON."
                .into(),
        );
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("fixed header"));
    if !(1..=CURRENT_FORMAT).contains(&version) {
        return Err(format!(
            "Unsupported project format {version}; this build supports 1 through {CURRENT_FORMAT}"
        ));
    }
    let length = u64::from_le_bytes(header[12..20].try_into().expect("fixed header"));
    let manifest_bytes = bytes(
        reader,
        length,
        limits
            .maximum_manifest_bytes
            .min(limits.maximum_file_bytes.saturating_sub(HEADER_BYTES)),
    )?;
    if ContentHash::of(&manifest_bytes).0 != header[20..52] {
        return Err("Project manifest checksum does not match".into());
    }
    let (manifest, source) = if version == 1 {
        let old: ManifestV1 = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| format!("Invalid project manifest: {e}"))?;
        let source = serde_json::to_vec(&old.document).map_err(|e| e.to_string())?;
        (
            Manifest {
                identity: SourceIdentity {
                    id: old.document.id(),
                    name: old.document.name().into(),
                },
                source: Chunk::of(&source),
                required_capabilities: old.required_capabilities,
                payloads: old.payloads,
                caches: old.caches,
                recovery: old.recovery,
                metadata: old.metadata,
            },
            Some(source),
        )
    } else {
        (
            serde_json::from_slice::<Manifest>(&manifest_bytes)
                .map_err(|e| format!("Invalid project manifest: {e}"))?,
            None,
        )
    };
    validate_budget(&manifest, HEADER_BYTES + length, limits, version != 1)?;
    if manifest.required_capabilities.len() > 128
        || manifest
            .required_capabilities
            .iter()
            .any(|c| c.trim().is_empty() || c.len() > 256)
        || manifest.identity.id.is_nil()
        || manifest.identity.name.trim().is_empty()
        || manifest.identity.name.len() > 1024
    {
        return Err("Invalid project identity or capability metadata".into());
    }
    let unsupported: Vec<_> = manifest
        .required_capabilities
        .difference(supported)
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        return Err(format!(
            "Project requires unsupported core capabilities: {}",
            unsupported.join(", ")
        ));
    }
    Ok(Envelope {
        manifest,
        inline_source: source,
        version,
        base_bytes: HEADER_BYTES + length,
    })
}
/// A checked manifest summary does not establish payload integrity. Full `read`
/// validation is required before a recovery snapshot can replace visible source.
pub fn summary(
    reader: &mut impl Read,
    limits: FileLimits,
    supported: &BTreeSet<String>,
) -> Result<ProjectSummary, String> {
    let Envelope {
        manifest,
        version,
        base_bytes,
        ..
    } = envelope(reader, limits, supported)?;
    let expected_file_bytes = validate_budget(&manifest, base_bytes, limits, version != 1)?;
    Ok(ProjectSummary {
        document: manifest.identity.id,
        name: manifest.identity.name,
        recovery: manifest.recovery,
        format_version: version,
        source_hash: manifest.source.sha256,
        expected_file_bytes,
    })
}
pub fn read(
    reader: &mut impl Read,
    limits: FileLimits,
    supported: &BTreeSet<String>,
) -> Result<OpenedProject, String> {
    let Envelope {
        manifest,
        inline_source: source,
        version,
        ..
    } = envelope(reader, limits, supported)?;
    let source = match source {
        Some(bytes) => bytes,
        None => checked_chunk(reader, &manifest.source, limits.maximum_source_bytes)?,
    };
    let document: Document =
        serde_json::from_slice(&source).map_err(|e| format!("Invalid canonical document: {e}"))?;
    let mut project = ProjectFile::new(document)?;
    if project.document.id() != manifest.identity.id
        || project.document.name() != manifest.identity.name
    {
        return Err("Project source identity does not match its checked manifest".into());
    }
    project.resources.required_capabilities = manifest.required_capabilities;
    if !project.capabilities().is_subset(supported) {
        return Err("Project source requires a core capability unavailable in this build".into());
    }
    project.resources.metadata = manifest.metadata;
    project.recovery = manifest.recovery;
    let mut ids = BTreeSet::new();
    for record in manifest.payloads {
        let id = record.descriptor.id;
        if !ids.insert(id) {
            return Err("Duplicate project payload identity".into());
        }
        let payload = Payload {
            descriptor: record.descriptor,
            bytes: Arc::from(checked_chunk(
                reader,
                &record.chunk,
                limits.maximum_payload_bytes,
            )?),
        };
        payload.validate()?;
        project.resources.payloads.insert(id, payload);
    }
    let mut diagnostics = Vec::new();
    if version != CURRENT_FORMAT {
        diagnostics.push(ProjectDiagnostic::Migrated {
            from: version,
            to: CURRENT_FORMAT,
        });
    }
    let current_source = ContentHash::document(&project.document)?;
    for record in manifest.caches {
        let id = record.descriptor.id;
        if !ids.insert(id) {
            return Err("Duplicate project cache identity".into());
        }
        let raw = bytes(reader, record.chunk.length, limits.maximum_payload_bytes)?;
        if ContentHash::of(&raw) != record.chunk.sha256 {
            diagnostics.push(ProjectDiagnostic::CacheCorrupt { cache: id });
            continue;
        }
        let cache = DerivedCache {
            descriptor: record.descriptor,
            key: record.key,
            bytes: Arc::from(raw),
        };
        if cache.validate().is_err() {
            diagnostics.push(ProjectDiagnostic::CacheCorrupt { cache: id });
            continue;
        }
        diagnostics.push(if cache.key.source == current_source {
            ProjectDiagnostic::CacheNeedsValidation { cache: id }
        } else {
            ProjectDiagnostic::CacheSourceChanged { cache: id }
        });
        project.resources.caches.insert(id, cache);
    }
    let mut trailing = [0];
    if reader.read(&mut trailing).map_err(|e| e.to_string())? != 0 {
        return Err("Project has unexpected trailing data".into());
    }
    project.validate()?;
    Ok(OpenedProject {
        project,
        diagnostics,
        original_format: version,
    })
}
