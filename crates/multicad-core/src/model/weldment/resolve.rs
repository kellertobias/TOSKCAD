//! Member resolution: the profile section in its own coordinates, the member
//! frame in world millimeters, and the placed section. Section coordinates
//! put the bounding box at [0, width] × [0, height]; an angle's heel is the
//! origin with legs along +x and +y.
use super::*;
use crate::{
    materials::ProfileShape, model::common::number, parameters::EvaluatedParameters,
    references::ReferenceFrame,
};

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

/// One exact segment of a section boundary, in section millimeters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SectionEdge {
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    Arc {
        center: [f64; 2],
        a: [f64; 2],
        b: [f64; 2],
    },
}

/// One closed section boundary in section coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionLoop {
    /// Counterclockwise vertices.
    Polygon(Vec<[f64; 2]>),
    /// Exact lines and counterclockwise circular arcs.
    Curved(Vec<SectionEdge>),
    Circle {
        center: [f64; 2],
        radius_mm: f64,
    },
}
/// A stable side of the profile cross-section, independent of member rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileGrooveSide {
    MinX,
    MaxX,
    MinY,
    MaxY,
}
impl ProfileGrooveSide {
    pub const ALL: [Self; 4] = [Self::MinX, Self::MaxX, Self::MinY, Self::MaxY];

    pub fn token(self) -> &'static str {
        match self {
            Self::MinX => "min_x",
            Self::MaxX => "max_x",
            Self::MinY => "min_y",
            Self::MaxY => "max_y",
        }
    }

    /// Identify a current native face by its semantic extrusion provenance.
    pub fn from_member_role(role: &str) -> Option<Self> {
        let (side, _) = role.strip_prefix("member.groove.")?.split_once('.')?;
        Self::ALL.into_iter().find(|s| s.token() == side)
    }
}

pub const ITEM_LINE_8_WIDTH_MM: f64 = 40.;
pub const ITEM_LINE_8_CORE_DIAMETER_MM: f64 = 6.8;
pub const ITEM_LINE_8_GROOVE_MOUTH_MM: f64 = 8.;
pub const ITEM_LINE_8_GROOVE_LIP_DEPTH_MM: f64 = 4.5;
pub const ITEM_LINE_8_GROOVE_DEPTH_MM: f64 = 12.25;

const ITEM_LINE_8_CORNER_RADIUS_MM: f64 = 4.;
// The item 0.0.026.03 sheet (status 18.07.25) dimensions the outer
// section, core, mouth and depths. These independent taper targets
// approximate the undimensioned groove flanks and omit internal fillets.
// (Half-width from groove center, depth from the outer face.)
const ITEM_LINE_8_GROOVE_FLANK: [[f64; 2]; 4] = [
    [9.25, ITEM_LINE_8_GROOVE_LIP_DEPTH_MM],
    [7.75, 8.],
    [5.75, 10.],
    [2.75, ITEM_LINE_8_GROOVE_DEPTH_MM],
];
const ITEM_LINE_8_EDGES_PER_SIDE: usize = 6 + 2 * ITEM_LINE_8_GROOVE_FLANK.len();
const ITEM_LINE_8_SIDES: [ProfileGrooveSide; 4] = [
    ProfileGrooveSide::MinY,
    ProfileGrooveSide::MaxX,
    ProfileGrooveSide::MaxY,
    ProfileGrooveSide::MinX,
];

fn item_line_8_outer() -> Vec<SectionEdge> {
    let w = ITEM_LINE_8_WIDTH_MM;
    let c = w / 2.;
    let r = ITEM_LINE_8_CORNER_RADIUS_MM;
    let m = ITEM_LINE_8_GROOVE_MOUTH_MM / 2.;
    let lip = ITEM_LINE_8_GROOVE_LIP_DEPTH_MM;
    let mut side = vec![[r, 0.], [c - m, 0.], [c - m, lip]];
    side.extend(ITEM_LINE_8_GROOVE_FLANK.map(|[half, depth]| [c - half, depth]));
    side.extend(
        ITEM_LINE_8_GROOVE_FLANK
            .iter()
            .rev()
            .map(|&[half, depth]| [c + half, depth]),
    );
    side.extend([[c + m, lip], [c + m, 0.], [w - r, 0.]]);
    let rotate = |[x, y]: [f64; 2]| [w - y, x];
    let mut center = [w - r, r];
    let mut edges = Vec::with_capacity(4 * ITEM_LINE_8_EDGES_PER_SIDE);
    for _ in ITEM_LINE_8_SIDES {
        edges.extend(
            side.windows(2)
                .map(|p| SectionEdge::Line { a: p[0], b: p[1] }),
        );
        edges.push(SectionEdge::Arc {
            center,
            a: side[side.len() - 1],
            b: rotate(side[0]),
        });
        for point in &mut side {
            *point = rotate(*point);
        }
        center = rotate(center);
    }
    edges
}

