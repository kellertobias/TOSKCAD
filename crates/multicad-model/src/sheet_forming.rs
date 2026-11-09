//! Sheet-metal forming geometry: flange cross-sections built from straight
//! and bent strip segments, their placement on base edges, reliefs and
//! cutouts in folded coordinates, and the matching developed regions.
//!
//! A flange cross-section lies in the plane spanned by the edge's outward
//! direction `o` (s) and the thickness direction `d` (w). The base occupies
//! s ≤ 0, 0 ≤ w ≤ t. Side A of the strip starts on the base face (w = 0),
//! side B on the opposite face (w = t).
use multicad_core::{
    document::EntityId,
    model::{CutoutFace, ResolvedCutout, ResolvedFlange, ResolvedShape, StripSegment},
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid,
};

const EPS: f64 = 1e-9;

fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: [f64; 2], s: f64) -> [f64; 2] {
    [a[0] * s, a[1] * s]
}
fn rotate(a: [f64; 2], angle: f64) -> [f64; 2] {
    let (s, c) = angle.sin_cos();
    [a[0] * c - a[1] * s, a[0] * s + a[1] * c]
}
fn about(p: [f64; 2], center: [f64; 2], angle: f64) -> [f64; 2] {
    add(center, rotate([p[0] - center[0], p[1] - center[1]], angle))
}

/// The strip state at a point along side A: position, tangent and the
/// normal from side A toward side B.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripState {
    pub point: [f64; 2],
    pub tangent: [f64; 2],
    pub normal: [f64; 2],
}
/// A flange cross-section: its closed profile edges with provenance roles,
/// and the strip state where the final leg starts.
pub struct Section {
    pub edges: Vec<CurvedEdge2d>,
    pub roles: Vec<String>,
    pub leg_start: StripState,
}
pub fn section(flange: &ResolvedFlange, thickness: f64) -> Section {
    let t = thickness;
    let mut state = StripState {
        point: [0., 0.],
        tangent: [1., 0.],
        normal: [0., 1.],
    };
    let mut a_side: Vec<(CurvedEdge2d, String)> = vec![];
    let mut b_side: Vec<(CurvedEdge2d, String)> = vec![];
    let mut leg_start = state;
    let last = flange.segments.len().saturating_sub(1);
    let mut bend = 0;
    for (j, segment) in flange.segments.iter().enumerate() {
        if j == last {
            leg_start = state;
        }
        let i = flange.index;
        match *segment {
            StripSegment::Straight { length_mm } => {
                if length_mm <= EPS {
                    continue;
                }
                let pa = state.point;
                let pb = add(pa, mul(state.normal, t));
                let step = mul(state.tangent, length_mm);
                let name = if j == last {
                    "leg".to_string()
                } else {
                    format!("straight.{j}")
                };
                a_side.push((
                    CurvedEdge2d::Line {
                        a: pa,
                        b: add(pa, step),
                    },
                    format!("sheet.flange.{i}.{name}.a"),
                ));
                b_side.push((
                    CurvedEdge2d::Line {
                        a: pb,
                        b: add(pb, step),
                    },
                    format!("sheet.flange.{i}.{name}.b"),
                ));
                state.point = add(pa, step);
            }
            StripSegment::Bend {
                angle_rad,
                radius_mm,
            } => {
                let r = radius_mm;
                let pa = state.point;
                let pb = add(pa, mul(state.normal, t));
                // Positive bends turn toward side B, which is then inside.
                let center = if angle_rad > 0. {
                    add(pa, mul(state.normal, t + r))
                } else {
                    add(pa, mul(state.normal, -r))
                };
                let clockwise = angle_rad < 0.;
                let qa = about(pa, center, angle_rad);
                let qb = about(pb, center, angle_rad);
                for (side, p, q) in [(&mut a_side, pa, qa), (&mut b_side, pb, qb)] {
                    let radius = ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt();
                    if radius > EPS {
                        side.push((
                            CurvedEdge2d::Arc {
                                center,
                                a: p,
                                b: q,
                                clockwise,
                            },
                            String::new(),
                        ));
                    }
                }
                if let Some(last) = a_side.last_mut().filter(|e| e.1.is_empty()) {
                    last.1 = format!("sheet.flange.{i}.bend.{bend}.a");
                }
                if let Some(last) = b_side.last_mut().filter(|e| e.1.is_empty()) {
                    last.1 = format!("sheet.flange.{i}.bend.{bend}.b");
                }
                bend += 1;
                state.point = qa;
                state.tangent = rotate(state.tangent, angle_rad);
                state.normal = rotate(state.normal, angle_rad);
            }
        }
    }
    let i = flange.index;
    let end_a = state.point;
    let end_b = add(end_a, mul(state.normal, t));
    let mut edges = vec![];
    let mut roles = vec![];
    for (e, r) in a_side {
        edges.push(e);
        roles.push(r);
    }
    edges.push(CurvedEdge2d::Line { a: end_a, b: end_b });
    roles.push(format!("sheet.flange.{i}.end"));
    for (e, r) in b_side.into_iter().rev() {
        edges.push(reversed(&e));
        roles.push(r);
    }
    edges.push(CurvedEdge2d::Line {
        a: [0., t],
        b: [0., 0.],
    });
    roles.push(format!("sheet.flange.{i}.root"));
    Section {
        edges,
        roles,
        leg_start,
    }
}
fn reversed(e: &CurvedEdge2d) -> CurvedEdge2d {
    match e {
        CurvedEdge2d::Line { a, b } => CurvedEdge2d::Line { a: *b, b: *a },
        CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        } => CurvedEdge2d::Arc {
            center: *center,
            a: *b,
            b: *a,
            clockwise: !clockwise,
        },
        other => other.clone(),
    }
}

