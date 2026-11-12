//! Internal ring gears: an exact outer rim minus one periodic involute cavity.
//!
//! The construction follows the native ring exactly. Phase zero denotes a
//! MATERIAL tooth on +X. The space between teeth `i` and `i + 1` is the
//! complementary external profile with half base angle `pi / z - half_base`:
//! involute flanks run outward from the inward tip circle to a foot where a
//! circular fillet meets the OUTER root circle.

use super::{
    Edge, Error, FRAC_PI_2, Involute, MAX_COORDINATE_MM, PI, Point, TAU, ToothEdge, ToothRole, add,
    normal_sweep, polar, rotate, scale,
};
use crate::{
    bevel_gear::Containment,
    display::{DisplayMesh, Footprint, MeshError, Tessellation},
    revolution::{Sum, length},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingSpec {
    pub teeth: u32,
    pub pitch_radius_mm: f64,
    pub base_radius_mm: f64,
    /// Inward tooth tips; smaller than the pitch radius.
    pub tip_radius_mm: f64,
    /// Outward roots of the spaces; larger than the pitch radius.
    pub root_radius_mm: f64,
    pub root_fillet_radius_mm: f64,
    /// Half the angular MATERIAL tooth width at the base circle.
    pub half_base_angle_rad: f64,
    pub face_width_mm: f64,
    pub outer_radius_mm: f64,
}

/// Material half angle of one tooth as a function of radius.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Width {
    base_mm: f64,
    half_base_rad: f64,
    pitch_rad: f64,
    foot_mm: f64,
    center_mm: f64,
    center_rad: f64,
    fillet_mm: f64,
    tip_half_rad: f64,
    root_half_rad: f64,
}

