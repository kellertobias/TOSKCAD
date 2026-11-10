//! Full circle revolution as one periodic analytic face with two seam uses.
use super::*;
use multicad_core::references::{
    AnalyticGeometry, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind, TopologySnapshot,
};
use multicad_rust_kernel::torus::Torus;
use std::{collections::BTreeSet, f64::consts::TAU};

pub(crate) struct TorusBody {
    pub(super) geometry: Torus,
    pub(super) feature: EntityId,
    profile: Origin,
}

impl Solid {
    pub(super) fn rust_torus_named(
        radii_mm: [f64; 2],
        frame: PlaneFrame,
        feature: EntityId,
        profile: Origin,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let [major, minor] = radii_mm;
        policy.validate()?;
        frame.checked(policy)?;
        policy.length(minor)?;
        policy.length(major - minor)?;
        let geometry =
            Torus::new(major, minor).ok_or_else(|| input("Invalid regular torus radii"))?;
        let body = TorusBody {
            geometry,
            feature,
            profile,
        };
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Torus(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn torus_body(&self) -> Option<(&TorusBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Torus(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl TorusBody {
    pub(crate) fn radii_mm(&self) -> [f64; 2] {
        [
            self.geometry.major_radius_mm(),
            self.geometry.minor_radius_mm(),
        ]
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let major = self.geometry.major_radius_mm();
        let minor = self.geometry.minor_radius_mm();
        let half: [f64; 3] = std::array::from_fn(|i| major * frame.u[i].hypot(frame.v[i]) + minor);
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - half[i]),
            max: std::array::from_fn(|i| frame.origin[i] + half[i]),
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
        let major = self.geometry.major_radius_mm();
        let minor = self.geometry.minor_radius_mm();
        let origin = |role: &str| Origin {
            source: self.feature,
            role: role.into(),
        };
        let face_origins = BTreeSet::from([
            self.profile.clone(),
            origin(&format!(
                "side/{}/{}",
                self.profile.source, self.profile.role
            )),
        ]);
        let face = TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 0,
            },
            origins: face_origins.clone(),
            geometry: None,
            frames: vec![],
            boundary: BTreeSet::new(),
        };
        let seam = |ordinal, role, frame, radius_mm| TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Edge,
                ordinal,
            },
            origins: BTreeSet::from([origin(role)]),
            geometry: Some(AnalyticGeometry::CircularArc {
                frame,
                radius_mm,
                start_rad: 0.,
                sweep_rad: TAU,
            }),
            frames: vec![],
            boundary: face_origins.clone(),
        };
        let equator = seam(
            0,
            "torus.equator",
            ReferenceFrame {
                origin_mm: frame.origin,
                u: frame.u,
                v: frame.v,
            },
            major + minor,
        );
        let meridian = seam(
            1,
            "torus.meridian",
            ReferenceFrame {
                origin_mm: frame.point([major, 0.]),
                u: frame.u,
                v: frame.normal(),
            },
            minor,
        );
        let vertex = TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Vertex,
                ordinal: 0,
            },
            origins: BTreeSet::from([origin("torus.seam.vertex")]),
            geometry: Some(AnalyticGeometry::Point {
                position_mm: frame.point([major + minor, 0.]),
            }),
            frames: vec![],
            boundary: equator.origins.union(&meridian.origins).cloned().collect(),
        };
        TopologySnapshot {
            owner,
            revision,
            elements: vec![face, equator, meridian, vertex],
        }
    }
}
