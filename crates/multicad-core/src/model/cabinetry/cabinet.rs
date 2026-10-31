//! Explicit cabinet shelf identities, stocks, floor datum and connection
//! settings. These remain source data; panel and machining evaluators produce
//! the geometry from the same definition used by UI and MCP.
use super::*;
use crate::model::length;

pub const CABINET_CAPABILITY: &str = "cabinetry.cabinet.1";
pub const DOWEL_CAPABILITY: &str = "cabinetry.dowel.1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShelfSupport {
    Fixed,
    System32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CabinetShelf {
    /// Stable slot, independent of row order. Removing a different shelf must
    /// not repoint this panel's downstream features to its neighbor.
    pub key: u32,
    /// Local floor to the shelf underside, including toe clearance.
    pub underside: Dimension,
    /// Finished front-to-back size; never silently shortened to fit a back.
    pub depth: Dimension,
    /// None inherits the cabinet's default shelf stock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock: Option<EntityId>,
    pub support: ShelfSupport,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DowelSpec {
    pub diameter: Dimension,
    pub count: u32,
    /// Distance along the mating edge to the first and last hole centers.
    pub inset: Dimension,
    /// Offset from the middle of the edge panel's thickness.
    pub offset: Dimension,
    pub face_depth: Dimension,
    pub edge_depth: Dimension,
    /// Total diametral allowance of each receiving hole.
    pub fit: Dimension,
}
impl Default for DowelSpec {
    fn default() -> Self {
        Self {
            diameter: length(8.),
            count: 2,
            inset: length(50.),
            offset: length(0.),
            face_depth: length(12.),
            edge_depth: length(20.),
            fit: length(0.2),
        }
    }
}
impl DowelSpec {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        vec![
            &self.diameter,
            &self.inset,
            &self.offset,
            &self.face_depth,
            &self.edge_depth,
            &self.fit,
        ]
    }
    pub(super) fn validate(&self) -> Result<(), DocumentError> {
        if !(1..=MAX_TENONS).contains(&self.count) {
            return Err(invalid(format!(
                "A dowel connection needs 1–{MAX_TENONS} dowels per pair"
            )));
        }
        for dimension in self.dimensions() {
            check_dimension(dimension, Unit::Millimeter)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "settings",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CabinetJoinery {
    Dowels(DowelSpec),
    Domino(LooseTenonSpec),
}
impl Default for CabinetJoinery {
    fn default() -> Self {
        Self::Dowels(DowelSpec::default())
    }
}
impl CabinetJoinery {
    fn dimensions(&self) -> Vec<&Dimension> {
        match self {
            Self::Dowels(s) => s.dimensions(),
            Self::Domino(s) => s.dimensions(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CabinetPins {
    pub diameter: Dimension,
    pub depth: Dimension,
    /// Hole center above the interior bottom (top of the bottom panel).
    pub start: Dimension,
    /// Clearance from the top panel's underside to the last hole center.
    pub end_clearance: Dimension,
    /// Row centers measured from each shelf's front and rear edges.
    pub front_inset: Dimension,
    pub rear_inset: Dimension,
}
impl Default for CabinetPins {
    fn default() -> Self {
        Self {
            diameter: length(5.),
            depth: length(12.),
            start: length(37.),
            end_clearance: length(37.),
            front_inset: length(37.),
            rear_inset: length(37.),
        }
    }
}
impl CabinetPins {
    fn dimensions(&self) -> Vec<&Dimension> {
        vec![
            &self.diameter,
            &self.depth,
            &self.start,
            &self.end_clearance,
            &self.front_inset,
            &self.rear_inset,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CabinetOptions {
    /// Floor to the bottom panel's underside; no front recess is implied.
    pub toe_clearance: Dimension,
    pub shelf_stock: EntityId,
    pub shelves: Vec<CabinetShelf>,
    pub joinery: CabinetJoinery,
    pub pins: CabinetPins,
}
impl CabinetOptions {
    pub(super) fn validate(&self) -> Result<(), DocumentError> {
        if self.shelf_stock.is_nil() || self.shelves.len() > MAX_SHELVES {
            return Err(invalid(
                "A cabinet needs default shelf stock and at most 64 shelves",
            ));
        }
        let mut keys = BTreeSet::new();
        for shelf in &self.shelves {
            if shelf.key as usize >= MAX_SHELVES || !keys.insert(shelf.key) {
                return Err(invalid(
                    "Cabinet shelf keys must be distinct stable slots from 0 to 63",
                ));
            }
            if shelf.stock.is_some_and(|s| s.is_nil()) {
                return Err(invalid("Shelf stock identity is nil"));
            }
        }
        match &self.joinery {
            CabinetJoinery::Dowels(s) => s.validate()?,
            CabinetJoinery::Domino(s) => s.validate()?,
        }
        for dimension in self.dimensions() {
            check_dimension(dimension, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        let mut out = vec![&self.toe_clearance];
        for shelf in &self.shelves {
            out.extend([&shelf.underside, &shelf.depth]);
        }
        out.extend(self.joinery.dimensions());
        out.extend(self.pins.dimensions());
        out
    }
    pub fn shelf(&self, key: u32) -> Option<&CabinetShelf> {
        self.shelves.iter().find(|s| s.key == key)
    }
}
