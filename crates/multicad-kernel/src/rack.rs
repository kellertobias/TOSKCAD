//! Exact straight rack flanks, root arcs, and authored tooth identities.
use crate::{
    CurvedEdge2d, CurvedProfile, GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    Profile, ProfileEdge, Solid,
};
use multicad_core::{document::EntityId, references::Origin};
use multicad_rust_kernel::{normal_sweep::Edge, straight_rack};

pub use straight_rack::Spec as StraightRack;

fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "straight rack",
        kind,
        message: message.into(),
    }
}

fn profile(
    spec: StraightRack,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<(Vec<Edge>, Vec<Origin>), GeometryError> {
    policy.validate()?;
    if feature.is_nil() {
        return Err(failure(
            GeometryErrorKind::Input,
            "Rack feature identity must not be nil",
        ));
    }
    let (edges, roles) = straight_rack::outline(
        spec,
        policy.minimum_feature_mm,
        policy.maximum_coordinate_mm,
    )
    .map_err(|cause| match cause {
        straight_rack::Error::Limit => failure(
            GeometryErrorKind::Limit,
            "Rack exceeds the supported cell count or coordinate range",
        ),
        straight_rack::Error::InvalidDimensions => failure(
            GeometryErrorKind::Input,
            "Rack dimensions or pressure angle are outside the supported range",
        ),
        straight_rack::Error::InvalidProfile | straight_rack::Error::Sweep(_) => failure(
            GeometryErrorKind::Input,
            "Rack needs distinct tangent root circles, positive end/root flats, tooth tips and straight flanks crossing the pitch line",
        ),
    })?;
    let origins = roles
        .into_iter()
        .map(|role| Origin {
            source: feature,
            role,
        })
        .collect();
    Ok((edges, origins))
}

impl Solid {
    /// Exact rack with centered pitch cells, pitch line Y=0, and face width along Z.
    pub fn straight_rack_named(
        spec: StraightRack,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let (edges, origins) = profile(spec, feature, policy)?;
        let outer = edges
            .into_iter()
            .map(|edge| match edge {
                Edge::Line { a, b } => ProfileEdge::Line { a, b },
                Edge::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                } => ProfileEdge::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                },
                Edge::Circle {
                    center, radius_mm, ..
                } => ProfileEdge::Circle { center, radius_mm },
            })
            .collect();
        let solid = Self::extrude_profile_named(
            &Profile {
                frame: PlaneFrame::default(),
                outer,
                holes: vec![],
            },
            spec.face_width_mm,
            feature,
            &origins,
            policy,
        )?;
        if solid.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Rack must retain one connected backing solid",
            ));
        }
        Ok(solid)
    }

    /// Rust normal sweep of the same exact, named rack profile.
    pub fn rust_straight_rack_named(
        spec: StraightRack,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let (edges, origins) = profile(spec, feature, policy)?;
        let outer = edges
            .into_iter()
            .map(|edge| match edge {
                Edge::Line { a, b } => CurvedEdge2d::Line { a, b },
                Edge::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                } => CurvedEdge2d::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                },
                Edge::Circle {
                    center, radius_mm, ..
                } => CurvedEdge2d::Circle { center, radius_mm },
            })
            .collect();
        Self::rust_extrude_normal_profile_named(
            &CurvedProfile {
                frame: PlaneFrame::default(),
                outer,
                holes: vec![],
            },
            spec.face_width_mm,
            feature,
            &origins,
            policy,
        )
    }
}
