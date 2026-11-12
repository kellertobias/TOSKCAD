//! Apply a certified symmetric bevel to a through-bore's circular cap edge.
use super::*;
use crate::TreatmentEdge;
use multicad_rust_kernel::{
    chamfered_bores::{BoreChamfer, ChamferedBores},
    circular_pockets::PocketError,
};
use std::collections::BTreeMap;

const OPERATION: &str = "Rust edge treatment";

#[derive(Clone)]
pub(crate) struct BoreTreatments {
    pub(crate) geometry: ChamferedBores,
    pub(crate) origins: BTreeMap<(usize, bool), Origin>,
}

impl Solid {
    pub(crate) fn chamfer_polygon_bores(
        &self,
        edges: &[TreatmentEdge],
        size_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let unsupported = || {
            error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Rust chamfer requires circular through-bore edges on a polygon stock cap",
            )
        };
        let stock = self
            .polygon_prism()
            .filter(|stock| stock.pockets.is_empty())
            .ok_or_else(unsupported)?;
        if self.policy != policy {
            return Err(input("Rust chamfer requires the source geometry policy"));
        }
        let count = stock.edges.len();
        let straight = count - stock.prism.circular_holes().len();
        let mut chamfers = Vec::new();
        let mut origins = BTreeMap::new();
        if let Some(previous) = &stock.bore_treatments {
            chamfers.extend_from_slice(previous.geometry.chamfers());
            origins.clone_from(&previous.origins);
        }
        for edge in edges {
            let index = edge.key.ordinal as usize;
            if index >= 2 * count || index % count < straight {
                return Err(unsupported());
            }
            let hole = index % count - straight;
            let end = index >= count;
            if origins.contains_key(&(hole, end)) {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Unsupported,
                    "This circular bore mouth already has a chamfer",
                ));
            }
            origins.insert((hole, end), edge.origin.clone());
            chamfers.push(BoreChamfer { hole, end, size_mm });
        }
        self.with_polygon_bore_treatments(&chamfers, origins, policy)
    }

    pub(super) fn with_polygon_bore_treatments(
        &self,
        chamfers: &[BoreChamfer],
        origins: BTreeMap<(usize, bool), Origin>,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let stock = self
            .polygon_prism()
            .filter(|stock| stock.pockets.is_empty())
            .ok_or_else(|| {
                input("Circular bore chamfers require polygon stock without blind cuts")
            })?;
        if origins.len() != chamfers.len()
            || chamfers
                .iter()
                .any(|c| !origins.contains_key(&(c.hole, c.end)))
        {
            return Err(input(
                "Each circular bore chamfer needs an exact authored origin",
            ));
        }
        let geometry = ChamferedBores::new(
            &stock.prism,
            chamfers,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .map_err(|cause| {
            error(
                OPERATION,
                match cause {
                    PocketError::BudgetExceeded => GeometryErrorKind::Limit,
                    _ => GeometryErrorKind::InvalidResult,
                },
                format!("Invalid circular bore chamfer: {cause:?}"),
            )
        })?;
        let mut stock = stock.clone();
        stock.bore_treatments = Some(Rc::new(BoreTreatments { geometry, origins }));
        Ok(Self {
            body: SolidBody::PolygonPrism(Rc::new(stock)),
            policy,
            provenance: vec![],
            rust_origins: self.rust_origins.clone(),
            retained_placement: None,
        })
    }
}
