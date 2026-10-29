//! Member resolution: the profile section in its own coordinates, the member
//! frame in world millimeters, and the placed section. Section coordinates
//! put the bounding box at [0, width] × [0, height]; an angle's heel is the
//! origin with legs along +x and +y.
use super::*;
use crate::{materials::ProfileShape, model::common::number, parameters::EvaluatedParameters};

pub(crate) const PATH_TOLERANCE_MM: f64 = 1e-3;

pub(crate) mod vec3 {
    pub fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    pub fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    pub fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
        [a[0] * s, a[1] * s, a[2] * s]
    }
    pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    pub fn norm(a: [f64; 3]) -> f64 {
        dot(a, a).sqrt()
    }
    pub fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
        let n = norm(a);
        (n > 1e-12).then(|| scale(a, 1. / n))
    }
    pub fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
        norm(sub(a, b))
    }
}
use vec3::*;

/// One closed section boundary in section coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionLoop {
    /// Counterclockwise vertices.
    Polygon(Vec<[f64; 2]>),
    Circle {
        center: [f64; 2],
        radius_mm: f64,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileSection {
    pub designation: String,
    pub shape: ProfileShape,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Tube wall, or angle leg thickness; none for a solid bar.
    pub wall_mm: Option<f64>,
    /// Bar length the stock is supplied in.
    pub stock_length_mm: f64,
}
impl ProfileSection {
    pub fn from_stock(stock: &Stock) -> Result<Self, DocumentError> {
        let StockKind::Profile {
            designation,
            length_mm,
            width_mm,
            height_mm,
            wall_mm,
            shape,
        } = &stock.specification
        else {
            return Err(invalid("Members need profile stock"));
        };
        Ok(Self {
            designation: designation.clone(),
            shape: *shape,
            width_mm: *width_mm,
            height_mm: *height_mm,
            wall_mm: *wall_mm,
            stock_length_mm: *length_mm,
        })
    }
    pub fn is_hollow(&self) -> bool {
        self.wall_mm.is_some() && self.shape != ProfileShape::Angle
    }
    /// The outside boundary, grown outward by `grow_mm` (0 for the section
    /// itself; a positive value for a clearance envelope).
    pub fn outer(&self, grow_mm: f64) -> SectionLoop {
        let (w, h, g) = (self.width_mm, self.height_mm, grow_mm);
        match self.shape {
            ProfileShape::Rectangular => {
                SectionLoop::Polygon(vec![[-g, -g], [w + g, -g], [w + g, h + g], [-g, h + g]])
            }
            ProfileShape::Round => SectionLoop::Circle {
                center: [w / 2., h / 2.],
                radius_mm: w / 2. + g,
            },
            ProfileShape::Angle => {
                let t = self.wall_mm.unwrap_or(0.);
                SectionLoop::Polygon(vec![
                    [-g, -g],
                    [w + g, -g],
                    [w + g, t + g],
                    [t + g, t + g],
                    [t + g, h + g],
                    [-g, h + g],
                ])
            }
        }
    }
    /// Inner boundaries of a hollow section.
    pub fn holes(&self) -> Vec<SectionLoop> {
        let Some(t) = self.wall_mm.filter(|_| self.is_hollow()) else {
            return vec![];
        };
        let (w, h) = (self.width_mm, self.height_mm);
        match self.shape {
            ProfileShape::Rectangular => vec![SectionLoop::Polygon(vec![
                [t, t],
                [w - t, t],
                [w - t, h - t],
                [t, h - t],
            ])],
            ProfileShape::Round => vec![SectionLoop::Circle {
                center: [w / 2., h / 2.],
                radius_mm: w / 2. - t,
            }],
            ProfileShape::Angle => vec![],
        }
    }
    /// Exact cross-section area of the modeled (sharp-cornered) section.
    pub fn area_mm2(&self) -> f64 {
        let (w, h) = (self.width_mm, self.height_mm);
        match (self.shape, self.wall_mm) {
            (ProfileShape::Rectangular, None) => w * h,
            (ProfileShape::Rectangular, Some(t)) => w * h - (w - 2. * t) * (h - 2. * t),
            (ProfileShape::Round, None) => std::f64::consts::PI * w * w / 4.,
            (ProfileShape::Round, Some(t)) => {
                let r = w / 2.;
                std::f64::consts::PI * (r * r - (r - t) * (r - t))
            }
            (ProfileShape::Angle, t) => {
                let t = t.unwrap_or(0.);
                w * t + (h - t) * t
            }
        }
    }
    /// Points describing the outside shape: polygon vertices, or the four
    /// quadrant points and center of a circle. Used to compare placed
    /// sections, never as geometry.
    pub fn outline_points(&self) -> Vec<[f64; 2]> {
        match self.outer(0.) {
            SectionLoop::Polygon(points) => points,
            SectionLoop::Circle { center, radius_mm } => vec![
                center,
                [center[0] + radius_mm, center[1]],
                [center[0], center[1] + radius_mm],
                [center[0] - radius_mm, center[1]],
                [center[0], center[1] - radius_mm],
            ],
        }
    }
    pub fn label(&self) -> String {
        let shape = match (self.shape, self.wall_mm) {
            (ProfileShape::Rectangular, Some(_)) => "rectangular tube",
            (ProfileShape::Rectangular, None) => "bar",
            (ProfileShape::Round, Some(_)) => "round tube",
            (ProfileShape::Round, None) => "round bar",
            (ProfileShape::Angle, _) => "angle",
        };
        format!("{} ({shape})", self.designation)
    }
}

/// A resolved straight path segment in world millimeters, with the
/// reference direction for the section height when the path supplies one
/// (a sketch normal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathSegment {
    pub start: [f64; 3],
    pub end: [f64; 3],
    pub reference: Option<[f64; 3]>,
}
/// Member frame: `axis` runs along the path, `y` is the rotated section
/// height direction and `x = y × axis` the rotated width direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemberFrame {
    pub axis: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMember {
    pub stock: EntityId,
    pub stock_spec: Stock,
    pub section: ProfileSection,
    pub path: PathSegment,
    pub path_length_mm: f64,
    pub frame: MemberFrame,
    /// Section coordinates of the point that lies on the path.
    pub anchor_mm: [f64; 2],
    pub rotation_rad: f64,
    /// Extension beyond the path start and end; negative trims.
    pub extend_mm: [f64; 2],
}
impl ResolvedMember {
    /// Nominal length: path length plus both extensions.
    pub fn length_mm(&self) -> f64 {
        self.path_length_mm + self.extend_mm[0] + self.extend_mm[1]
    }
    /// Distance along the axis from the path start to the member start.
    pub fn start_offset_mm(&self) -> f64 {
        -self.extend_mm[0]
    }
    /// World point of section coordinates `p` at axial distance `s` from the
    /// path start.
    pub fn point(&self, p: [f64; 2], s: f64) -> [f64; 3] {
        let f = &self.frame;
        add(
            add(self.path.start, scale(f.axis, s)),
            add(
                scale(f.x, p[0] - self.anchor_mm[0]),
                scale(f.y, p[1] - self.anchor_mm[1]),
            ),
        )
    }
    /// Section plane origin at axial distance `s`: the world position of
    /// section coordinate (0, 0). Section u and v are the frame x and y.
    pub fn section_origin(&self, s: f64) -> [f64; 3] {
        self.point([0., 0.], s)
    }
    /// Axial coordinate of a world point, from the path start.
    pub fn axial(&self, p: [f64; 3]) -> f64 {
        dot(sub(p, self.path.start), self.frame.axis)
    }
    /// The placed outline points at axial distance `s`.
    pub fn outline_world(&self, s: f64) -> Vec<[f64; 3]> {
        self.section
            .outline_points()
            .into_iter()
            .map(|p| self.point(p, s))
            .collect()
    }
}

