//! Receiving wall details share the canonical wall/opening providers. These
//! request values are never copied into an architectural ownership record.
use super::*;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureOpeningTypeRequest {
    pub id: EntityId,
    pub name: String,
    pub value: model::OpeningTypeDefinition,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureWallDetails {
    #[serde(default)]
    pub openings: Option<Vec<model::Opening>>,
    #[serde(default)]
    pub cutouts: Option<Vec<model::VerticalCutout>>,
    #[serde(default)]
    pub junctions: Option<Vec<model::WallJunction>>,
    #[serde(default)]
    pub new_opening_types: Vec<ArchitectureOpeningTypeRequest>,
}
pub(super) fn apply(wall: &mut model::WallDefinition, details: &Option<ArchitectureWallDetails>) {
    if let Some(details) = details {
        if let Some(openings) = &details.openings {
            wall.openings = openings.clone();
        }
        if let Some(cutouts) = &details.cutouts {
            wall.cutouts = cutouts.clone();
        }
        if let Some(junctions) = &details.junctions {
            wall.layer_placement
                .get_or_insert_with(Default::default)
                .junctions = junctions.clone();
        }
    }
}
pub(super) fn mutations(
    document: &Document,
    context: &ArchitectureContext,
) -> Result<Vec<Mutation>, CommandError> {
    let ArchitectureContext::Wall {
        details: Some(details),
        ..
    } = context
    else {
        return Ok(Vec::new());
    };
    if details.new_opening_types.len() > model::MAX_WALL_OPENINGS {
        return Err(invalid("Receiving wall details exceed the opening type budget").into());
    }
    let mut seen = BTreeSet::new();
    details
        .new_opening_types
        .iter()
        .map(|value| {
            if value.id.is_nil()
                || value.id == document.id()
                || document.get(value.id).is_some()
                || !seen.insert(value.id)
            {
                return Err(invalid("New opening types require distinct unused identities").into());
            }
            let request = model::NewType {
                id: value.id,
                name: value.name.clone(),
                kind: model::NewTypeKind::Opening(value.value.clone()),
            };
            Ok(Mutation::Insert(request.entity()?))
        })
        .collect()
}
