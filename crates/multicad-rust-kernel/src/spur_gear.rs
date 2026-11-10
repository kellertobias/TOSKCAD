//! External straight spur profiles with exact involute and circular boundaries.

use crate::normal_sweep;
use crate::revolution::{Sum, cross};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub mod clearance;
mod hub;
mod involute;
mod sweep;
pub use hub::{Hub, HubExtension};
pub use involute::{CubicSpline, Involute};

type Point = [f64; 2];
const MAX_COORDINATE_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub teeth: u32,
    pub pitch_radius_mm: f64,
    pub base_radius_mm: f64,
    pub tip_radius_mm: f64,
    pub root_radius_mm: f64,
    pub root_fillet_radius_mm: f64,
    pub half_base_angle_rad: f64,
    pub face_width_mm: f64,
    pub bore_radius_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDimensions,
    InvalidProfile,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Planar(normal_sweep::Edge),
    Involute(Involute),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToothRole {
    LeftFillet,
    LeftRadial,
    LeftFlank,
    Tip,
    RightFlank,
    RightRadial,
    RightFillet,
    Root,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToothEdge {
    pub tooth: u32,
    pub role: ToothRole,
    pub edge: Edge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    outline: Vec<ToothEdge>,
    bore: Option<normal_sweep::Edge>,
    height_mm: f64,
    root_radius_mm: f64,
    area_mm2: f64,
    hub: Option<Hub>,
}

impl Profile {
    /// Construct a single counterclockwise rim and an optional clockwise bore.
    /// The generated flank geometry is the exact involute. A consumer that
    /// needs splines can explicitly request a tolerance-bounded conversion.
    pub fn new(spec: Spec, minimum_mm: f64, maximum_coordinate_mm: f64) -> Result<Self, Error> {
        check_dimensions(spec, minimum_mm, maximum_coordinate_mm)?;
        let tooth = Tooth::new(spec, minimum_mm)?;
        let mut outline = Vec::with_capacity(spec.teeth as usize * 8);
        let increment = TAU / f64::from(spec.teeth);
        for index in 0..spec.teeth {
            tooth.append(index, spec.teeth, increment, &mut outline)?;
        }
        let bore = (spec.bore_radius_mm > 0.).then_some(normal_sweep::Edge::Circle {
            center: [0.; 2],
            radius_mm: spec.bore_radius_mm,
            clockwise: true,
        });
        let mut area = Sum::default();
        for edge in &outline {
            area.add(match edge.edge {
                Edge::Planar(edge) => {
                    cross(edge.start(), edge.end()) / 2. + normal_sweep::signed_area(&[edge])
                }
                Edge::Involute(curve) => curve.signed_area_mm2(),
            });
        }
        if let Some(bore) = bore {
            area.add(normal_sweep::signed_area(&[bore]));
        }
        Ok(Self {
            outline,
            bore,
            height_mm: spec.face_width_mm,
            root_radius_mm: spec.root_radius_mm,
            area_mm2: area.value(),
            hub: None,
        })
    }

    pub fn outline(&self) -> &[ToothEdge] {
        &self.outline
    }

    pub fn bore(&self) -> Option<normal_sweep::Edge> {
        self.bore
    }

    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }

    pub fn area_mm2(&self) -> f64 {
        self.area_mm2
    }

    pub fn volume_mm3(&self) -> f64 {
        let volume = self.area_mm2 * self.height_mm;
        self.hub.map_or(volume, |hub| {
            let bore = self.bore_radius_mm().unwrap_or(0.);
            volume
                + PI * (hub.radius_mm - bore) * (hub.radius_mm + bore) * self.extension_length_mm()
        })
    }
}

fn check_dimensions(spec: Spec, minimum: f64, maximum: f64) -> Result<(), Error> {
    if !(4..=128).contains(&spec.teeth) {
        return Err(Error::Limit);
    }
    if !minimum.is_finite()
        || minimum <= 0.
        || !(minimum..=MAX_COORDINATE_MM).contains(&maximum)
        || minimum == maximum
        || [
            spec.pitch_radius_mm,
            spec.base_radius_mm,
            spec.tip_radius_mm,
            spec.root_radius_mm,
            spec.root_fillet_radius_mm,
            spec.face_width_mm,
        ]
        .into_iter()
        .any(|value| !(minimum..=maximum).contains(&value))
        || (spec.bore_radius_mm != 0.
            && (!(minimum..=maximum).contains(&spec.bore_radius_mm)
                || spec.root_radius_mm - spec.bore_radius_mm < minimum))
        || spec.base_radius_mm > spec.pitch_radius_mm
        || spec.root_radius_mm >= spec.pitch_radius_mm
        || spec.tip_radius_mm <= spec.pitch_radius_mm
        || spec.root_radius_mm + spec.root_fillet_radius_mm >= spec.tip_radius_mm
        || !(0. < spec.half_base_angle_rad && spec.half_base_angle_rad < PI / f64::from(spec.teeth))
    {
        return Err(Error::InvalidDimensions);
    }
    Ok(())
}

