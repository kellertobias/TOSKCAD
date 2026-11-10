//! Analytic circular cuts through a planar face of supported stock.

use super::*;
use multicad_rust_kernel::circular_pockets::{self, CircularPocket, PocketError};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct CircularPocketTool {
    pub frame: PlaneFrame,
    pub center_uv: [f64; 2],
    pub radius_mm: f64,
    pub displacement_mm: [f64; 3],
    pub origin: Origin,
}

const OPERATION: &str = "blind circular cut";

fn input_error(message: impl Into<String>) -> GeometryError {
    error(OPERATION, GeometryErrorKind::Input, message)
}

fn budget_error() -> GeometryError {
    error(
        OPERATION,
        GeometryErrorKind::Limit,
        "Circular pockets exceed the analytic geometry budget",
    )
}

fn length([x, y, z]: [f64; 3]) -> f64 {
    x.hypot(y).hypot(z)
}

pub(super) fn pocket_error(cause: PocketError) -> GeometryError {
    match cause {
        PocketError::InvalidInput => input_error("Invalid circular pocket dimensions or axis"),
        PocketError::UnresolvedBoundary => {
            input_error("Circular pocket touches stock boundaries or other cuts")
        }
        PocketError::BudgetExceeded => budget_error(),
    }
}

pub(super) fn check_origins(count: usize, origins: &[Origin]) -> Result<(), GeometryError> {
    if count != origins.len()
        || origins.iter().collect::<BTreeSet<_>>().len() != origins.len()
        || origins.iter().any(|origin| {
            origin.source.is_nil()
                || origin.role.trim().is_empty()
                || format!("pocket.floor/{}/{}", origin.source, origin.role).len() > 256
        })
    {
        return Err(input_error(
            "Each circular pocket needs a distinct valid origin",
        ));
    }
    Ok(())
}

pub(crate) fn face_origin(origin: &Origin, role: &str) -> Origin {
    Origin {
        source: origin.source,
        role: format!("pocket.{role}/{}/{}", origin.source, origin.role),
    }
}

pub(super) fn tool_point(
    frame: PlaneFrame,
    center_uv: [f64; 2],
    radius_mm: f64,
    displacement_mm: [f64; 3],
    policy: GeometryPolicy,
) -> Result<[f64; 3], GeometryError> {
    frame.checked(policy)?;
    policy.length(radius_mm)?;
    policy.point(displacement_mm)?;
    let start = frame.point(center_uv);
    let finish = std::array::from_fn(|i| start[i] + displacement_mm[i]);
    policy.point(start)?;
    policy.point(finish)?;
    if length(cross(frame.normal(), displacement_mm)) > 1e-10 * length(displacement_mm) {
        return Err(input_error(
            "Circular pocket tool must travel normal to its profile",
        ));
    }
    Ok(start)
}

pub(super) fn tool_start(
    tool: &CircularPocketTool,
    policy: GeometryPolicy,
) -> Result<[f64; 3], GeometryError> {
    tool_point(
        tool.frame,
        tool.center_uv,
        tool.radius_mm,
        tool.displacement_mm,
        policy,
    )
}

impl Solid {
    /// Subtract disjoint round pockets from polygon or orthogonal stock.
    /// Each tool begins outside the material and leaves a blind floor.
    pub fn rust_drill_circular_pockets(
        &self,
        tools: &[CircularPocketTool],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if policy != self.policy || policy.boolean_fuzzy_mm != 0. {
            return Err(input_error(
                "Circular pockets require the stock's exact geometry policy",
            ));
        }
        if tools.is_empty() {
            return self.share();
        }
        if let Some((stock, frame)) = self.orthogonal_body() {
            let (pockets, origins) = stock.drill(frame, tools, policy)?;
            return self.share()?.with_orthogonal_pockets(pockets, origins);
        }
        let Some(stock) = self.polygon_prism() else {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Rust blind drilling requires polygonal or orthogonal stock",
            ));
        };
        let normal = stock.frame.normal();
        if tools
            .iter()
            .any(|tool| length(cross(tool.frame.normal(), normal)) > 1e-10)
        {
            return self
                .orthogonal_stock()?
                .rust_drill_circular_pockets(tools, policy);
        }
        if tools.len() > circular_pockets::pocket_capacity(&stock.prism) - stock.pockets.len() {
            return Err(budget_error());
        }
        let mut pockets = stock.pockets.to_vec();
        let mut origins = stock.pocket_origins.to_vec();
        let h = stock.prism.height().as_f64();
        let sign = if stock.reverse_sweep { -1. } else { 1. };
        let normal = normal.map(|x| sign * x);
        let tolerance = policy.linear_tolerance_mm;
        for tool in tools {
            let start = tool_start(tool, policy)?;
            let travel = dot(tool.displacement_mm, normal);
            let sideways = std::array::from_fn(|i| tool.displacement_mm[i] - travel * normal[i]);
            if length(sideways) > 1e-10 * travel.abs() {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Unsupported,
                    "Circular pocket tools must enter normal to a stock cap",
                ));
            }
            let delta = std::array::from_fn(|i| start[i] - stock.frame.origin[i]);
            let start_z = dot(delta, normal);
            let finish_z = start_z + travel;
            let end = travel < 0.;
            if !(if end {
                start_z > h + tolerance
            } else {
                start_z < -tolerance
            }) || finish_z <= 0.
                || finish_z >= h
            {
                return Err(input_error(
                    "Circular pocket must enter from outside one cap and leave material at the opposite cap",
                ));
            }
            pockets.push(CircularPocket {
                center_mm: [stock.frame.u, stock.frame.v].map(|axis| dot(delta, axis)),
                radius_mm: tool.radius_mm,
                depth_mm: if end { h - finish_z } else { finish_z },
                end,
            });
            origins.push(tool.origin.clone());
        }
        self.share()?.with_polygon_pockets(pockets, origins)
    }

    pub(super) fn with_polygon_pockets(
        mut self,
        pockets: Vec<CircularPocket>,
        origins: Vec<Origin>,
    ) -> Result<Self, GeometryError> {
        let SolidBody::PolygonPrism(placed) = &mut self.body else {
            return Err(input_error("Circular pockets require polygon stock"));
        };
        check_origins(pockets.len(), &origins)?;
        circular_pockets::validate(
            &placed.prism,
            &pockets,
            self.policy.linear_tolerance_mm,
            self.policy.minimum_feature_mm,
        )
        .map_err(pocket_error)?;
        let placed = Rc::make_mut(placed);
        placed.pockets = pockets.into();
        placed.pocket_origins = origins.into();
        Ok(self)
    }
}
