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
    pub(crate) face_origins: Option<Vec<Vec<Origin>>>,
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
        if stock
            .bore_treatments
            .as_ref()
            .is_some_and(|t| !t.geometry.counterbores().is_empty())
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Combined counterbore and chamfer treatments are not yet supported",
            ));
        }
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
        if stock
            .bore_treatments
            .as_ref()
            .is_some_and(|t| !t.geometry.counterbores().is_empty())
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Combined counterbore and chamfer treatments are not yet supported",
            ));
        }
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
        stock.bore_treatments = Some(Rc::new(BoreTreatments {
            geometry,
            origins,
            face_origins: None,
        }));
        Ok(Self {
            body: SolidBody::PolygonPrism(Rc::new(stock)),
            policy,
            provenance: vec![],
            rust_origins: self.rust_origins.clone(),
            retained_placement: None,
        })
    }

    /// Apply cylindrical seats to a certified polygonal stock with existing
    /// through-bores. The original small bore and all stock sides remain.
    pub fn rust_counterbore_polygon(
        &self,
        cuts: &[multicad_rust_kernel::chamfered_bores::Counterbore],
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        self.with_polygon_counterbores(cuts, origins, policy, None)
    }

    pub(super) fn with_polygon_counterbores(
        &self,
        cuts: &[multicad_rust_kernel::chamfered_bores::Counterbore],
        origins: &[Origin],
        policy: GeometryPolicy,
        opaque_faces: Option<Vec<Vec<u8>>>,
    ) -> Result<Self, GeometryError> {
        if self.policy != policy {
            return Err(input("Counterbores require the stock geometry policy"));
        }
        circular_pocket_body::check_origins(cuts.len(), origins)?;
        let stock = self
            .polygon_prism()
            .filter(|s| s.pockets.is_empty() && s.bore_treatments.is_none())
            .ok_or_else(|| input("Counterbores require untreated polygonal through-bore stock"))?;
        let geometry = ChamferedBores::counterbored(
            &stock.prism,
            cuts,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .map_err(|cause| {
            error(
                "Rust counterbore",
                GeometryErrorKind::InvalidResult,
                format!("Invalid counterbore: {cause:?}"),
            )
        })?;
        let cut_origins = cuts
            .iter()
            .zip(origins)
            .map(|(cut, origin)| ((cut.hole, cut.end), origin.clone()))
            .collect();
        let mut stock = stock.clone();
        stock.bore_treatments = Some(Rc::new(BoreTreatments {
            geometry,
            origins: cut_origins,
            face_origins: None,
        }));
        if let Some(keys) = &opaque_faces {
            if keys.len() != stock.faces().len() {
                return Err(input("Counterbore source face count differs"));
            }
            let identities = keys
                .iter()
                .map(|key| {
                    vec![Origin {
                        source: stock.feature,
                        role: format!(
                            "step.counterbore-finite-face/{}",
                            multicad_core::project_file::ContentHash::of(key)
                        ),
                    }]
                })
                .collect();
            Rc::make_mut(stock.bore_treatments.as_mut().expect("new treatments")).face_origins =
                Some(identities);
        }
        let result = Self {
            body: SolidBody::PolygonPrism(Rc::new(stock)),
            policy,
            provenance: vec![],
            rust_origins: self.rust_origins.clone(),
            retained_placement: None,
        };
        if opaque_faces.is_none() && self.has_authored_metadata_override() {
            result.retain_authored_sources_with_generated(&[self], origins)
        } else {
            Ok(result)
        }
    }
}