struct Tooth {
    root: Point,
    fillet_center: Point,
    fillet_end: Point,
    flank: Involute,
    radial: bool,
}

impl Tooth {
    fn new(spec: Spec, minimum: f64) -> Result<Self, Error> {
        let base = spec.base_radius_mm;
        let tip = spec.tip_radius_mm;
        let root = spec.root_radius_mm;
        let radius = spec.root_fillet_radius_mm;
        let half = spec.half_base_angle_rad;
        let center_radius = root + radius;
        let foot_radius = (root * (root + 2. * radius)).sqrt();
        let radial = foot_radius < base;
        let start = if radial {
            0.
        } else {
            // Rationalized form of sqrt((root+radius)^2-base^2)-radius.
            let tangent = ((center_radius - base) * (center_radius + base)).sqrt();
            (foot_radius - base) * (foot_radius + base) / (base * (tangent + radius))
        };
        let end = ((tip - base) * (tip + base)).sqrt() / base;
        let tip_half = half - (end - end.atan());
        if start >= end || tip * tip_half < minimum {
            return Err(Error::InvalidProfile);
        }
        let flank = Involute::new(base, [start, end], -half)?;
        let fillet_end = if radial {
            polar(foot_radius, -half)
        } else {
            flank.point(0.)
        };
        let tangent_angle = start - half;
        let fillet_center = sub(fillet_end, polar(radius, tangent_angle + FRAC_PI_2));
        let root_half = -fillet_center[1].atan2(fillet_center[0]);
        let root_gap = TAU / f64::from(spec.teeth) - 2. * root_half;
        if root_half <= 0.
            || root_gap * root < minimum
            || (FRAC_PI_2 - root_half - tangent_angle) * radius < minimum
            || (radial && base - foot_radius < minimum)
            || flank.length_mm() < minimum
        {
            return Err(Error::InvalidProfile);
        }
        // Each fillet lies within the flank's angular sector, turning
        // monotonically from the root tangent to the outgoing flank tangent.
        // Increasing involute angle then separates both flanks from the tips
        // and root arcs of every neighboring tooth.
        Ok(Self {
            root: scale(fillet_center, root / center_radius),
            fillet_center,
            fillet_end,
            flank,
            radial,
        })
    }

    fn append(
        &self,
        tooth: u32,
        count: u32,
        increment: f64,
        outline: &mut Vec<ToothEdge>,
    ) -> Result<(), Error> {
        let phase = f64::from(tooth) * increment;
        let place = |p| rotate(p, phase);
        let opposite = |[x, y]: Point| place([x, -y]);
        let left = self.flank.rotated(phase)?;
        let right = self.flank.reflected().reversed().rotated(phase)?;
        let mut add = |role, edge| outline.push(ToothEdge { tooth, role, edge });
        let arc = |center, a, b, clockwise| {
            Edge::Planar(normal_sweep::Edge::Arc {
                center,
                a,
                b,
                clockwise,
            })
        };
        let line = |a, b| Edge::Planar(normal_sweep::Edge::Line { a, b });
        let [left_foot, right_foot] = if self.radial {
            [place(self.fillet_end), opposite(self.fillet_end)]
        } else {
            [left.point(0.), right.point(1.)]
        };
        add(
            ToothRole::LeftFillet,
            arc(place(self.fillet_center), place(self.root), left_foot, true),
        );
        if self.radial {
            add(ToothRole::LeftRadial, line(left_foot, left.point(0.)));
        }
        add(ToothRole::LeftFlank, Edge::Involute(left));
        add(
            ToothRole::Tip,
            arc([0.; 2], left.point(1.), right.point(0.), false),
        );
        add(ToothRole::RightFlank, Edge::Involute(right));
        if self.radial {
            add(ToothRole::RightRadial, line(right.point(1.), right_foot));
        }
        add(
            ToothRole::RightFillet,
            arc(
                opposite(self.fillet_center),
                right_foot,
                opposite(self.root),
                true,
            ),
        );
        let next_root = rotate(self.root, f64::from((tooth + 1) % count) * increment);
        add(
            ToothRole::Root,
            arc([0.; 2], opposite(self.root), next_root, false),
        );
        Ok(())
    }
}

fn polar(radius: f64, angle: f64) -> Point {
    let (sin, cos) = angle.sin_cos();
    [radius * cos, radius * sin]
}

fn rotate([x, y]: Point, angle: f64) -> Point {
    let (sin, cos) = angle.sin_cos();
    [x * cos - y * sin, x * sin + y * cos]
}

fn add([x, y]: Point, [a, b]: Point) -> Point {
    [x + a, y + b]
}

fn sub([x, y]: Point, [a, b]: Point) -> Point {
    [x - a, y - b]
}

fn scale([x, y]: Point, factor: f64) -> Point {
    [x * factor, y * factor]
}

#[cfg(test)]
mod tests;