/// A flange placed on a straight base edge, in base-frame (u, v)
/// coordinates: it starts at `start`, runs `width` along `along` and bends
/// outward along `outward`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedFlange {
    pub flange: ResolvedFlange,
    pub start: [f64; 2],
    pub along: [f64; 2],
    pub outward: [f64; 2],
    pub width: f64,
    pub edge_length: f64,
}
/// World geometry shared by every forming step of one sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetSpace {
    pub frame: PlaneFrame,
    pub thickness_direction: [f64; 3],
    pub thickness_mm: f64,
}
impl SheetSpace {
    fn world(&self, uv: [f64; 2]) -> [f64; 3] {
        self.frame.point(uv)
    }
    fn vector(&self, uv: [f64; 2]) -> [f64; 3] {
        std::array::from_fn(|i| self.frame.u[i] * uv[0] + self.frame.v[i] * uv[1])
    }
    /// A point in a flange's cross-section plane at `along` from its start.
    fn section_point(&self, f: &PlacedFlange, along: f64, sw: [f64; 2]) -> [f64; 3] {
        let base = self.world(add(
            add(f.start, mul(f.along, along)),
            mul(f.outward, sw[0]),
        ));
        std::array::from_fn(|i| base[i] + self.thickness_direction[i] * sw[1])
    }
    fn section_vector(&self, f: &PlacedFlange, sw: [f64; 2]) -> [f64; 3] {
        let o = self.vector(f.outward);
        std::array::from_fn(|i| o[i] * sw[0] + self.thickness_direction[i] * sw[1])
    }
    /// The solid of one flange: its cross-section swept along the edge.
    pub fn flange_solid(
        &self,
        f: &PlacedFlange,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Solid, String> {
        let s = section(&f.flange, self.thickness_mm);
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: self.world(f.start),
                u: self.vector(f.outward),
                v: self.thickness_direction,
            },
            outer: s.edges,
            holes: vec![],
        };
        let origins: Vec<_> = s
            .roles
            .into_iter()
            .map(|role| Origin {
                source: feature,
                role,
            })
            .collect();
        Solid::extrude_curved_profile_vector_named(
            &profile,
            self.vector(mul(f.along, f.width)),
            feature,
            &origins,
            policy,
        )
        .map_err(|e| format!("Flange {}: {e}", f.flange.index + 1))
    }
    /// A through-thickness prism of `outline` drawn in a face frame.
    fn prism(
        &self,
        frame: PlaneFrame,
        outline: Vec<CurvedEdge2d>,
        through: [f64; 3],
        policy: GeometryPolicy,
    ) -> Result<Solid, String> {
        Solid::extrude_curved_profile_vector(
            &CurvedProfile {
                frame,
                outer: outline,
                holes: vec![],
            },
            through,
            policy,
        )
        .map_err(|e| e.to_string())
    }
    /// Relief prisms cut into the base at each inset end of a flange.
    pub fn reliefs(&self, f: &PlacedFlange) -> Vec<[[f64; 2]; 4]> {
        let Some([w, depth]) = f.flange.relief_mm else {
            return vec![];
        };
        let mut out = vec![];
        let corner = |a0: f64, a1: f64| {
            let p = |a: f64, o: f64| add(add(f.start, mul(f.along, a)), mul(f.outward, o));
            [p(a0, -depth), p(a1, -depth), p(a1, 0.), p(a0, 0.)]
        };
        if f.flange.start_inset_mm > EPS {
            out.push(corner(-w, 0.));
        }
        if f.flange.end_inset_mm > EPS {
            out.push(corner(f.width, f.width + w));
        }
        out
    }
    /// A base-frame polygon as a through-thickness prism.
    pub fn base_prism(
        &self,
        polygon: &[[f64; 2]],
        policy: GeometryPolicy,
    ) -> Result<Solid, String> {
        self.prism(
            self.frame,
            polygon_edges(polygon),
            self.thickness_direction.map(|c| c * self.thickness_mm),
            policy,
        )
    }
    /// A face's frame on its side A, and the unit normal toward side B.
    /// Base coordinates are (u, v); flange leg coordinates run along the
    /// edge and along the leg.
    pub fn face_frame(
        &self,
        face: CutoutFace,
        flanges: &[PlacedFlange],
    ) -> Result<(PlaneFrame, [f64; 3]), String> {
        match face {
            CutoutFace::Base => Ok((self.frame, self.thickness_direction)),
            CutoutFace::Flange { index } => {
                let f = flanges
                    .iter()
                    .find(|f| f.flange.index == index as usize)
                    .ok_or_else(|| format!("Flange {} does not exist", index + 1))?;
                let leg = section(&f.flange, self.thickness_mm).leg_start;
                Ok((
                    PlaneFrame {
                        origin: self.section_point(f, 0., leg.point),
                        u: self.vector(f.along),
                        v: self.section_vector(f, leg.tangent),
                    },
                    self.section_vector(f, leg.normal),
                ))
            }
        }
    }
    /// The cutting prism of a cutout in folded coordinates.
    pub fn cutout_solid(
        &self,
        feature: EntityId,
        cutout: &ResolvedCutout,
        flanges: &[PlacedFlange],
        policy: GeometryPolicy,
    ) -> Result<Solid, String> {
        let (frame, normal) = self.face_frame(cutout.face, flanges)?;
        let outer = shape_edges(&cutout.shape);
        let name = cutout.source.id();
        let origins: Vec<_> = (0..outer.len())
            .map(|index| Origin {
                source: feature,
                role: format!("sheet.{name}.edge.{index}"),
            })
            .collect();
        Solid::extrude_curved_profile_vector_named(
            &CurvedProfile {
                frame,
                outer,
                holes: vec![],
            },
            normal.map(|c| c * self.thickness_mm),
            feature,
            &origins,
            policy,
        )
        .map_err(|e| e.to_string())
    }
}
pub fn polygon_edges(points: &[[f64; 2]]) -> Vec<CurvedEdge2d> {
    (0..points.len())
        .map(|i| CurvedEdge2d::Line {
            a: points[i],
            b: points[(i + 1) % points.len()],
        })
        .collect()
}
pub fn shape_edges(shape: &ResolvedShape) -> Vec<CurvedEdge2d> {
    match *shape {
        ResolvedShape::Circle {
            center,
            diameter_mm,
        } => vec![CurvedEdge2d::Circle {
            center,
            radius_mm: diameter_mm / 2.,
        }],
        ResolvedShape::Rectangle { center, size_mm } => {
            let [hx, hy] = [size_mm[0] / 2., size_mm[1] / 2.];
            polygon_edges(&[
                [center[0] - hx, center[1] - hy],
                [center[0] + hx, center[1] - hy],
                [center[0] + hx, center[1] + hy],
                [center[0] - hx, center[1] + hy],
            ])
        }
    }
}
/// Whether two flange solids overlap or touch; either leaves invalid
/// material at a corner or seam.
pub fn interfere(
    a: &Solid,
    b: &Solid,
    policy: GeometryPolicy,
) -> Result<Option<&'static str>, String> {
    let inspection = a.inspect_pair(b, 1e-3, policy).map_err(|e| e.to_string())?;
    Ok(match inspection.relation {
        multicad_kernel::SolidPairRelation::Overlapping => Some("overlap"),
        multicad_kernel::SolidPairRelation::ContactWithinTolerance => Some("touch"),
        multicad_kernel::SolidPairRelation::Separated => None,
    })
}
/// Union every solid into `acc`.
pub fn fuse(mut acc: Solid, others: &[Solid], policy: GeometryPolicy) -> Result<Solid, String> {
    for s in others {
        acc = acc
            .boolean(s, BooleanOperation::Union, policy)
            .map_err(|e| e.to_string())?
            .ok_or("A union removed all material")?;
    }
    Ok(acc)
}
/// A planar face's exact isometry into developed sheet coordinates.
#[derive(Clone, Copy)]
pub(crate) struct FlatFrame {
    origin: [f64; 2],
    u: [f64; 2],
    v: [f64; 2],
}
impl FlatFrame {
    pub const BASE: Self = Self {
        origin: [0., 0.],
        u: [1., 0.],
        v: [0., 1.],
    };
    pub fn flange(flange: &PlacedFlange, offset: f64) -> Self {
        Self {
            origin: add(flange.start, mul(flange.outward, offset)),
            u: flange.along,
            v: flange.outward,
        }
    }
    pub fn point(self, [x, y]: [f64; 2]) -> [f64; 2] {
        add(add(self.origin, mul(self.u, x)), mul(self.v, y))
    }
    pub fn coordinates(self, point: [f64; 2]) -> [f64; 2] {
        let delta = std::array::from_fn::<_, 2, _>(|i| point[i] - self.origin[i]);
        [self.u, self.v].map(|axis| axis[0] * delta[0] + axis[1] * delta[1])
    }
    pub fn reflected(self) -> bool {
        self.u[0] * self.v[1] - self.u[1] * self.v[0] < 0.
    }
    pub fn quad(self, width: f64, length: f64) -> [[f64; 2]; 4] {
        [[0., 0.], [width, 0.], [width, length], [0., length]].map(|p| self.point(p))
    }
}

