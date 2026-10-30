//! Planar model faces as Extrude/Revolve profiles (CAD-146).
//!
//! A face profile stores only the identity of its canonical persistent face
//! reference. The selector itself (a [`GeometryIntent`] with a planar
//! attachment) lives once on `Feature.inputs` under [`PROFILE_FACE_ROLE`], so
//! the shared reference inspection reports it and explicit repair replaces it
//! without leaving a duplicated payload stale. Native evaluation reads the
//! exact current face boundary (outer loop and holes); nothing here samples or
//! guesses geometry.
use super::{FaceInput, ProfileSource, source::invalid, source::validate_region};
use crate::{
    document::{DocumentError, EntityData, EntityId, Link},
    references::{
        AnalyticGeometry, Attachment, GeometryIntent, Origin, TopologyKey, TopologyKind,
        TopologySnapshot,
    },
};
use serde::{Deserialize, Serialize};

/// Documents containing a face-profile Extrude/Revolve require this reader
/// capability; older readers refuse them instead of misreading the profile.
pub const FACE_PROFILE_CAPABILITY: &str = "model.face_profile.1";
/// Canonical `Feature.inputs` role of a profile face reference.
pub const PROFILE_FACE_ROLE: &str = "profile/face";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceProfile {
    /// Identity of the canonical face intent stored on `Feature.inputs`.
    pub face: EntityId,
}

/// Extrude/Revolve profile source. The sketch form serializes exactly as the
/// historical `ProfileSource { sketch, region }`, so sketch documents are
/// unchanged; the face form is `{ "face": <reference id> }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelProfile {
    Sketch(ProfileSource),
    Face(FaceProfile),
}
impl From<ProfileSource> for ModelProfile {
    fn from(source: ProfileSource) -> Self {
        Self::Sketch(source)
    }
}
impl ModelProfile {
    pub fn sketch(&self) -> Option<&ProfileSource> {
        match self {
            Self::Sketch(source) => Some(source),
            Self::Face(_) => None,
        }
    }
    /// The profile face reference identity, if this is a face profile.
    pub fn face(&self) -> Option<EntityId> {
        match self {
            Self::Sketch(_) => None,
            Self::Face(face) => Some(face.face),
        }
    }
    pub fn sketch_id(&self) -> Option<EntityId> {
        self.sketch().map(|source| source.sketch)
    }
    pub(super) fn validate(&self) -> Result<(), DocumentError> {
        match self {
            Self::Sketch(source) => {
                if source.sketch.is_nil() {
                    return Err(invalid("Profile sketch identity is nil"));
                }
                validate_region(&source.region)
            }
            Self::Face(face) if face.face.is_nil() => {
                Err(invalid("Profile face reference identity is nil"))
            }
            Self::Face(_) => Ok(()),
        }
    }
    /// Plain (non-geometry) links of this profile.
    pub(super) fn plain_links(&self) -> Vec<Link> {
        self.sketch_id()
            .map(|sketch| Link::new(sketch, "profile"))
            .into_iter()
            .collect()
    }
    /// Validate the profile face among `faces` and return its canonical link.
    /// Termination faces are matched separately by their own identities.
    pub(super) fn face_link(&self, faces: &[FaceInput]) -> Result<Option<Link>, DocumentError> {
        let Some(id) = self.face() else {
            return Ok(None);
        };
        let mut matches = faces.iter().filter(|face| face.intent.id == id);
        let (Some(face), None) = (matches.next(), matches.next()) else {
            return Err(invalid(
                "A face profile requires exactly one matching face input",
            ));
        };
        validate_profile_face(face)?;
        Ok(Some(Link {
            target: face.source,
            role: PROFILE_FACE_ROLE.into(),
            geometry: Some(face.intent.clone()),
        }))
    }
}

/// Structural checks shared by preparation and reading. The planar
/// attachment supplies the stable profile U,V frame used by typed directions
/// and revolution axes; current planarity is proven natively.
pub fn validate_profile_face(face: &FaceInput) -> Result<(), DocumentError> {
    face.intent.validate().map_err(invalid)?;
    if face.source.is_nil() || face.intent.kind != TopologyKind::Face {
        return Err(invalid(
            "A profile face input must reference a face of a modeling feature",
        ));
    }
    if face.intent.attachment.is_none() {
        return Err(invalid(
            "A profile face needs a planar source frame; choose a named planar face",
        ));
    }
    Ok(())
}

