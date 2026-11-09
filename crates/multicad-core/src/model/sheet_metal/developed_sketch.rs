//! Persistent flat-pattern sketch chart. The sketch stays in developed XY
//! millimeters and depends only on the forming feature, never its final Part.
use super::{CutoutFace, read_sheet, sheet_feature, sheet_profile_head};
use crate::{
    constraint_sketch::source::Source,
    document::{Document, DocumentError, EntityData, EntityId, Link},
    model::source::invalid,
    sketch::Datum,
};

pub const DEVELOPED_SHEET_SKETCH_CAPABILITY: &str = "sheet_metal.developed_sketch.1";
const ROLE: &str = "sheet/developed/";
const BASE: &str = "base";
const FLANGE: &str = "flange/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevelopedSheetSketch {
    pub part: EntityId,
    pub forming: EntityId,
    pub face: CutoutFace,
}

pub(crate) fn is_developed_sheet_link(link: &Link) -> bool {
    link.role.starts_with(ROLE)
}

fn face_role(face: CutoutFace) -> String {
    match face {
        CutoutFace::Base => format!("{ROLE}{BASE}"),
        CutoutFace::Flange { index } => format!("{ROLE}{FLANGE}{index}"),
    }
}

fn parse_face_role(role: &str) -> Result<CutoutFace, DocumentError> {
    let malformed = || invalid("Invalid developed sheet sketch face link");
    let face = match role.strip_prefix(ROLE) {
        Some(BASE) => CutoutFace::Base,
        Some(rest) => CutoutFace::Flange {
            index: rest
                .strip_prefix(FLANGE)
                .and_then(|index| index.parse().ok())
                .ok_or_else(malformed)?,
        },
        None => return Err(malformed()),
    };
    if face_role(face) != role {
        return Err(malformed());
    }
    Ok(face)
}

fn chart(
    document: &Document,
    forming: EntityId,
    face: CutoutFace,
) -> Result<DevelopedSheetSketch, DocumentError> {
    let sheet = read_sheet(document, forming)?;
    let part = sheet.output.body;
    if sheet_feature(document, part) != Some(forming) {
        return Err(invalid(
            "The developed sketch must link to its Part's original sheet forming feature",
        ));
    }
    if matches!(face, CutoutFace::Flange { index } if index as usize >= sheet.flanges.len()) {
        return Err(invalid(
            "The developed sketch's selected flange no longer exists",
        ));
    }
    Ok(DevelopedSheetSketch {
        part,
        forming,
        face,
    })
}

pub(crate) fn developed_sheet_link(
    document: &Document,
    part: EntityId,
    face: CutoutFace,
) -> Result<Link, DocumentError> {
    let forming = sheet_feature(document, part)
        .filter(|_| sheet_profile_head(document, part).is_some())
        .ok_or_else(|| invalid("Select a sheet Part with a current developable producer"))?;
    chart(document, forming, face)?;
    Ok(Link::new(forming, face_role(face)))
}

/// Read a saved developed sheet chart. An ordinary sketch has no chart.
pub fn developed_sheet_sketch(
    document: &Document,
    sketch: EntityId,
) -> Result<Option<DevelopedSheetSketch>, DocumentError> {
    let Some(EntityData::Sketch {
        plane,
        definition,
        inputs,
    }) = document.get(sketch).map(|entity| &entity.data)
    else {
        return Ok(None);
    };
    let mut links = inputs.iter().filter(|link| is_developed_sheet_link(link));
    let Some(link) = links.next() else {
        return Ok(None);
    };
    if link.geometry.is_some() || links.next().is_some() {
        return Err(invalid(
            "A developed sheet sketch needs one non-geometric sheet link",
        ));
    }
    let xy = Datum::Xy.frame();
    if !matches!(
        document.get(*plane).map(|entity| &entity.data),
        Some(EntityData::Plane { origin_mm, normal, anchor: None })
            if *origin_mm == xy.origin && *normal == xy.n
    ) || Source::from_definition(definition)?.u != xy.u
    {
        return Err(invalid(
            "A developed sheet sketch must keep its unrotated XY plane",
        ));
    }
    chart(document, link.target, parse_face_role(&link.role)?).map(Some)
}
