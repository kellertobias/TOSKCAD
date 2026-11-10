//! Exact, evaluation-local analytic geometry. No tessellation or persistent
//! topology ordinal is used to reconstruct these descriptors.
use super::{ReferenceFrame, TopologyKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalyticGeometry {
    Point {
        position_mm: [f64; 3],
    },
    Line {
        a_mm: [f64; 3],
        b_mm: [f64; 3],
    },
    CircularArc {
        frame: ReferenceFrame,
        radius_mm: f64,
        start_rad: f64,
        sweep_rad: f64,
    },
    EllipticArc {
        frame: ReferenceFrame,
        semiaxes_mm: [f64; 2],
        start_rad: f64,
        sweep_rad: f64,
    },
    InvoluteArc {
        frame: ReferenceFrame,
        base_radius_mm: f64,
        parameters: [f64; 2],
    },
    Plane {
        frame: ReferenceFrame,
    },
}
fn point_valid(p: [f64; 3]) -> bool {
    p.into_iter().all(|v| v.is_finite() && v.abs() <= 1e9)
}
fn valid_arc(frame: ReferenceFrame, start_rad: f64, sweep_rad: f64) -> bool {
    frame.validate().is_ok()
        && start_rad.is_finite()
        && start_rad.abs() <= 1e9
        && sweep_rad.is_finite()
        && sweep_rad.abs() > 1e-12
        && sweep_rad.abs() <= std::f64::consts::TAU + 1e-9
}

impl AnalyticGeometry {
    pub fn topology_kind(self) -> TopologyKind {
        match self {
            Self::Point { .. } => TopologyKind::Vertex,
            Self::Line { .. }
            | Self::CircularArc { .. }
            | Self::EllipticArc { .. }
            | Self::InvoluteArc { .. } => TopologyKind::Edge,
            Self::Plane { .. } => TopologyKind::Face,
        }
    }
    pub fn validate(self) -> Result<(), String> {
        let valid = match self {
            Self::Point { position_mm } => point_valid(position_mm),
            Self::Line { a_mm, b_mm } => {
                point_valid(a_mm)
                    && point_valid(b_mm)
                    && a_mm
                        .iter()
                        .zip(b_mm)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f64>()
                        > 1e-20
            }
            Self::CircularArc {
                frame,
                radius_mm,
                start_rad,
                sweep_rad,
            } => (1e-9..=1e9).contains(&radius_mm) && valid_arc(frame, start_rad, sweep_rad),
            Self::EllipticArc {
                frame,
                semiaxes_mm: [major, minor],
                start_rad,
                sweep_rad,
            } => {
                minor <= major
                    && (1e-9..=1e9).contains(&minor)
                    && (1e-9..=1e9).contains(&major)
                    && valid_arc(frame, start_rad, sweep_rad)
            }
            Self::InvoluteArc {
                frame,
                base_radius_mm,
                parameters: [a, b],
            } => {
                frame.validate().is_ok()
                    && (1e-9..=1e7).contains(&base_radius_mm)
                    && [a, b]
                        .into_iter()
                        .all(|v| (0.0..=std::f64::consts::PI).contains(&v))
                    && a != b
            }
            Self::Plane { frame } => frame.validate().is_ok(),
        };
        if valid {
            Ok(())
        } else {
            Err("Invalid exact analytic geometry descriptor".into())
        }
    }
    /// Apply one checked rigid occurrence pose without changing source geometry.
    pub fn transformed(
        self,
        point: impl Fn([f64; 3]) -> [f64; 3],
        vector: impl Fn([f64; 3]) -> [f64; 3],
    ) -> Self {
        let frame = |f: ReferenceFrame| ReferenceFrame {
            origin_mm: point(f.origin_mm),
            u: vector(f.u),
            v: vector(f.v),
        };
        match self {
            Self::Point { position_mm } => Self::Point {
                position_mm: point(position_mm),
            },
            Self::Line { a_mm, b_mm } => Self::Line {
                a_mm: point(a_mm),
                b_mm: point(b_mm),
            },
            Self::CircularArc {
                frame: f,
                radius_mm,
                start_rad,
                sweep_rad,
            } => Self::CircularArc {
                frame: frame(f),
                radius_mm,
                start_rad,
                sweep_rad,
            },
            Self::EllipticArc {
                frame: f,
                semiaxes_mm,
                start_rad,
                sweep_rad,
            } => Self::EllipticArc {
                frame: frame(f),
                semiaxes_mm,
                start_rad,
                sweep_rad,
            },
            Self::InvoluteArc {
                frame: f,
                base_radius_mm,
                parameters,
            } => Self::InvoluteArc {
                frame: frame(f),
                base_radius_mm,
                parameters,
            },
            Self::Plane { frame: f } => Self::Plane { frame: frame(f) },
        }
    }
}
