//! Reusable fence construction source. The same definition drives the bay
//! editor and run planning; it contains no sample path or generated geometry.
//! Resolution checks source values, not native solid validity.
use super::{
    common::{number, parameter_links},
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod layout;
pub use layout::*;

pub const MAX_FENCE_INFILL_RECIPES: usize = 32;

/// Rectangular width runs along the post's local tangent, depth across it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FencePostProfile {
    Rectangular { width: Dimension, depth: Dimension },
    Round { diameter: Dimension },
}

/// Spacing is between post centers. Both policies retain every path corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceBayLayout {
    /// Divide each segment into equal bays no longer than the preferred pitch.
    Equal,
    /// Use the preferred pitch followed by an explicitly shorter final bay.
    ShortLast,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceShortPanelPolicy {
    Reject,
    /// A shorter bay may cut a panel down. A longer bay can never stretch it.
    Cut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FencePanelWidth {
    /// Authored to fit the clear space between the posts of each bay.
    FitBay,
    Purchased {
        width: Dimension,
        short_bay: FenceShortPanelPolicy,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FenceInfillKind {
    Panel {
        thickness: Dimension,
        width: FencePanelWidth,
    },
    /// One horizontal rectangular member in each bay. Separate recipes author
    /// separate rail heights and retain independent stable identities.
    Rail { depth: Dimension },
    /// Vertical members distributed with equal clear gaps, bounded by this
    /// maximum. The members retain their authored width rather than scaling.
    Pickets {
        width: Dimension,
        depth: Dimension,
        maximum_gap: Dimension,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceInfill {
    pub id: EntityId,
    pub name: String,
    pub material: EntityId,
    pub bottom: Dimension,
    pub height: Dimension,
    /// Signed offset across the run, permitting separately placed rails and
    /// pickets instead of implicitly intersecting all members at the datum.
    pub normal_offset: Dimension,
    pub construction: FenceInfillKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FenceConstruction {
    pub height: Dimension,
    pub post: FencePostProfile,
    pub post_material: EntityId,
    pub preferred_post_spacing: Dimension,
    pub layout: FenceBayLayout,
    pub infills: Vec<FenceInfill>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedFencePostProfile {
    Rectangular { width_mm: f64, depth_mm: f64 },
    Round { diameter_mm: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedFencePanelWidth {
    FitBay,
    Purchased {
        width_mm: f64,
        short_bay: FenceShortPanelPolicy,
    },
}
impl ResolvedFencePanelWidth {
    /// Check a measured clear bay width. Cutting is explicit; purchased panels
    /// never grow to fit, and Reject accepts only the original authored span.
    pub fn width_for_bay(self, clear_width_mm: f64) -> Result<f64, DocumentError> {
        if !clear_width_mm.is_finite() || clear_width_mm <= 0. || clear_width_mm > 1e9 {
            return Err(invalid("Fence bay needs a positive bounded clear width"));
        }
        match self {
            Self::FitBay => Ok(clear_width_mm),
            Self::Purchased {
                width_mm,
                short_bay,
            } => {
                if !width_mm.is_finite() || width_mm <= 0. || width_mm > 1e9 {
                    return Err(invalid("Purchased fence panel width is invalid"));
                }
                // Comparison tolerance never changes the physical panel width.
                if (clear_width_mm - width_mm).abs() <= 1e-6 {
                    return Ok(width_mm);
                }
                if clear_width_mm < width_mm && short_bay == FenceShortPanelPolicy::Cut {
                    return Ok(clear_width_mm);
                }
                Err(invalid(format!(
                    "Purchased panel width {width_mm:.3} mm does not fit a {clear_width_mm:.3} mm clear bay; change post positions or explicitly permit a shorter cut panel"
                )))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedFenceInfillKind {
    Panel {
        thickness_mm: f64,
        width: ResolvedFencePanelWidth,
    },
    Rail {
        depth_mm: f64,
    },
    Pickets {
        width_mm: f64,
        depth_mm: f64,
        maximum_gap_mm: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFenceInfill {
    pub id: EntityId,
    pub name: String,
    pub material: EntityId,
    pub bottom_mm: f64,
    pub height_mm: f64,
    pub normal_offset_mm: f64,
    pub construction: ResolvedFenceInfillKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFenceConstruction {
    pub height_mm: f64,
    pub post: ResolvedFencePostProfile,
    pub post_material: EntityId,
    pub preferred_post_spacing_mm: f64,
    pub layout: FenceBayLayout,
    pub infills: Vec<ResolvedFenceInfill>,
}

#[derive(Clone, Copy)]
enum Range {
    Positive,
    Nonnegative,
    Signed,
}
impl FenceConstruction {
    fn dimensions(&self) -> Vec<(&Dimension, Range)> {
        let mut values = vec![
            (&self.height, Range::Positive),
            (&self.preferred_post_spacing, Range::Positive),
        ];
        match &self.post {
            FencePostProfile::Rectangular { width, depth } => {
                values.extend([(width, Range::Positive), (depth, Range::Positive)])
            }
            FencePostProfile::Round { diameter } => values.push((diameter, Range::Positive)),
        }
        for infill in &self.infills {
            values.extend([
                (&infill.bottom, Range::Nonnegative),
                (&infill.height, Range::Positive),
                (&infill.normal_offset, Range::Signed),
            ]);
            match &infill.construction {
                FenceInfillKind::Panel { thickness, width } => {
                    values.push((thickness, Range::Positive));
                    if let FencePanelWidth::Purchased { width, .. } = width {
                        values.push((width, Range::Positive));
                    }
                }
                FenceInfillKind::Rail { depth } => values.push((depth, Range::Positive)),
                FenceInfillKind::Pickets {
                    width,
                    depth,
                    maximum_gap,
                } => values.extend([
                    (width, Range::Positive),
                    (depth, Range::Positive),
                    (maximum_gap, Range::Positive),
                ]),
            }
        }
        values
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.infills.is_empty() || self.infills.len() > MAX_FENCE_INFILL_RECIPES {
            return Err(invalid(format!(
                "A fence needs 1-{MAX_FENCE_INFILL_RECIPES} infill recipes"
            )));
        }
        let mut identities = BTreeSet::new();
        if self.post_material.is_nil() {
            return Err(invalid("Fence post material is missing"));
        }
        for infill in &self.infills {
            if infill.id.is_nil() || !identities.insert(infill.id) {
                return Err(invalid(
                    "Fence infill recipes need distinct nonempty identities",
                ));
            }
            if infill.name.trim().is_empty() || infill.name.len() > 1000 {
                return Err(invalid("Fence infill names must contain 1-1000 bytes"));
            }
            if infill.material.is_nil() {
                return Err(invalid("Fence infill material is missing"));
            }
        }
        for (dimension, range) in self.dimensions() {
            check_dimension(dimension, Unit::Millimeter)?;
            if let Dimension::Literal { value, .. } = dimension {
                check_range(*value, range)?;
            }
        }
        Ok(())
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(self.dimensions().into_iter().map(|(d, _)| d).collect());
        links.push(Link::new(self.post_material, "post/material"));
        links.extend(
            self.infills
                .iter()
                .map(|i| Link::new(i.material, format!("infill/{}/material", i.id))),
        );
        links
    }
    pub fn resolve(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedFenceConstruction, DocumentError> {
        self.validate()?;
        for (dimension, range) in self.dimensions() {
            check_range(number(dimension, Unit::Millimeter, parameters)?, range)?;
        }
        for material in
            std::iter::once(self.post_material).chain(self.infills.iter().map(|i| i.material))
        {
            if !matches!(
                document.get(material).map(|e| &e.data),
                Some(EntityData::Material { .. })
            ) {
                return Err(invalid(
                    "Fence construction needs existing canonical materials",
                ));
            }
        }
        let mm = |d| number(d, Unit::Millimeter, parameters);
        let height_mm = mm(&self.height)?;
        let post = match &self.post {
            FencePostProfile::Rectangular { width, depth } => {
                ResolvedFencePostProfile::Rectangular {
                    width_mm: mm(width)?,
                    depth_mm: mm(depth)?,
                }
            }
            FencePostProfile::Round { diameter } => ResolvedFencePostProfile::Round {
                diameter_mm: mm(diameter)?,
            },
        };
        let mut infills = Vec::with_capacity(self.infills.len());
        for infill in &self.infills {
            let bottom_mm = mm(&infill.bottom)?;
            let infill_height = mm(&infill.height)?;
            if bottom_mm + infill_height > height_mm {
                return Err(invalid(format!(
                    "Fence infill {} extends above its posts",
                    infill.name
                )));
            }
            let construction = match &infill.construction {
                FenceInfillKind::Panel { thickness, width } => ResolvedFenceInfillKind::Panel {
                    thickness_mm: mm(thickness)?,
                    width: match width {
                        FencePanelWidth::FitBay => ResolvedFencePanelWidth::FitBay,
                        FencePanelWidth::Purchased { width, short_bay } => {
                            ResolvedFencePanelWidth::Purchased {
                                width_mm: mm(width)?,
                                short_bay: *short_bay,
                            }
                        }
                    },
                },
                FenceInfillKind::Rail { depth } => ResolvedFenceInfillKind::Rail {
                    depth_mm: mm(depth)?,
                },
                FenceInfillKind::Pickets {
                    width,
                    depth,
                    maximum_gap,
                } => ResolvedFenceInfillKind::Pickets {
                    width_mm: mm(width)?,
                    depth_mm: mm(depth)?,
                    maximum_gap_mm: mm(maximum_gap)?,
                },
            };
            infills.push(ResolvedFenceInfill {
                id: infill.id,
                name: infill.name.clone(),
                material: infill.material,
                bottom_mm,
                height_mm: infill_height,
                normal_offset_mm: mm(&infill.normal_offset)?,
                construction,
            });
        }
        Ok(ResolvedFenceConstruction {
            height_mm,
            post,
            post_material: self.post_material,
            preferred_post_spacing_mm: mm(&self.preferred_post_spacing)?,
            layout: self.layout,
            infills,
        })
    }
}

fn check_range(value: f64, range: Range) -> Result<(), DocumentError> {
    if !value.is_finite()
        || value.abs() > 1e9
        || match range {
            Range::Positive => value <= 0.,
            Range::Nonnegative => value < 0.,
            Range::Signed => false,
        }
    {
        return Err(invalid(
            "Fence dimension is outside its supported positive/nonnegative or signed model range",
        ));
    }
    Ok(())
}
