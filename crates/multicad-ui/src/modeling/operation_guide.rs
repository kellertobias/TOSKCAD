//! Display-only placement of the Extrude arrow and Revolve angular handle.
//! The guide is derived once from the solved sketch when the tool opens; it
//! positions handles only and never becomes manufacturing geometry.
use super::gizmo::math::{Point, add, cross, dot, mul, sub};
use multicad_core::{document::EntityId, sketch::Frame};

#[derive(Clone, Debug, PartialEq)]
pub struct GuideLine {
    pub id: EntityId,
    pub a: [f64; 2],
    pub b: [f64; 2],
}
/// Solved sketch frame, a representative point of the accepted profiles and
/// the straight sketch lines that may serve as an explicit revolution axis.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileGuide {
    pub sketch: EntityId,
    pub frame: Frame,
    pub anchor: [f64; 2],
    pub lines: Vec<GuideLine>,
}

fn unit(v: Point) -> Option<Point> {
    let length = dot(v, v).sqrt();
    (length.is_finite() && length > 1e-12).then(|| mul(v, 1. / length))
}

/// World start point and unit direction of the extrusion arrow. `local` is
/// the positive sweep direction in sketch U,V,N; the start is moved along N by
/// the start offset, so signed distances are measured from that plane.
pub fn extrude_axis(
    guide: &ProfileGuide,
    local: [f64; 3],
    start_offset_mm: f64,
) -> Option<(Point, Point)> {
    let frame = guide.frame;
    let direction = unit(add(
        add(mul(frame.u, local[0]), mul(frame.v, local[1])),
        mul(frame.n, local[2]),
    ))?;
    let start = add(frame.point(guide.anchor), mul(frame.n, start_offset_mm));
    Some((start, direction))
}

/// Revolution axis point/direction and the zero-angle radius vector from the
/// axis to the profile anchor. A profile centered on the axis uses the sketch
/// normal scaled to `fallback_radius` so the handle remains grabbable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RevolveRing {
    pub center: Point,
    pub axis: Point,
    pub radius: Point,
}
impl RevolveRing {
    pub fn new(
        guide: &ProfileGuide,
        origin: [f64; 2],
        bearing_rad: f64,
        fallback_radius: f64,
    ) -> Option<Self> {
        let frame = guide.frame;
        let axis = unit(add(
            mul(frame.u, bearing_rad.cos()),
            mul(frame.v, bearing_rad.sin()),
        ))?;
        let on_axis = frame.point(origin);
        let anchor = frame.point(guide.anchor);
        let along = dot(sub(anchor, on_axis), axis);
        let center = add(on_axis, mul(axis, along));
        let mut radius = sub(anchor, center);
        if dot(radius, radius).sqrt() < 1e-6 {
            radius = mul(frame.n, fallback_radius);
        }
        Some(Self {
            center,
            axis,
            radius,
        })
    }
    /// Point on the handle circle after a right-handed sweep about the axis.
    pub fn point(&self, degrees: f64) -> Point {
        let t = degrees.to_radians();
        add(
            self.center,
            add(
                mul(self.radius, t.cos()),
                mul(cross(self.axis, self.radius), t.sin()),
            ),
        )
    }
    /// Right-handed angle (radians, -PI..PI) of a point projected onto the
    /// handle plane, measured from the zero-angle radius.
    pub fn angle_of(&self, point: Point) -> Option<f64> {
        let p = sub(point, self.center);
        let p = sub(p, mul(self.axis, dot(p, self.axis)));
        if dot(p, p) < 1e-12 {
            return None;
        }
        Some(dot(self.axis, cross(self.radius, p)).atan2(dot(self.radius, p)))
    }
}

/// Representative profile point: the mean of the selected boundary curve
/// points, or of all sketch points when no curve identities match.
pub fn anchor(points: &[(EntityId, [f64; 2])], selected: &[EntityId]) -> Option<[f64; 2]> {
    let mean = |filter: &dyn Fn(&EntityId) -> bool| {
        let chosen: Vec<_> = points.iter().filter(|(id, _)| filter(id)).collect();
        (!chosen.is_empty()).then(|| {
            let n = chosen.len() as f64;
            [
                chosen.iter().map(|(_, p)| p[0]).sum::<f64>() / n,
                chosen.iter().map(|(_, p)| p[1]).sum::<f64>() / n,
            ]
        })
    };
    mean(&|id| selected.contains(id)).or_else(|| mean(&|_| true))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn guide() -> ProfileGuide {
        ProfileGuide {
            sketch: EntityId::new(),
            frame: Frame {
                origin: [0., 0., 0.],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                n: [0., 0., 1.],
            },
            anchor: [20., 10.],
            lines: vec![],
        }
    }
    fn near(a: Point, b: Point) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-9, "{a:?} != {b:?}");
        }
    }
    #[test]
    fn extrusion_arrow_starts_on_the_offset_plane_at_the_profile() {
        let (start, direction) = extrude_axis(&guide(), [0., 0., 1.], 4.).unwrap();
        near(start, [20., 10., 4.]);
        near(direction, [0., 0., 1.]);
        assert!(extrude_axis(&guide(), [0., 0., 0.], 0.).is_none());
    }
    #[test]
    fn revolve_ring_rotates_right_handed_about_the_chosen_axis() {
        // Axis along sketch V through the origin; anchor at U=20.
        let ring = RevolveRing::new(&guide(), [0., 0.], std::f64::consts::FRAC_PI_2, 10.).unwrap();
        near(ring.center, [0., 10., 0.]);
        near(ring.point(0.), [20., 10., 0.]);
        // +90° about +Y takes +X to -Z.
        near(ring.point(90.), [0., 10., -20.]);
        let angle = ring.angle_of([0., 10., -20.]).unwrap();
        assert!((angle.to_degrees() - 90.).abs() < 1e-9);
        // Profile on the axis falls back to the sketch normal radius.
        let mut centered = guide();
        centered.anchor = [0., 5.];
        let ring = RevolveRing::new(&centered, [0., 0.], std::f64::consts::FRAC_PI_2, 10.).unwrap();
        near(ring.radius, [0., 0., 10.]);
    }
    #[test]
    fn anchor_prefers_selected_curves() {
        let a = EntityId::new();
        let b = EntityId::new();
        let points = [(a, [0., 0.]), (a, [10., 0.]), (b, [100., 100.])];
        assert_eq!(anchor(&points, &[a]), Some([5., 0.]));
        assert_eq!(anchor(&points, &[EntityId::new()]).unwrap()[0], 110. / 3.);
        assert_eq!(anchor(&[], &[a]), None);
    }
}
