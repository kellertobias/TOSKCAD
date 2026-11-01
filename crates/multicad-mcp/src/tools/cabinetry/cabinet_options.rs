//! Typed Cabinet input over the canonical cabinetry definition.
use super::*;

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SupportArgs {
    Fixed,
    /// Zero-based level on the 32 mm grid, measured from pins.start above the
    /// bottom panel top. The underside rests one pin radius above its hole.
    System32 {
        level: u32,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShelfArgs {
    /// Stable slot 0–63. Keep it when reordering or editing; omit a row to remove it.
    pub key: u32,
    /// Fixed shelves: local floor to underside. Adjustable shelves: optional
    /// retained fixed position for switching support later.
    #[serde(default)]
    pub underside: Option<Quantity>,
    /// Finished front-to-back depth, without silently trimming it at the back.
    pub depth: Quantity,
    /// Omit to inherit the default shelf stock.
    #[serde(default)]
    pub stock: Option<String>,
    pub support: SupportArgs,
}
#[derive(Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JoineryArg {
    #[default]
    Dowels,
    Domino,
}
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinArgs {
    #[serde(default)]
    pub diameter: Option<Quantity>,
    #[serde(default)]
    pub depth: Option<Quantity>,
    /// First hole center above the bottom panel's top (default 37 mm).
    #[serde(default)]
    pub start: Option<Quantity>,
    #[serde(default)]
    pub end_clearance: Option<Quantity>,
    #[serde(default)]
    pub front_inset: Option<Quantity>,
    #[serde(default)]
    pub rear_inset: Option<Quantity>,
}
impl PinArgs {
    fn spec(&self, d: &Document, e: &EvaluatedParameters) -> Result<model::CabinetPins, ToolError> {
        let or = |v: &Option<Quantity>, default: f64| {
            opt(v, d, e).map(|v| v.unwrap_or_else(|| length(default)))
        };
        Ok(model::CabinetPins {
            diameter: or(&self.diameter, 5.)?,
            depth: or(&self.depth, 12.)?,
            start: or(&self.start, 37.)?,
            end_clearance: or(&self.end_clearance, 37.)?,
            front_inset: or(&self.front_inset, 37.)?,
            rear_inset: or(&self.rear_inset, 37.)?,
        })
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CabinetArgs {
    /// Floor to bottom-panel underside; no front recess is implied.
    pub toe_clearance: Quantity,
    pub shelf_stock: String,
    pub shelves: Vec<ShelfArgs>,
    #[serde(default)]
    pub joinery: JoineryArg,
    #[serde(default)]
    pub dowels: DowelArgs,
    #[serde(default)]
    pub domino: TenonArgs,
    #[serde(default)]
    pub pins: PinArgs,
}
impl CabinetArgs {
    pub(super) fn definition(
        &self,
        d: &Document,
        e: &EvaluatedParameters,
        previous: Option<&model::CabinetOptions>,
    ) -> Result<model::CabinetOptions, ToolError> {
        if self.shelves.len() > model::MAX_SHELVES
            || self
                .shelves
                .iter()
                .any(|s| s.key as usize >= model::MAX_SHELVES)
        {
            return Err(ToolError::invalid(
                "Cabinet shelves need at most 64 rows with stable keys from 0 to 63",
            ));
        }
        let shelves = self
            .shelves
            .iter()
            .map(|s| {
                let underside = match &s.underside {
                    Some(value) => q(value, d, e)?,
                    None if matches!(s.support, SupportArgs::Fixed) => {
                        return Err(ToolError::invalid(format!(
                            "Fixed shelf {} needs underside measured from the floor",
                            s.key + 1
                        )));
                    }
                    None => previous
                        .and_then(|c| c.shelf(s.key))
                        .map_or_else(|| length(0.), |s| s.underside.clone()),
                };
                Ok(model::CabinetShelf {
                    key: s.key,
                    underside,
                    depth: q(&s.depth, d, e)?,
                    stock: s.stock.as_deref().map(id).transpose()?,
                    support: match s.support {
                        SupportArgs::Fixed => model::ShelfSupport::Fixed,
                        SupportArgs::System32 { level } => model::ShelfSupport::System32 { level },
                    },
                })
            })
            .collect::<Result<_, ToolError>>()?;
        Ok(model::CabinetOptions {
            toe_clearance: q(&self.toe_clearance, d, e)?,
            shelf_stock: id(&self.shelf_stock)?,
            shelves,
            joinery: match self.joinery {
                JoineryArg::Dowels => {
                    model::CabinetJoinery::Dowels(Box::new(self.dowels.spec(d, e)?))
                }
                JoineryArg::Domino => {
                    model::CabinetJoinery::Domino(Box::new(self.domino.spec(d, e)?))
                }
            },
            pins: self.pins.spec(d, e)?,
        })
    }
}