impl Width {
    /// Valid between the tip and root circles, where the cavity boundary has
    /// one point at each radius on either side of a tooth.
    fn at(self, radius: f64) -> f64 {
        if radius <= self.foot_mm {
            let t = ((radius - self.base_mm) * (radius + self.base_mm)).sqrt() / self.base_mm;
            return self.half_base_rad + t - t.atan();
        }
        let cos = (radius * radius + self.center_mm * self.center_mm
            - self.fillet_mm * self.fillet_mm)
            / (2. * radius * self.center_mm);
        self.pitch_rad - (cos.clamp(-1., 1.).acos() - self.center_rad)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ring {
    cavity: Vec<ToothEdge>,
    teeth: u32,
    outer_radius_mm: f64,
    height_mm: f64,
    tip_radius_mm: f64,
    root_radius_mm: f64,
    area_mm2: f64,
    centroid: Point,
    width: Width,
}

impl Ring {
    /// Refuse the native internal-ring envelope: 4..=128 teeth, ordered
    /// base < tip < pitch < root < outer radii, a rim of at least the minimum
    /// feature, a fillet center outside the base circle, a resolvable inward
    /// tooth and a separated root space.
    pub fn new(
        spec: RingSpec,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
        tolerance_mm: f64,
    ) -> Result<Self, Error> {
        let RingSpec {
            teeth,
            pitch_radius_mm: pitch,
            base_radius_mm: base,
            tip_radius_mm: tip,
            root_radius_mm: root,
            root_fillet_radius_mm: fillet,
            half_base_angle_rad: half_base,
            face_width_mm: height,
            outer_radius_mm: outer,
        } = spec;
        let range = minimum_mm..=maximum_coordinate_mm;
        if !(minimum_mm > 0.
            && minimum_mm < maximum_coordinate_mm
            && maximum_coordinate_mm <= MAX_COORDINATE_MM
            && tolerance_mm > 0.
            && tolerance_mm <= minimum_mm)
            || !(4..=128).contains(&teeth)
            || ![pitch, base, tip, root, fillet, height, outer]
                .into_iter()
                .all(|value| range.contains(&value))
            || !half_base.is_finite()
            || base >= tip
            || tip >= pitch
            || root <= pitch
            || outer - root < minimum_mm
            || root - fillet <= base
        {
            return Err(Error::InvalidDimensions);
        }
        let pitch_rad = PI / f64::from(teeth);
        let start = ((tip - base) * (tip + base)).sqrt() / base;
        let tip_half = half_base + start - start.atan();
        if tip_half * tip < minimum_mm || tip_half >= pitch_rad {
            return Err(Error::InvalidDimensions);
        }
        let space_half = pitch_rad - half_base;
        let inner = root - fillet;
        let end = (((inner - base) * (inner + base)).sqrt() + fillet) / base;
        if end <= start {
            return Err(Error::InvalidProfile);
        }
        // The exact flank parameter is bounded to one half turn.
        let flank = Involute::new(base, [start, end], -space_half).map_err(|_| Error::Limit)?;
        let foot = flank.point(1.);
        let center = add(foot, polar(fillet, end - space_half + FRAC_PI_2));
        let center_mm = length(center);
        let root_point = scale(center, root / center_mm);
        let root_half = -root_point[1].atan2(root_point[0]);
        let space_tip = space_half - (start - start.atan());
        if length(foot) >= root
            || root_half <= tolerance_mm / root
            || root_half >= pitch_rad
            || space_tip <= 0.
            || space_tip >= pitch_rad - tolerance_mm / tip
            || (center_mm - inner).abs() > tolerance_mm
        {
            return Err(Error::InvalidProfile);
        }
        let spaces = (0..teeth)
            .map(|space| space_edges(flank, [center, root_point], [space, teeth], pitch_rad))
            .collect::<Result<Vec<_>, _>>()?;
        let mut cavity: Vec<ToothEdge> = spaces.into_iter().flatten().collect();
        // Material lies on the left of the clockwise cavity.
        cavity.reverse();
        for entry in &mut cavity {
            entry.edge = match entry.edge {
                Edge::Planar(edge) => Edge::Planar(edge.reversed()),
                Edge::Involute(curve) => Edge::Involute(curve.reversed()),
            };
        }
        let mut moments = [Sum::default(); 3];
        moments[0].add(PI * outer * outer);
        for entry in &cavity {
            let measures = match entry.edge {
                Edge::Planar(edge) => edge.moments([0.; 2]),
                Edge::Involute(curve) => {
                    let [x, y] = curve.first_moment_mm3();
                    [curve.signed_area_mm2(), x, y]
                }
            };
            for (total, measure) in moments.iter_mut().zip(measures) {
                total.add(measure);
            }
        }
        let [area_mm2, x, y] = moments.map(Sum::value);
        Ok(Self {
            cavity,
            teeth,
            outer_radius_mm: outer,
            height_mm: height,
            tip_radius_mm: tip,
            root_radius_mm: root,
            area_mm2,
            centroid: [x / area_mm2, y / area_mm2],
            width: Width {
                base_mm: base,
                half_base_rad: half_base,
                pitch_rad,
                foot_mm: length(foot),
                center_mm,
                center_rad: center[1].atan2(center[0]),
                fillet_mm: fillet,
                tip_half_rad: tip_half,
                root_half_rad: root_half,
            },
        })
    }

    /// Clockwise cavity edges with their native tooth roles.
    pub fn cavity(&self) -> &[ToothEdge] {
        &self.cavity
    }

    /// The counterclockwise outside rim.
    pub fn rim(&self) -> normal_sweep::Edge {
        normal_sweep::Edge::Circle {
            center: [0.; 2],
            radius_mm: self.outer_radius_mm,
            clockwise: false,
        }
    }

    pub fn teeth(&self) -> u32 {
        self.teeth
    }

    pub fn outer_radius_mm(&self) -> f64 {
        self.outer_radius_mm
    }

    pub fn tip_radius_mm(&self) -> f64 {
        self.tip_radius_mm
    }

    pub fn root_radius_mm(&self) -> f64 {
        self.root_radius_mm
    }

    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }

    /// Exact transverse area from the Green integrals of every boundary.
    pub fn area_mm2(&self) -> f64 {
        self.area_mm2
    }

    pub fn volume_mm3(&self) -> f64 {
        self.area_mm2 * self.height_mm
    }

    pub fn surface_area_mm2(&self) -> f64 {
        let perimeter = self
            .cavity
            .iter()
            .map(|entry| entry.edge.length_mm())
            .sum::<f64>()
            + TAU * self.outer_radius_mm;
        2. * self.area_mm2 + perimeter * self.height_mm
    }

    /// Volume centroid from the exact first moments of the boundary.
    pub fn centroid(&self) -> [f64; 3] {
        [self.centroid[0], self.centroid[1], self.height_mm / 2.]
    }

    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if !direction.into_iter().all(f64::is_finite) {
            return f64::NAN;
        }
        let [x, y, z] = direction;
        self.outer_radius_mm * x.hypot(y) + (z * self.height_mm).max(0.)
    }

    pub fn bounds(&self) -> [[f64; 3]; 2] {
        let r = self.outer_radius_mm;
        [[-r, -r, 0.], [r, r, self.height_mm]]
    }