/// The developed region of a flange in base-frame coordinates.
pub fn developed_rect(f: &PlacedFlange, developed: f64) -> [[f64; 2]; 4] {
    FlatFrame::flange(f, 0.).quad(f.width, developed)
}
/// A cutout shape moved into flat base-frame coordinates.
pub fn flat_cutout(
    cutout: &ResolvedCutout,
    flanges: &[PlacedFlange],
    thickness: f64,
    k: f64,
) -> Option<Vec<CurvedEdge2d>> {
    match cutout.face {
        CutoutFace::Base => Some(shape_edges(&cutout.shape)),
        CutoutFace::Flange { index } => {
            let f = flanges.iter().find(|f| f.flange.index == index as usize)?;
            let offset = f.flange.leg_offset_mm(thickness, k);
            let frame = FlatFrame::flange(f, offset);
            let map = |p| frame.point(p);
            Some(match cutout.shape {
                ResolvedShape::Circle {
                    center,
                    diameter_mm,
                } => vec![CurvedEdge2d::Circle {
                    center: map(center),
                    radius_mm: diameter_mm / 2.,
                }],
                ResolvedShape::Rectangle { center, size_mm } => {
                    let [hx, hy] = [size_mm[0] / 2., size_mm[1] / 2.];
                    polygon_edges(&[
                        map([center[0] - hx, center[1] - hy]),
                        map([center[0] + hx, center[1] - hy]),
                        map([center[0] + hx, center[1] + hy]),
                        map([center[0] - hx, center[1] + hy]),
                    ])
                }
            })
        }
    }
}
