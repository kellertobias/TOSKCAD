//! Exact straight rack outlines with tangent circular transitions at each root.

use crate::normal_sweep::{self, Edge, Sweep};
use std::f64::consts::FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    /// Complete centered pitch cells, bounded to 4..=64.
    pub teeth: u32,
    pub pitch_mm: f64,
    pub pitch_thickness_mm: f64,
    pub pressure_angle_rad: f64,
    pub addendum_mm: f64,
    pub dedendum_mm: f64,
    pub root_fillet_radius_mm: f64,
    pub backing_thickness_mm: f64,
    pub face_width_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDimensions,
    InvalidProfile,
    Limit,
    Sweep(normal_sweep::Error),
}

#[derive(Clone, Copy)]
struct Tooth {
    circle_x: f64,
    circle_y: f64,
    tangent_x: f64,
    tangent_y: f64,
    tip_x: f64,
}

impl Tooth {
    fn new(spec: Spec, minimum_mm: f64) -> Result<Self, Error> {
        let (sin, cos) = spec.pressure_angle_rad.sin_cos();
        let slope = spec.pressure_angle_rad.tan();
        let circle_y = -spec.dedendum_mm + spec.root_fillet_radius_mm;
        let circle_x =
            spec.pitch_thickness_mm / 2. + spec.root_fillet_radius_mm / cos - circle_y * slope;
        let tangent_x = circle_x - spec.root_fillet_radius_mm * cos;
        let tangent_y = circle_y - spec.root_fillet_radius_mm * sin;
        let tip_x = spec.pitch_thickness_mm / 2. - spec.addendum_mm * slope;
        if spec.pitch_mm / 2. - circle_x < minimum_mm
            || 2. * tip_x < minimum_mm
            || tangent_y > -minimum_mm
            || spec.root_fillet_radius_mm * (FRAC_PI_2 - spec.pressure_angle_rad) < minimum_mm
        {
            return Err(Error::InvalidProfile);
        }
        Ok(Self {
            circle_x,
            circle_y,
            tangent_x,
            tangent_y,
            tip_x,
        })
    }
}

struct Outline {
    edges: Vec<Edge>,
    roles: Vec<String>,
}

impl Outline {
    fn line(&mut self, a: [f64; 2], b: [f64; 2], role: impl Into<String>) {
        self.edges.push(Edge::Line { a, b });
        self.roles.push(role.into());
    }

    fn fillet(&mut self, a: [f64; 2], b: [f64; 2], center: [f64; 2], role: String) {
        self.edges.push(Edge::Arc {
            center,
            a,
            b,
            clockwise: true,
        });
        self.roles.push(role);
    }
}

/// Returns one counterclockwise XY ring and the corresponding edge roles.
/// The pitch line is Y=0. The outline is centered in X.
pub fn outline(
    spec: Spec,
    minimum_mm: f64,
    maximum_coordinate_mm: f64,
) -> Result<(Vec<Edge>, Vec<String>), Error> {
    if !(4..=64).contains(&spec.teeth) {
        return Err(Error::Limit);
    }
    if !minimum_mm.is_finite()
        || minimum_mm <= 0.
        || !(minimum_mm..=1e7).contains(&maximum_coordinate_mm)
        || maximum_coordinate_mm == minimum_mm
        || !(0. < spec.pressure_angle_rad && spec.pressure_angle_rad < FRAC_PI_2)
    {
        return Err(Error::InvalidDimensions);
    }
    let lengths = [
        spec.pitch_mm,
        spec.pitch_thickness_mm,
        spec.addendum_mm,
        spec.dedendum_mm,
        spec.root_fillet_radius_mm,
        spec.backing_thickness_mm,
        spec.face_width_mm,
    ];
    if lengths
        .into_iter()
        .any(|value| !(minimum_mm..=maximum_coordinate_mm).contains(&value))
    {
        return Err(Error::InvalidDimensions);
    }
    let tooth = Tooth::new(spec, minimum_mm)?;
    let half_length = f64::from(spec.teeth) * spec.pitch_mm / 2.;
    let root_y = -spec.dedendum_mm;
    let bottom_y = root_y - spec.backing_thickness_mm;
    if half_length > maximum_coordinate_mm || -bottom_y > maximum_coordinate_mm {
        return Err(Error::Limit);
    }

    let count = spec.teeth as usize * 6 + 4;
    let mut ring = Outline {
        edges: Vec::with_capacity(count),
        roles: Vec::with_capacity(count),
    };
    let left_bottom = [-half_length, bottom_y];
    let right_bottom = [half_length, bottom_y];
    let mut current = [half_length, root_y];
    ring.line(left_bottom, right_bottom, "rack/backing/bottom");
    ring.line(right_bottom, current, "rack/end/right");
    for ordinal in (0..spec.teeth).rev() {
        let center = (f64::from(ordinal) + 0.5 - f64::from(spec.teeth) / 2.) * spec.pitch_mm;
        let right_root = [center + tooth.circle_x, root_y];
        let right_tangent = [center + tooth.tangent_x, tooth.tangent_y];
        let right_tip = [center + tooth.tip_x, spec.addendum_mm];
        let left_tip = [center - tooth.tip_x, spec.addendum_mm];
        let left_tangent = [center - tooth.tangent_x, tooth.tangent_y];
        let left_root = [center - tooth.circle_x, root_y];
        let prefix = format!("rack/tooth/{ordinal}");
        ring.line(current, right_root, format!("rack/space/{ordinal}"));
        ring.fillet(
            right_root,
            right_tangent,
            [center + tooth.circle_x, tooth.circle_y],
            format!("{prefix}/fillet/right"),
        );
        ring.line(right_tangent, right_tip, format!("{prefix}/flank/right"));
        ring.line(right_tip, left_tip, format!("{prefix}/tip"));
        ring.line(left_tip, left_tangent, format!("{prefix}/flank/left"));
        ring.fillet(
            left_tangent,
            left_root,
            [center - tooth.circle_x, tooth.circle_y],
            format!("{prefix}/fillet/left"),
        );
        current = left_root;
    }
    ring.line(current, [-half_length, root_y], "rack/space/left_end");
    ring.line([-half_length, root_y], left_bottom, "rack/end/left");
    Ok((ring.edges, ring.roles))
}

/// Sweeps the rack across its face width, retaining the outline's wall roles.
pub fn build(
    spec: Spec,
    tolerance_mm: f64,
    minimum_mm: f64,
    maximum_coordinate_mm: f64,
) -> Result<(Sweep, Vec<String>), Error> {
    let (edges, roles) = outline(spec, minimum_mm, maximum_coordinate_mm)?;
    let sweep = Sweep::new(vec![edges], spec.face_width_mm, tolerance_mm, minimum_mm)
        .map_err(Error::Sweep)?;
    Ok((sweep, roles))
}

#[cfg(test)]
mod tests;
