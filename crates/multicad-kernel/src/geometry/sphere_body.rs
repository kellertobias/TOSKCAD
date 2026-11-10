//! Named exact sphere, with a single periodic face, seam and two poles.
use super::*;
use multicad_core::references::{
    AnalyticGeometry, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind, TopologySnapshot,
};
use multicad_rust_kernel::sphere::Sphere;
use std::f64::consts::{FRAC_PI_2, PI};

pub(crate) struct SphereBody {
    pub(super) geometry: Sphere,
    pub(super) feature: EntityId,
}

impl Solid {
    pub fn rust_sphere_named(
        radius_mm: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        policy.length(radius_mm)?;
        if feature.is_nil() {
            return Err(input("Sphere feature identity must not be nil"));
        }
        let geometry =
            Sphere::new(radius_mm).ok_or_else(|| input("Invalid analytic sphere radius"))?;
        let body = SphereBody { geometry, feature };
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Sphere(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn sphere_body(&self) -> Option<(&SphereBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Sphere(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl SphereBody {
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let r = self.geometry.radius_mm();
        Bounds {
            min: frame.origin.map(|x| x - r),
            max: frame.origin.map(|x| x + r),
        }
    }

    pub(super) fn face(&self, frame: PlaneFrame) -> Face {
        Face {
            index: 0,
            plane: None,
            center_mm: frame.origin,
            area_mm2: self.geometry.surface_area_mm2(),
        }
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let origin = |role: &str| Origin {
            source: self.feature,
            role: role.into(),
        };
        let surface = [
            origin("primitive.edge.0"),
            origin(&format!("side/{}/primitive.edge.0", self.feature)),
        ];
        let face = TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 0,
            },
            origins: surface.iter().cloned().collect(),
            boundary: Default::default(),
            frames: vec![],
            geometry: None,
        };
        let boundary = |kind, ordinal, role, geometry| TopologyElement {
            key: TopologyKey { kind, ordinal },
            origins: surface.iter().cloned().chain([origin(role)]).collect(),
            boundary: surface.iter().cloned().collect(),
            frames: vec![],
            geometry: Some(geometry),
        };
        let n = frame.normal();
        let radius_mm = self.geometry.radius_mm();
        let seam = boundary(
            TopologyKind::Edge,
            0,
            "primitive.sphere.seam",
            AnalyticGeometry::CircularArc {
                frame: ReferenceFrame {
                    origin_mm: frame.origin,
                    u: frame.u,
                    v: n,
                },
                radius_mm,
                start_rad: -FRAC_PI_2,
                sweep_rad: PI,
            },
        );
        let pole = |ordinal, sign, role| {
            boundary(
                TopologyKind::Vertex,
                ordinal,
                role,
                AnalyticGeometry::Point {
                    position_mm: std::array::from_fn(|i| frame.origin[i] + sign * radius_mm * n[i]),
                },
            )
        };
        TopologySnapshot {
            owner,
            revision,
            elements: vec![
                face,
                seam,
                pole(0, -1., "primitive.sphere.pole.south"),
                pole(1, 1., "primitive.sphere.pole.north"),
            ],
        }
    }
}