    /// Exact material side of a local point. `Boundary` is reported only when
    /// a sampled boundary point lies within the tolerance.
    pub fn classify(&self, [x, y, z]: [f64; 3], tolerance_mm: f64) -> Containment {
        if ![x, y, z, tolerance_mm].into_iter().all(f64::is_finite) || tolerance_mm < 0. {
            return Containment::Outside;
        }
        let w = self.width;
        let r = x.hypot(y);
        let step = TAU / f64::from(self.teeth);
        let angle = y.atan2(x);
        let offset = (angle - (angle / step).round() * step).abs();
        let in_root_space = offset >= w.pitch_rad - w.root_half_rad;
        let on_tip = offset <= w.tip_half_rad;
        let outer = self.outer_radius_mm;
        let (inside, near) = if r >= outer {
            (false, r - outer)
        } else if r > self.root_radius_mm {
            let root = if in_root_space {
                r - self.root_radius_mm
            } else {
                f64::INFINITY
            };
            (true, (outer - r).min(root))
        } else if r < self.tip_radius_mm {
            let tip = if on_tip {
                self.tip_radius_mm - r
            } else {
                f64::INFINITY
            };
            (false, tip)
        } else {
            let half = w.at(r);
            let mut near = r * (offset - half).abs();
            if on_tip {
                near = near.min(r - self.tip_radius_mm);
            }
            if in_root_space {
                near = near.min(self.root_radius_mm - r);
            }
            (offset < half, near)
        };
        // Positive above either cap; otherwise minus the nearest cap distance.
        let below = (-z).max(z - self.height_mm);
        let distance = if below > 0. {
            if inside { below } else { below.hypot(near) }
        } else if inside {
            near.min(-below)
        } else {
            near
        };
        if distance <= tolerance_mm {
            Containment::Boundary
        } else if inside && below < 0. {
            Containment::Inside
        } else {
            Containment::Outside
        }
    }

    /// Mesh with cap faces 0 and 1, one face per cavity edge in authored
    /// order, then the outside rim.
    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        // Rim chords never reach the root circle of the cavity.
        let rim = Tessellation {
            deflection: options
                .deflection
                .min((self.outer_radius_mm - self.root_radius_mm) / 2.),
            ..options
        };
        let mut footprint = Footprint::default();
        footprint.circle(
            [0.; 2],
            self.outer_radius_mm,
            self.cavity.len() as u32 + 2,
            false,
            rim,
        )?;
        let limit = (options.triangle_limit + 4) / 4;
        let mut samples = Vec::new();
        for (index, entry) in self.cavity.iter().enumerate() {
            let count = entry.edge.segment_count(options)?;
            if samples.len() + count > limit {
                return Err(MeshError::BudgetExceeded);
            }
            let face = index as u32 + 2;
            samples.extend((0..count).map(|i| (entry.edge.point(i as f64 / count as f64), face)));
        }
        footprint.hole(samples.into_iter(), options)?;
        footprint.tessellate(0., self.height_mm)
    }
}

/// One native counterclockwise space between teeth `space` and
/// `space + 1`, with endpoints shared bit-for-bit between joined edges.
fn space_edges(
    flank: Involute,
    [center, root]: [Point; 2],
    [space, teeth]: [u32; 2],
    pitch_rad: f64,
) -> Result<[ToothEdge; 6], Error> {
    let phase = |index: u32| f64::from(2 * index + 1) * pitch_rad;
    let next = (space + 1) % teeth;
    let angle = phase(space);
    let place = |p| rotate(p, angle);
    let opposite = |[x, y]: Point| place([x, -y]);
    let right = flank.rotated(angle)?;
    let left = flank.reflected().reversed().rotated(angle)?;
    let following = flank.rotated(phase(next))?;
    let arc = |center, a, b| {
        Edge::Planar(normal_sweep::Edge::Arc {
            center,
            a,
            b,
            clockwise: false,
        })
    };
    let edges = [
        (space, ToothRole::RightFlank, Edge::Involute(right)),
        (
            space,
            ToothRole::RightFillet,
            arc(place(center), right.point(1.), place(root)),
        ),
        (
            space,
            ToothRole::Root,
            arc([0.; 2], place(root), opposite(root)),
        ),
        (
            next,
            ToothRole::LeftFillet,
            arc(opposite(center), opposite(root), left.point(0.)),
        ),
        (next, ToothRole::LeftFlank, Edge::Involute(left)),
        (
            next,
            ToothRole::Tip,
            arc([0.; 2], left.point(1.), following.point(0.)),
        ),
    ];
    Ok(edges.map(|(tooth, role, edge)| ToothEdge { tooth, role, edge }))
}

#[cfg(test)]
mod tests;
