use crate::{
    constraint_sketch::Dimension,
    document::{DocumentError, EntityId},
    model::{
        AlignmentInput, BodyInput, OccurrencePath,
        common::number,
        length,
        source::{check_dimension, invalid},
    },
    parameters::{EvaluatedParameters, Unit},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransferSelection {
    Circles { references: Vec<EntityId> },
    FaceHoles { face: EntityId },
    FaceBoundary { face: EntityId },
}
impl TransferSelection {
    pub fn captures(&self) -> Vec<EntityId> {
        match self {
            Self::Circles { references } => references.clone(),
            Self::FaceHoles { face } | Self::FaceBoundary { face } => vec![*face],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferSource {
    pub component: EntityId,
    pub path: OccurrencePath,
    pub selection: TransferSelection,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransferDepth {
    Through,
    Blind { depth: Dimension },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferTargetParameters {
    pub diameter: Option<Dimension>,
    pub clearance: Dimension,
    pub depth: TransferDepth,
    pub reverse: bool,
}
impl Default for TransferTargetParameters {
    fn default() -> Self {
        Self {
            diameter: None,
            clearance: length(0.),
            depth: TransferDepth::Through,
            reverse: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTransferTarget {
    pub diameter_mm: Option<f64>,
    pub clearance_mm: f64,
    pub blind_depth_mm: Option<f64>,
    pub reverse: bool,
}
impl TransferTargetParameters {
    pub fn dimensions(&self) -> Vec<&Dimension> {
        let mut d = vec![&self.clearance];
        d.extend(self.diameter.iter());
        if let TransferDepth::Blind { depth } = &self.depth {
            d.push(depth);
        }
        d
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        p: &EvaluatedParameters,
    ) -> Result<ResolvedTransferTarget, DocumentError> {
        self.validate()?;
        let diameter_mm = self
            .diameter
            .as_ref()
            .map(|d| number(d, Unit::Millimeter, p))
            .transpose()?;
        let clearance_mm = number(&self.clearance, Unit::Millimeter, p)?;
        let blind_depth_mm = if let TransferDepth::Blind { depth } = &self.depth {
            Some(number(depth, Unit::Millimeter, p)?)
        } else {
            None
        };
        if diameter_mm
            .iter()
            .chain(blind_depth_mm.iter())
            .any(|v| !v.is_finite() || *v <= 0. || *v > 1e9)
            || !clearance_mm.is_finite()
            || clearance_mm.abs() > 1e9
        {
            return Err(invalid(
                "Transfer diameter/depth must be positive finite lengths; radial clearance must be finite and bounded",
            ));
        }
        Ok(ResolvedTransferTarget {
            diameter_mm,
            clearance_mm,
            blind_depth_mm,
            reverse: self.reverse,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferTarget {
    pub id: EntityId,
    pub feature: EntityId,
    pub input: BodyInput,
    pub path: OccurrencePath,
    pub face: EntityId,
    pub parameters: TransferTargetParameters,
}
impl TransferTarget {
    pub fn new(
        input: BodyInput,
        path: OccurrencePath,
        face: EntityId,
        parameters: TransferTargetParameters,
    ) -> Self {
        Self {
            id: EntityId::new(),
            feature: EntityId::new(),
            input,
            path,
            face,
            parameters,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferDefinition {
    pub source: TransferSource,
    pub targets: Vec<TransferTarget>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferFeatureDefinition {
    pub group: EntityId,
    pub target: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub group: EntityId,
    pub name: String,
    pub source: TransferSource,
    pub targets: Vec<TransferTarget>,
    pub references: Vec<AlignmentInput>,
}
impl TransferRequest {
    pub fn new(name: impl Into<String>, source: TransferSource) -> Self {
        Self {
            group: EntityId::new(),
            name: name.into(),
            source,
            targets: vec![],
            references: vec![],
        }
    }
}
