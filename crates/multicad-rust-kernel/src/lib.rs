//! Bounded exact lengths and one analytic curve probe for the kernel study.

pub mod annulus;
pub mod axial_profile_stack;
pub mod bevel_gear;
pub mod chamfered_bores;
pub mod circular_pockets;
pub mod cosmetic_fastener;
pub mod curved_projection;
pub mod cylinder;
pub mod display;
mod drawing_events;
pub mod headed_screw;
pub mod helical_receiver;
pub mod helical_thread;
pub mod helical_thread_mesh;
pub mod helical_tube_cut;
pub mod hex_nut;
pub mod linear_sweep;
pub mod linear_sweep_mesh;
pub mod manufacturing;
pub mod normal_relief;
pub mod normal_sweep;
pub mod orthogonal;
pub mod orthogonal_boundary;
pub mod orthogonal_pockets;
pub mod orthogonal_section;
pub mod orthogonal_slots;
pub mod planar_boolean;
pub mod planar_distance;
pub mod planar_projection;
pub mod planar_section;
pub mod polygon;
pub mod prism;
pub mod retained_section;
pub mod revolution;
pub mod revolution_mesh;
pub mod round_occlusion;
pub mod rounded_cylinder;
pub mod rounded_thread;
pub mod section_hatch;
pub mod sketch_reference;
pub mod sphere;
pub mod spline_intersection;
pub mod spline_sweep;
pub mod spur_gear;
pub mod stadium;
pub mod step;
pub mod step_read;
pub mod straight_rack;
pub mod tapered_thread;
pub mod thread_end_relief;
pub mod threaded_shaft;
pub mod torus;
pub mod trim_curve;
pub mod trimmed_sweep;
pub mod warped_union;
pub mod worm_gear;

pub use multicad_precision::{Error, GridCoordinate, Quantum, Rational, parse_length_mm};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Circle {
    center: [Rational; 2],
    radius: Rational,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerticalIntersection {
    Disjoint,
    Tangent {
        y: Rational,
    },
    /// The two ordinates are exactly `y_center ± sqrt(radicand)`.
    Secant {
        y_center: Rational,
        radicand: Rational,
    },
}

impl Circle {
    pub fn new(center: [Rational; 2], radius: Rational) -> Result<Self, Error> {
        if radius.ratio().0 <= 0 {
            return Err(Error::NonPositiveRadius);
        }
        Ok(Self { center, radius })
    }

    pub fn center(self) -> [Rational; 2] {
        self.center
    }

    pub fn radius(self) -> Rational {
        self.radius
    }

    pub fn intersect_vertical(self, x: Rational) -> Result<VerticalIntersection, Error> {
        let [center_x, y_center] = self.center;
        let dx = x.checked_sub(center_x)?;
        let radius_squared = self.radius.checked_mul(self.radius)?;
        let radicand = radius_squared.checked_sub(dx.checked_mul(dx)?)?;
        Ok(match radicand.ratio().0.signum() {
            -1 => VerticalIntersection::Disjoint,
            0 => VerticalIntersection::Tangent { y: y_center },
            _ => VerticalIntersection::Secant { y_center, radicand },
        })
    }
}