/// Capture a selected planar face as a profile input. This refuses curved,
/// unnamed, ambiguous or multi-frame faces with an actionable message and
/// never infers a face by proximity. `anchor` chooses among several source
/// frames; with none, exactly one frame must exist.
pub fn capture_profile_face(
    snapshot: &TopologySnapshot,
    key: TopologyKey,
    anchor: Option<&Origin>,
) -> Result<FaceInput, String> {
    if key.kind != TopologyKind::Face {
        return Err("Select a planar model face".into());
    }
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key == key)
        .ok_or("The selected face is no longer part of the current model")?;
    if element
        .geometry
        .is_some_and(|g| !matches!(g, AnalyticGeometry::Plane { .. }))
    {
        return Err("This face is curved; Extrude and Revolve need a planar face".into());
    }
    if element.frames.is_empty() {
        return Err(
            "This face has no planar reference frame; choose a named planar face or sketch on it"
                .into(),
        );
    }
    let frame = match anchor {
        Some(anchor) => element.frames.iter().find(|f| f.origin == *anchor),
        None if element.frames.len() == 1 => element.frames.first(),
        None => {
            return Err(
                "This face has several source frames and is ambiguous; sketch on it or choose another face"
                    .into(),
            );
        }
    }
    .ok_or("The chosen planar source frame is unavailable on this face")?;
    let mut intent = GeometryIntent::capture(snapshot, key)
        .map_err(|_| "This face has no unique persistent source; choose another face")?;
    intent.attachment = Some(Attachment {
        anchor: frame.origin.clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let face = FaceInput {
        source: snapshot.owner,
        intent,
    };
    validate_profile_face(&face).map_err(|e| e.to_string())?;
    Ok(face)
}

/// Whether a stored Extrude/Revolve definition uses a face profile, read
/// without parsing so capability collection never fails on other errors.
pub(super) fn definition_uses_face_profile(data: &serde_json::Value) -> bool {
    data.get("profile")
        .and_then(|profile| profile.get("face"))
        .is_some()
}

/// The canonical profile-face producer must be an active Feature.
pub(super) fn check_profile_face_producer(
    document: &crate::document::Document,
    link: &Link,
) -> Result<(), String> {
    if matches!(
        document.get(link.target).map(|e| &e.data),
        Some(EntityData::Feature {
            suppressed: false,
            ..
        })
    ) {
        Ok(())
    } else {
        Err("Choose a face of an existing, unsuppressed modeling feature".into())
    }
}

/// The link-owned profile face input of an Extrude/Revolve feature, if any.
pub fn stored_profile_face(
    document: &crate::document::Document,
    feature: EntityId,
) -> Option<FaceInput> {
    let Some(EntityData::Feature { inputs, .. }) = document.get(feature).map(|e| &e.data) else {
        return None;
    };
    inputs
        .iter()
        .find(|link| link.role == PROFILE_FACE_ROLE)
        .and_then(|link| {
            link.geometry.as_ref().map(|intent| FaceInput {
                source: link.target,
                intent: intent.clone(),
            })
        })
}

/// Region placeholder of a face output: face outputs carry no sketch region.
pub fn empty_region() -> crate::sketch_regions::RegionIntent {
    crate::sketch_regions::RegionIntent {
        outer: crate::sketch_regions::BoundaryIntent {
            curves: Default::default(),
        },
        holes: Default::default(),
    }
}

/// Source-only checks of one face output in an Extrude/Revolve request. The
/// region must be empty, the face structurally valid, owned by an active
/// Feature that is not produced by this request, and selected only once.
pub(super) fn check_face_output(
    document: &crate::document::Document,
    face: &FaceInput,
    region: &crate::sketch_regions::RegionIntent,
    outputs: &[EntityId],
    seen: &mut Vec<FaceInput>,
) -> Result<(), crate::commands::CommandError> {
    let error = |message: &str| super::common::error(message);
    if !region.outer.curves.is_empty() || !region.holes.is_empty() {
        return Err(error(
            "A face output cannot also name sketch profile curves",
        ));
    }
    validate_profile_face(face).map_err(|e| error(&e.to_string()))?;
    if outputs.contains(&face.source) {
        return Err(error("A profile face cannot come from its own output"));
    }
    let link = Link::new(face.source, PROFILE_FACE_ROLE);
    check_profile_face_producer(document, &link).map_err(|e| error(&e))?;
    if seen.iter().any(|other| {
        other.intent.id == face.intent.id
            || (other.source == face.source
                && other.intent.origins == face.intent.origins
                && other.intent.boundary == face.intent.boundary)
    }) {
        return Err(error("Select each profile face once"));
    }
    seen.push(face.clone());
    Ok(())
}
