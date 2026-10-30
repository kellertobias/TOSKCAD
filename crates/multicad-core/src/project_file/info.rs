//! Authored file identity: name, description and preview image.
//!
//! The name is the canonical document name (also stored in the checked
//! manifest identity). Description and preview live in one optional
//! `core.file_info` extension entity of the canonical source, so they share
//! the typed transaction API, Undo/Redo, dirty state and save path with every
//! other authored edit. The information is optional and requires no core
//! capability: an older reader opens the project normally and preserves the
//! extension entity unchanged.
use super::ContentHash;
use crate::document::{Definition, Document, DocumentError, Entity, EntityData, Mutation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const FILE_INFO_TYPE: &str = "core.file_info";
pub const FILE_INFO_SCHEMA: u32 = 1;
pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_DESCRIPTION_CHARS: usize = 4000;
pub const MAX_PREVIEW_BYTES: usize = 1024 * 1024;
pub const MAX_PREVIEW_SIDE: u32 = 2048;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// A checked PNG preview. Only the signature and header dimensions are
/// verified here; pixel decoding belongs to the UI/storage boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewImage {
    width: u32,
    height: u32,
    png: Arc<[u8]>,
}
impl PreviewImage {
    pub fn from_png(bytes: impl Into<Arc<[u8]>>) -> Result<Self, String> {
        let png: Arc<[u8]> = bytes.into();
        if png.len() > MAX_PREVIEW_BYTES {
            return Err(format!(
                "The preview image exceeds {} KiB. Choose a smaller PNG.",
                MAX_PREVIEW_BYTES / 1024
            ));
        }
        // Signature, IHDR length (13) and type, width, height.
        if png.len() < 33 || &png[..8] != PNG_SIGNATURE || &png[8..16] != b"\0\0\0\rIHDR" {
            return Err("The preview image is not a PNG file. Choose a .png image.".into());
        }
        let width = u32::from_be_bytes(png[16..20].try_into().expect("checked length"));
        let height = u32::from_be_bytes(png[20..24].try_into().expect("checked length"));
        if !(1..=MAX_PREVIEW_SIDE).contains(&width) || !(1..=MAX_PREVIEW_SIDE).contains(&height) {
            return Err(format!(
                "The preview image is {width}×{height} px; use at most {MAX_PREVIEW_SIDE}×{MAX_PREVIEW_SIDE} px."
            ));
        }
        Ok(Self { width, height, png })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn png(&self) -> &[u8] {
        &self.png
    }
    pub fn content(&self) -> ContentHash {
        ContentHash::of(&self.png)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileInfo {
    pub name: String,
    pub description: String,
    pub preview: Option<PreviewImage>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPreview {
    media_type: String,
    width: u32,
    height: u32,
    data: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    #[serde(default)]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<StoredPreview>,
}

fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
pub fn validate_name(name: &str) -> Result<(), String> {
    let count = name.chars().count();
    if name.trim().is_empty() || count > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(format!(
            "Enter a file name of 1–{MAX_NAME_CHARS} characters without control characters"
        ));
    }
    Ok(())
}
pub fn validate_description(description: &str) -> Result<(), String> {
    if description.chars().count() > MAX_DESCRIPTION_CHARS
        || description
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(format!(
            "Use a description of at most {MAX_DESCRIPTION_CHARS} characters without control characters"
        ));
    }
    Ok(())
}
fn entities(document: &Document) -> impl Iterator<Item = (&Entity, &Definition)> {
    document.entities().filter_map(|entity| match &entity.data {
        EntityData::Extension { definition, .. } if definition.type_id == FILE_INFO_TYPE => {
            Some((entity, definition))
        }
        _ => None,
    })
}
fn decode(definition: &Definition) -> Result<(String, Option<PreviewImage>), String> {
    if definition.schema_version != FILE_INFO_SCHEMA {
        return Err(format!(
            "File information schema {} was written by a newer version and is read-only here",
            definition.schema_version
        ));
    }
    let stored: Stored = serde_json::from_value(definition.data.clone())
        .map_err(|e| format!("Invalid file information: {e}"))?;
    validate_description(&stored.description)?;
    let preview = match stored.preview {
        None => None,
        Some(p) => {
            if p.media_type != "image/png" {
                return Err("File preview must be a PNG image".into());
            }
            if p.data.len() > MAX_PREVIEW_BYTES.div_ceil(3) * 4 {
                return Err("File preview exceeds its size limit".into());
            }
            let image = PreviewImage::from_png(base64_decode(&p.data)?)?;
            if image.width != p.width || image.height != p.height {
                return Err("File preview dimensions do not match the image".into());
            }
            Some(image)
        }
    };
    Ok((stored.description, preview))
}
/// Structural check run by `Document::validate`. A newer schema is preserved
/// as opaque source; current-schema records must be well formed.
pub(crate) fn validate(document: &Document) -> Result<(), DocumentError> {
    let mut records = entities(document);
    let first = records.next();
    if records.next().is_some() {
        return Err(invalid("A project has at most one file information record"));
    }
    if let Some((entity, definition)) = first {
        if let EntityData::Extension { inputs, .. } = &entity.data
            && !inputs.is_empty()
        {
            return Err(invalid("File information has no dependencies"));
        }
        if definition.schema_version == FILE_INFO_SCHEMA {
            decode(definition).map_err(invalid)?;
        }
    }
    Ok(())
}
/// The file's authored information. Fails only for a record written by a
/// newer schema; a project without a record has an empty description.
pub fn read(document: &Document) -> Result<FileInfo, String> {
    let (description, preview) = match entities(document).next() {
        Some((_, definition)) => decode(definition)?,
        None => (String::new(), None),
    };
    Ok(FileInfo {
        name: document.name().into(),
        description,
        preview,
    })
}
/// Typed source edits that make the document's file information equal `info`.
/// Returns no edits when nothing changes.
pub fn edits(document: &Document, info: &FileInfo) -> Result<Vec<Mutation>, String> {
    validate_name(&info.name)?;
    validate_description(&info.description)?;
    let current = read(document)?;
    let mut edits = Vec::new();
    if current.name != info.name {
        edits.push(Mutation::RenameDocument(info.name.clone()));
    }
    if current.description == info.description && current.preview == info.preview {
        return Ok(edits);
    }
    let existing = entities(document).next().map(|(entity, _)| entity.clone());
    if info.description.is_empty() && info.preview.is_none() {
        if let Some(entity) = existing {
            edits.push(Mutation::Remove(entity.id));
        }
        return Ok(edits);
    }
    let stored = Stored {
        description: info.description.clone(),
        preview: info.preview.as_ref().map(|p| StoredPreview {
            media_type: "image/png".into(),
            width: p.width,
            height: p.height,
            data: base64_encode(&p.png),
        }),
    };
    let definition = Definition {
        type_id: FILE_INFO_TYPE.into(),
        schema_version: FILE_INFO_SCHEMA,
        data: serde_json::to_value(stored).map_err(|e| e.to_string())?,
    };
    let data = EntityData::Extension {
        definition,
        inputs: vec![],
    };
    edits.push(match existing {
        Some(mut entity) => {
            entity.data = data;
            Mutation::Replace(entity)
        }
        None => Mutation::Insert(Entity::new("File information", data)),
    });
    Ok(edits)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err("File preview data is not valid base64".into());
    }
    let value = |c: u8| -> Result<u32, String> {
        ALPHABET
            .iter()
            .position(|a| *a == c)
            .map(|p| p as u32)
            .ok_or_else(|| "File preview data is not valid base64".to_string())
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunks = bytes.len() / 4;
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let padding = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if padding > 2 || (padding > 0 && index + 1 != chunks) {
            return Err("File preview data is not valid base64".into());
        }
        let mut n = 0;
        for (i, c) in chunk.iter().enumerate() {
            let sextet = if i >= 4 - padding { 0 } else { value(*c)? };
            n |= sextet << (18 - 6 * i);
        }
        let decoded = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&decoded[..3 - padding]);
    }
    // Canonical form only, so one image has exactly one stored representation.
    if base64_encode(&out) != text {
        return Err("File preview data is not canonical base64".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Actor, Channel, Request, Session};

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(b"\0\0\0\rIHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }
    #[test]
    fn base64_round_trips_every_padding_length_and_rejects_noncanonical_text() {
        for length in 0..10 {
            let data: Vec<u8> = (0..length).map(|i| (i * 37 + 11) as u8).collect();
            let text = base64_encode(&data);
            assert_eq!(base64_decode(&text).unwrap(), data);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert!(base64_decode("TWE").is_err());
        assert!(base64_decode("TW==TWFu").is_err());
        assert!(base64_decode("TWF=").is_err(), "nonzero trailing bits");
        assert!(base64_decode("T*Fu").is_err());
    }
    #[test]
    fn previews_must_be_bounded_png_images() {
        let image = PreviewImage::from_png(png(320, 200)).unwrap();
        assert_eq!((image.width(), image.height()), (320, 200));
        assert!(PreviewImage::from_png(b"GIF89a...........................".to_vec()).is_err());
        assert!(PreviewImage::from_png(png(0, 10)).is_err());
        assert!(PreviewImage::from_png(png(4096, 10)).is_err());
        let mut large = png(10, 10);
        large.resize(MAX_PREVIEW_BYTES + 1, 0);
        assert!(PreviewImage::from_png(large).is_err());
    }
    #[test]
    fn info_edits_are_one_undoable_transaction_and_empty_info_removes_the_record() {
        let mut session = Session::new(Document::new("Bracket").unwrap()).unwrap();
        assert_eq!(
            read(session.document()).unwrap(),
            FileInfo {
                name: "Bracket".into(),
                ..Default::default()
            }
        );
        let info = FileInfo {
            name: "Wall bracket".into(),
            description: "Holds a 20 mm shelf.\nPrint flat.".into(),
            preview: Some(PreviewImage::from_png(png(64, 48)).unwrap()),
        };
        let edits = edits(session.document(), &info).unwrap();
        session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Ui),
                label: "Edit file information".into(),
                edits,
            })
            .unwrap();
        assert_eq!(read(session.document()).unwrap(), info);
        assert!(super::edits(session.document(), &info).unwrap().is_empty());
        let json = serde_json::to_vec(session.document()).unwrap();
        let reparsed: Document = serde_json::from_slice(&json).unwrap();
        assert_eq!(read(&reparsed).unwrap(), info);
        let cleared = FileInfo {
            name: "Wall bracket".into(),
            ..Default::default()
        };
        let removal = super::edits(session.document(), &cleared).unwrap();
        assert!(matches!(removal.as_slice(), [Mutation::Remove(_)]));
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(read(session.document()).unwrap().name, "Bracket");
        assert!(read(session.document()).unwrap().preview.is_none());
    }
    #[test]
    fn invalid_names_descriptions_and_duplicate_records_are_rejected() {
        let document = Document::new("Part").unwrap();
        let mut info = read(&document).unwrap();
        info.name = " ".into();
        assert!(edits(&document, &info).is_err());
        info.name = "Part".into();
        info.description = "bad\u{7}".into();
        assert!(edits(&document, &info).is_err());
        info.description = "Fine".into();
        let edits = edits(&document, &info).unwrap();
        let document = document.edited(&edits).unwrap();
        let Mutation::Insert(mut duplicate) = edits[0].clone() else {
            panic!("expected insert")
        };
        duplicate.id = crate::document::EntityId::new();
        assert!(document.edited(&[Mutation::Insert(duplicate)]).is_err());
    }
    #[test]
    fn a_newer_schema_record_is_preserved_but_read_only() {
        let document = Document::new("Future").unwrap();
        let future = Entity::new(
            "File information",
            EntityData::Extension {
                definition: Definition {
                    type_id: FILE_INFO_TYPE.into(),
                    schema_version: 2,
                    data: serde_json::json!({"title":"something new"}),
                },
                inputs: vec![],
            },
        );
        let document = document.edited(&[Mutation::Insert(future)]).unwrap();
        assert!(read(&document).is_err());
        assert!(edits(&document, &FileInfo::default()).is_err());
    }
    #[test]
    fn info_round_trips_through_the_native_file_and_card_without_new_capabilities() {
        use crate::project_file::{self, FileLimits, ProjectFile};
        let plain = Document::new("Shelf").unwrap();
        let info = FileInfo {
            name: "Shelf bracket".into(),
            description: "Two screws".into(),
            preview: Some(PreviewImage::from_png(png(8, 8)).unwrap()),
        };
        let edited = plain.edited(&edits(&plain, &info).unwrap()).unwrap();
        let without = ProjectFile::new(plain).unwrap();
        let with = ProjectFile::new(edited).unwrap();
        assert_eq!(
            without.capabilities(),
            with.capabilities(),
            "optional file information must not lock out older readers"
        );
        let mut bytes = Vec::new();
        project_file::write(&with, &mut bytes, FileLimits::default()).unwrap();
        let supported = project_file::supported_capabilities();
        let opened =
            project_file::read(&mut bytes.as_slice(), FileLimits::default(), &supported).unwrap();
        assert_eq!(read(&opened.project.document).unwrap(), info);
        let card =
            project_file::card(&mut bytes.as_slice(), FileLimits::default(), &supported).unwrap();
        assert_eq!(card.info, info);
        assert!(card.info_error.is_none());
        assert_eq!(card.document, with.document.id());
        assert_eq!(card.symbol_definitions, 0);
        let summary =
            project_file::summary(&mut bytes.as_slice(), FileLimits::default(), &supported)
                .unwrap();
        assert_eq!(summary.name, "Shelf bracket");
        let mut corrupt = bytes.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        assert!(
            project_file::card(&mut corrupt.as_slice(), FileLimits::default(), &supported).is_err(),
            "the card checks the canonical source checksum"
        );
    }
}