fn item_line_8_area() -> f64 {
    let r = ITEM_LINE_8_CORNER_RADIUS_MM;
    let core = ITEM_LINE_8_CORE_DIAMETER_MM / 2.;
    let groove = ITEM_LINE_8_GROOVE_MOUTH_MM * ITEM_LINE_8_GROOVE_LIP_DEPTH_MM
        + ITEM_LINE_8_GROOVE_FLANK
            .windows(2)
            .map(|p| {
                let [a, b] = [p[0], p[1]];
                (a[0] + b[0]) * (b[1] - a[1])
            })
            .sum::<f64>();
    ITEM_LINE_8_WIDTH_MM.powi(2)
        - (4. - std::f64::consts::PI) * r * r
        - std::f64::consts::PI * core * core
        - 4. * groove
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
    pub fn is_item_line_8_40x40(&self) -> bool {
        self.shape == ProfileShape::ItemLine8_40x40
    }

    pub fn is_hollow(&self) -> bool {
        self.is_item_line_8_40x40() || (self.wall_mm.is_some() && self.shape != ProfileShape::Angle)
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
            ProfileShape::ItemLine8_40x40 if g == 0. => SectionLoop::Curved(item_line_8_outer()),
            // A conservative outside cover. The native evaluator refuses
            // weld/cope operations on the slotted mounting section.
            ProfileShape::ItemLine8_40x40 => {
                SectionLoop::Polygon(vec![[-g, -g], [w + g, -g], [w + g, h + g], [-g, h + g]])
            }
        }
    }

    /// Preserve semantic groove and lip identities instead of native face indices.
    pub fn slotted_outer_role(&self, edge: usize) -> Option<String> {
        if !self.is_item_line_8_40x40() {
            return None;
        }
        let side = ITEM_LINE_8_SIDES
            .get(edge / ITEM_LINE_8_EDGES_PER_SIDE)?
            .token();
        let segment = edge % ITEM_LINE_8_EDGES_PER_SIDE;
        let role = match segment {
            0 => "land_before".to_owned(),
            1 => "mouth_before".to_owned(),
            2 => "retention_before".to_owned(),
            6 => "floor".to_owned(),
            10 => "retention_after".to_owned(),
            11 => "mouth_after".to_owned(),
            12 => "land_after".to_owned(),
            13 => return Some(format!("corner_after.{side}")),
            _ => format!("flank.{segment}"),
        };
        Some(format!("groove.{side}.{role}"))
    }
    /// Inner boundaries of a hollow section.
    pub fn holes(&self) -> Vec<SectionLoop> {
        if self.is_item_line_8_40x40() {
            let center = [ITEM_LINE_8_WIDTH_MM / 2.; 2];
            return vec![SectionLoop::Circle {
                center,
                radius_mm: ITEM_LINE_8_CORE_DIAMETER_MM / 2.,
            }];
        }
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
            ProfileShape::Angle | ProfileShape::ItemLine8_40x40 => vec![],
        }
    }
    /// Exact cross-section area of the modeled section.
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
            (ProfileShape::ItemLine8_40x40, _) => item_line_8_area(),
        }
    }
    /// Points describing the outside shape: polygon vertices, or the four
    /// quadrant points and center of a circle. Used to compare placed
    /// sections, never as geometry.
    pub fn outline_points(&self) -> Vec<[f64; 2]> {
        match self.outer(0.) {
            SectionLoop::Polygon(points) => points,
            SectionLoop::Curved(edges) => edges
                .into_iter()
                .map(|edge| match edge {
                    SectionEdge::Line { a, .. } | SectionEdge::Arc { a, .. } => a,
                })
                .collect(),
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
            (ProfileShape::ItemLine8_40x40, _) => "item Line 8 40×40 · four open grooves",
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

    /// Groove mouth frame at a distance from the current extended start.
    /// U runs along the member, V completes a right-handed frame, and N
    /// points outward. The host occurrence pose is applied separately.
    pub fn groove_frame(
        &self,
        side: ProfileGrooveSide,
        station_mm: f64,
    ) -> Result<ReferenceFrame, DocumentError> {
        if !self.section.is_item_line_8_40x40() {
            return Err(invalid("Choose a compatible item Line 8 slotted member"));
        }
        if !station_mm.is_finite() || !(0. ..=self.length_mm()).contains(&station_mm) {
            return Err(invalid(
                "The groove station falls outside the current member span",
            ));
        }
        let w = ITEM_LINE_8_WIDTH_MM;
        let h = w / 2.;
        let (point, normal) = match side {
            ProfileGrooveSide::MinX => ([0., h], scale(self.frame.x, -1.)),
            ProfileGrooveSide::MaxX => ([w, h], self.frame.x),
            ProfileGrooveSide::MinY => ([h, 0.], scale(self.frame.y, -1.)),
            ProfileGrooveSide::MaxY => ([h, w], self.frame.y),
        };
        Ok(ReferenceFrame {
            origin_mm: self.point(point, self.start_offset_mm() + station_mm),
            u: self.frame.axis,
            v: cross(normal, self.frame.axis),
        })
    }

    /// Actual modeled end plane at the central bore, with outward N.
    pub fn slotted_end_frame(&self, end: MemberEnd) -> Result<ReferenceFrame, DocumentError> {
        if !self.section.is_item_line_8_40x40() {
            return Err(invalid("Choose a compatible item Line 8 slotted member"));
        }
        let (station, sign) = match end {
            MemberEnd::Start => (self.start_offset_mm(), -1.),
            MemberEnd::End => (self.start_offset_mm() + self.length_mm(), 1.),
        };
        let h = ITEM_LINE_8_WIDTH_MM / 2.;
        Ok(ReferenceFrame {
            origin_mm: self.point([h, h], station),
            u: self.frame.x,
            v: scale(self.frame.y, sign),
        })
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
    let member_length = path_length_mm + extend_mm[0] + extend_mm[1];
    if member_length < PATH_TOLERANCE_MM {
        return Err(invalid("The end extensions trim away the whole member"));
    }
    if section.is_item_line_8_40x40() && member_length > section.stock_length_mm + PATH_TOLERANCE_MM
    {
        return Err(invalid(
            "The item Line 8 member exceeds the selected stock length",
        ));
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