/// The section height reference before rotation. A sketch normal is used
/// when the path has one; otherwise world +Z, or +Y for a vertical member.
fn reference_up(axis: [f64; 3], reference: Option<[f64; 3]>) -> [f64; 3] {
    let project = |r: [f64; 3]| unit(sub(r, scale(axis, dot(r, axis))));
    if let Some(up) = reference.and_then(project) {
        return up;
    }
    let vertical = axis[0].hypot(axis[1]) < 1e-9;
    project(if vertical { [0., 1., 0.] } else { [0., 0., 1.] }).unwrap_or([0., 1., 0.])
}
pub fn member_frame(axis: [f64; 3], reference: Option<[f64; 3]>, rotation_rad: f64) -> MemberFrame {
    let y0 = reference_up(axis, reference);
    let x0 = cross(y0, axis);
    let (s, c) = rotation_rad.sin_cos();
    MemberFrame {
        axis,
        x: add(scale(x0, c), scale(y0, s)),
        y: add(scale(x0, -s), scale(y0, c)),
    }
}

fn mm(d: &Dimension, p: &EvaluatedParameters) -> Result<f64, DocumentError> {
    number(d, Unit::Millimeter, p)
}
/// Resolve a point-pair path. Sketch-line paths are solved by the native
/// evaluator, which passes the segment to [`resolve_member`].
pub fn point_path(
    path: &MemberPath,
    parameters: &EvaluatedParameters,
) -> Result<Option<PathSegment>, DocumentError> {
    let MemberPath::Points { start, end } = path else {
        return Ok(None);
    };
    let point = |p: &[Dimension; 3]| -> Result<[f64; 3], DocumentError> {
        Ok([
            mm(&p[0], parameters)?,
            mm(&p[1], parameters)?,
            mm(&p[2], parameters)?,
        ])
    };
    Ok(Some(PathSegment {
        start: point(start)?,
        end: point(end)?,
        reference: None,
    }))
}

