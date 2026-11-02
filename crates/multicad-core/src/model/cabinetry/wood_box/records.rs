use super::super::*;
use crate::{
    document::Mutation,
    model::{angle, length},
};
use std::collections::BTreeMap;

pub const WOOD_BOX_KIND: &str = "cabinetry.wood_box";
pub const WOOD_BOX_CAPABILITY: &str = "cabinetry.wood_box.1";
pub const RECIPE_PANEL_SCHEMA: u32 = 2;
pub const RECIPE_PANEL_CAPABILITY: &str = "cabinetry.panel.2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WoodBoxFace {
    Left,
    Right,
    Front,
    Back,
    Bottom,
    Top,
}
impl WoodBoxFace {
    pub const ALL: [Self; 6] = [
        Self::Left,
        Self::Right,
        Self::Front,
        Self::Back,
        Self::Bottom,
        Self::Top,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Front => "Front",
            Self::Back => "Back",
            Self::Bottom => "Bottom",
            Self::Top => "Top",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WoodBoxPanelIds {
    pub feature: EntityId,
    pub body: EntityId,
}
impl Default for WoodBoxPanelIds {
    fn default() -> Self {
        Self {
            feature: EntityId::new(),
            body: EntityId::new(),
        }
    }
}
/// Face identities remain reserved while disabled. Presence controls entities,
/// rather than discarding the identity when a checkbox is temporarily off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WoodBoxDefinition {
    pub stock: EntityId,
    pub origin: [Dimension; 3],
    pub rotation: Dimension,
    pub width: Dimension,
    pub depth: Dimension,
    pub height: Dimension,
    pub faces: BTreeSet<WoodBoxFace>,
    pub panels: BTreeMap<WoodBoxFace, WoodBoxPanelIds>,
    pub group: EntityId,
}
impl WoodBoxDefinition {
    pub fn new(stock: EntityId, width: Dimension, depth: Dimension, height: Dimension) -> Self {
        Self {
            stock,
            origin: [length(0.), length(0.), length(0.)],
            rotation: angle(0.),
            width,
            depth,
            height,
            faces: WoodBoxFace::ALL.into_iter().collect(),
            panels: WoodBoxFace::ALL
                .into_iter()
                .map(|face| (face, WoodBoxPanelIds::default()))
                .collect(),
            group: EntityId::new(),
        }
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.stock.is_nil() || self.group.is_nil() || self.stock == self.group {
            return Err(invalid(
                "Wood Box needs explicit panel stock and a distinct group identity",
            ));
        }
        if self.faces.is_empty() {
            return Err(invalid("A Wood Box must contain at least one face"));
        }
        if self.panels.keys().copied().collect::<BTreeSet<_>>()
            != WoodBoxFace::ALL.into_iter().collect()
        {
            return Err(invalid(
                "Wood Box must retain exactly six named face identities",
            ));
        }
        let mut ids = BTreeSet::from([self.group, self.stock]);
        for panel in self.panels.values() {
            for id in [panel.body, panel.feature] {
                if id.is_nil() || !ids.insert(id) {
                    return Err(invalid(
                        "Wood Box face identities must be non-nil and distinct from stock, group and other faces",
                    ));
                }
            }
        }
        for dimension in self
            .origin
            .iter()
            .chain([&self.width, &self.depth, &self.height])
        {
            check_dimension(dimension, Unit::Millimeter)?;
        }
        check_dimension(&self.rotation, Unit::Degree)?;
        Ok(())
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(
            self.origin
                .iter()
                .chain([&self.width, &self.depth, &self.height, &self.rotation])
                .collect(),
        );
        links.push(Link::new(self.stock, "stock"));
        links
    }
    pub fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(WOOD_BOX_KIND, self)?,
            inputs: self.inputs(),
        })
    }
}
pub fn wood_boxes(document: &Document) -> impl Iterator<Item = &Entity> {
    document
        .entities()
        .filter(|entity| is_kind(entity, WOOD_BOX_KIND))
}
pub fn read_wood_box_definition(
    document: &Document,
    id: EntityId,
) -> Result<WoodBoxDefinition, DocumentError> {
    let source: WoodBoxDefinition = parse(&extension(document, id, WOOD_BOX_KIND)?, WOOD_BOX_KIND)?;
    source.validate()?;
    let EntityData::Extension { inputs, .. } =
        &document.get(id).ok_or(DocumentError::Missing(id))?.data
    else {
        unreachable!()
    };
    let expected = source.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Wood Box source and dependency links disagree; repair its recipe",
        ));
    }
    if source.group == id
        || source.stock == id
        || source
            .panels
            .values()
            .any(|panel| panel.body == id || panel.feature == id)
    {
        return Err(invalid(
            "Wood Box recipe identity collides with a face, group or stock",
        ));
    }
    Ok(source)
}
pub fn read_wood_box(
    document: &Document,
    id: EntityId,
) -> Result<WoodBoxDefinition, DocumentError> {
    let source = read_wood_box_definition(document, id)?;
    if !matches!(
        document.get(source.group).map(|entity| &entity.data),
        Some(EntityData::Group {})
    ) {
        return Err(invalid("Wood Box group is missing; repair its ownership"));
    }
    for (face, ids) in &source.panels {
        if !source.faces.contains(face) {
            if document.get(ids.feature).is_some() || document.get(ids.body).is_some() {
                return Err(invalid(
                    "An omitted Wood Box face still has entities; repair the recipe",
                ));
            }
            continue;
        }
        if !matches!(
            document.get(ids.feature).map(|entity| &entity.data),
            Some(EntityData::Feature {
                suppressed: false,
                ..
            })
        ) {
            return Err(invalid(format!(
                "Wood Box {} panel is missing or suppressed; repair it",
                face.label()
            )));
        }
        let panel = read_panel(document, ids.feature)?;
        if panel.output.body != ids.body
            || !matches!(panel.source,PanelSource::WoodBox {recipe,face:member}if recipe==id&&member==*face)
        {
            return Err(invalid(
                "Generated Wood Box panel has conflicting source ownership; repair it",
            ));
        }
        let Some(EntityData::Part { features, .. }) =
            document.get(ids.body).map(|entity| &entity.data)
        else {
            return Err(invalid(
                "Wood Box receiving panel Part is missing; repair it",
            ));
        };
        if features.first() != Some(&ids.feature) {
            return Err(invalid(
                "Wood Box panel source must remain its first producer; repair the Part history",
            ));
        }
        if document.entities().any(|entity|entity.id!=ids.body&&matches!(&entity.data,EntityData::Part {features,..}if features.contains(&ids.feature))) {return Err(invalid("Wood Box panel belongs to another Part; repair its ownership"));}
    }
    for entity in panels(document) {
        if let PanelSource::WoodBox { recipe, face } = read_panel(document, entity.id)?.source
            && recipe == id
            && (!source.faces.contains(&face) || source.panels[&face].feature != entity.id)
        {
            return Err(invalid(
                "Wood Box has an unregistered generated panel; repair its recipe",
            ));
        }
    }
    Ok(source)
}
pub fn wood_box_owner(document: &Document, id: EntityId) -> Option<EntityId> {
    if is_kind(document.get(id)?, WOOD_BOX_KIND) {
        return read_wood_box(document, id).ok().map(|_| id);
    }
    if let Ok(panel) = read_panel(document, id)
        && let PanelSource::WoodBox { recipe, .. } = panel.source
    {
        return read_wood_box(document, recipe).ok().map(|_| recipe);
    }
    if let Some(feature) = panel_feature(document, id)
        && let Ok(panel) = read_panel(document, feature)
        && let PanelSource::WoodBox { recipe, .. } = panel.source
    {
        return read_wood_box(document, recipe).ok().map(|_| recipe);
    }
    wood_boxes(document)
        .find(|entity| {
            read_wood_box_definition(document, entity.id).is_ok_and(|source| source.group == id)
        })
        .map(|entity| entity.id)
}

pub(super) fn removed_ids(edits: &[Mutation]) -> BTreeSet<EntityId> {
    edits
        .iter()
        .filter_map(|edit| {
            if let Mutation::Remove(id) = edit {
                Some(*id)
            } else {
                None
            }
        })
        .collect()
}