/// Resolve a member against its stock, parameters and solved path segment.
pub fn resolve_member(
    document: &Document,
    parameters: &EvaluatedParameters,
    member: &MemberDefinition,
    path: PathSegment,
) -> Result<ResolvedMember, DocumentError> {
    let stock_spec = read_profile_stock(document, member.stock)?;
    let section = ProfileSection::from_stock(&stock_spec)?;
    let path_length_mm = distance(path.start, path.end);
    if path_length_mm < PATH_TOLERANCE_MM {
        return Err(invalid(
            "The member path has zero length; its start and end coincide",
        ));
    }
    let axis = scale(sub(path.end, path.start), 1. / path_length_mm);
    let rotation_rad = member
        .rotation
        .as_ref()
        .map_or(Ok(0.), |r| number(r, Unit::Radian, parameters))?;
    let extend_mm = [
        member
            .extend_start
            .as_ref()
            .map_or(Ok(0.), |d| mm(d, parameters))?,
        member
            .extend_end
            .as_ref()
            .map_or(Ok(0.), |d| mm(d, parameters))?,
    ];
    if path_length_mm + extend_mm[0] + extend_mm[1] < PATH_TOLERANCE_MM {
        return Err(invalid("The end extensions trim away the whole member"));
    }
    let anchor_mm = [
        member.anchor[0].fraction() * section.width_mm,
        member.anchor[1].fraction() * section.height_mm,
    ];
    Ok(ResolvedMember {
        stock: member.stock,
        stock_spec,
        frame: member_frame(axis, path.reference, rotation_rad),
        section,
        path,
        path_length_mm,
        anchor_mm,
        rotation_rad,
        extend_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }
    #[test]
    fn frames_follow_reference_and_rotation() {
        // Horizontal along +X: height up, width toward −Y... x = Z × X = +Y.
        let f = member_frame([1., 0., 0.], None, 0.);
        assert_eq!(f.y, [0., 0., 1.]);
        assert_eq!(f.x, [0., 1., 0.]);
        // Vertical: height along +Y, width along +X.
        let f = member_frame([0., 0., 1.], None, 0.);
        assert_eq!(f.y, [0., 1., 0.]);
        assert_eq!(f.x, [1., 0., 0.]);
        // A sketch normal overrides; a quarter turn swaps the axes.
        let f = member_frame(
            [1., 0., 0.],
            Some([0., -1., 0.]),
            std::f64::consts::FRAC_PI_2,
        );
        near(f.x[1], -1.);
        near(f.y[2], -1.);
    }
    #[test]
    fn section_areas_match_their_outlines() {
        let s = |shape, w, h, wall| ProfileSection {
            designation: "x".into(),
            shape,
            width_mm: w,
            height_mm: h,
            wall_mm: wall,
            stock_length_mm: 6000.,
        };
        near(
            s(ProfileShape::Rectangular, 40., 20., Some(2.)).area_mm2(),
            800. - 36. * 16.,
        );
        near(
            s(ProfileShape::Angle, 40., 30., Some(4.)).area_mm2(),
            160. + 104.,
        );
        assert!(
            s(ProfileShape::Angle, 40., 30., Some(4.))
                .holes()
                .is_empty()
        );
        assert_eq!(s(ProfileShape::Round, 30., 30., Some(2.)).holes().len(), 1);
    }
}
