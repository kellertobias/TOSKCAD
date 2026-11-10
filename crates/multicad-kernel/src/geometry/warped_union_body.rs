//! Place a plane-and-cylinder Boolean whose straight and quarter-round bands
//! share one certified boundary in a monotone height coordinate.
use super::*;
use crate::rust_prism_topology::{Builder, position};
use multicad_core::references::{AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyKind};
use multicad_rust_kernel::{
    display::Tessellation,
    planar_boolean::{self, ConvexPiece, Union},
    warped_union::{self, AxialCut, Band, Boss, Curve, OpenBoreSide, Shell, Support},
};
use std::collections::BTreeSet;

mod axial_cut;
mod crop;
mod cut;
mod flat_stock;
mod operands;
use super::planar_boolean_body::winding;
use operands::QuarterFillet;

type Point = [f64; 3];

pub(crate) struct WarpedUnionBody {
    pub(super) feature: EntityId,
    pub(super) geometry: Shell,
    origins: Vec<Vec<Origin>>,
    anchors: Vec<Vec<AnchoredFrame>>,
    chart: Option<Chart>,
}

#[derive(Clone)]
struct SourcePart {
    operand: usize,
    faces: Vec<Option<usize>>,
}

#[derive(Clone)]
struct Chart {
    band: Option<Band>,
    stock_count: usize,
    pieces: Vec<ConvexPiece>,
    sources: Vec<SourcePart>,
    topologies: Vec<Rc<TopologySnapshot>>,
    bosses: Vec<AuthoredBoss>,
    axial: Vec<AuthoredAxialCut>,
    axial_sources: Vec<SourcePart>,
    axial_bounds: Vec<Bounds>,
}

#[derive(Clone)]
struct AuthoredBoss {
    ends: [Point; 2],
    radius: f64,
    source: SourcePart,
}

#[derive(Clone)]
struct AuthoredAxialCut {
    cut: AxialCut,
    open_side: Option<OpenBoreSide>,
}

impl AuthoredAxialCut {
    fn bounds(&self, sources: &[Bounds]) -> Bounds {
        let round = sources[self.cut.source];
        self.cut.recess.as_ref().map_or(round, |recess| {
            let wider = sources[recess.source];
            Bounds {
                min: std::array::from_fn(|i| round.min[i].min(wider.min[i])),
                max: std::array::from_fn(|i| round.max[i].max(wider.max[i])),
            }
        })
    }
}

impl Chart {
    fn new(band: Option<Band>) -> Self {
        Self {
            band,
            stock_count: 0,
            pieces: Vec::new(),
            sources: Vec::new(),
            topologies: Vec::new(),
            bosses: Vec::new(),
            axial: Vec::new(),
            axial_sources: Vec::new(),
            axial_bounds: Vec::new(),
        }
    }

    fn append(
        &mut self,
        solid: &Solid,
        fillet: Option<QuarterFillet>,
        policy: GeometryPolicy,
    ) -> Option<Result<(), GeometryError>> {
        let tolerance = policy.linear_tolerance_mm;
        let topology = match solid.rust_topology()? {
            Ok(topology) => topology,
            Err(cause) => return Some(Err(cause)),
        };
        let operand = self.topologies.len();
        if let Some(fillet) = fillet {
            let Band {
                center_z_mm: center,
                radius_mm: radius,
            } = self.band?;
            let fillet_pieces = match fillet.pieces(center, radius, tolerance) {
                Ok(fillet_pieces) => fillet_pieces,
                Err(cause) => return Some(Err(source_error(cause))),
            };
            for piece in fillet_pieces {
                self.pieces.push(piece);
                self.sources.push(SourcePart {
                    operand,
                    faces: Vec::new(),
                });
            }
        } else {
            for (piece, ancestry) in solid.planar_convex_parts(&topology, policy)? {
                if let Err(cause) = self.append_piece(piece, &ancestry, operand, policy)? {
                    return Some(Err(cause));
                }
            }
        }
        self.topologies.push(Rc::new(topology));
        Some(Ok(()))
    }

    fn append_piece(
        &mut self,
        piece: ConvexPiece,
        ancestry: &[Option<usize>],
        operand: usize,
        policy: GeometryPolicy,
    ) -> Option<Result<(), GeometryError>> {
        let Some(Band {
            center_z_mm: center,
            radius_mm: radius,
        }) = self.band
        else {
            self.pieces.push(piece);
            self.sources.push(SourcePart {
                operand,
                faces: ancestry.to_vec(),
            });
            return Some(Ok(()));
        };
        let tolerance = policy.linear_tolerance_mm;
        let floor = center - radius;
        let limit = policy.maximum_coordinate_mm;
        let levels = [-limit, floor, center, limit];
        for (i, pair) in levels.windows(2).enumerate() {
            let slab = match piece.clip_z_band([pair[0], pair[1]], tolerance) {
                Ok(Some(slab)) => slab,
                Ok(None) => continue,
                Err(cause) => return Some(Err(source_error(cause))),
            };
            let placed = match i {
                0 => slab.translate_z(floor, 0., tolerance),
                1 => slab.remap_quarter_circle(floor, radius, tolerance),
                _ => slab.translate_z(center, radius, tolerance),
            };
            let placed = match placed {
                Ok(placed) => placed,
                Err(planar_boolean::Error::Unresolved) if i == 1 => return None,
                Err(cause) => return Some(Err(source_error(cause))),
            };
            let faces = placed
                .source_faces
                .into_iter()
                .map(|i| i.and_then(|i| ancestry.get(i)).copied().flatten())
                .collect();
            self.pieces.push(placed.piece);
            self.sources.push(SourcePart { operand, faces });
        }
        Some(Ok(()))
    }

    fn append_boss(&mut self, solid: &Solid) -> Option<Result<(), GeometryError>> {
        let (cylinder, frame) = solid.axial_cylinder_frame()?;
        let topology = match solid.rust_topology()? {
            Ok(topology) => topology,
            Err(cause) => return Some(Err(cause)),
        };
        let mut faces = vec![None; 3];
        for (i, element) in topology.elements.iter().enumerate() {
            if element.key.kind == TopologyKind::Face {
                *faces.get_mut(element.key.ordinal as usize)? = Some(i);
            }
        }
        self.bosses.push(AuthoredBoss {
            ends: [
                frame.origin,
                position(frame, [0., 0., cylinder.height().as_f64()]),
            ],
            radius: cylinder.circle().radius().as_f64(),
            source: SourcePart {
                operand: self.topologies.len(),
                faces,
            },
        });
        self.topologies.push(Rc::new(topology));
        Some(Ok(()))
    }

    fn finish(
        self,
        union: Union,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Solid, GeometryError> {
        let tolerance = policy.linear_tolerance_mm;
        let mut geometry = match self.band {
            Some(band) => warped_union::remap(&union, band, tolerance),
            None => warped_union::from_planar_union(&union, tolerance),
        }
        .map_err(|cause| warped_error("curved boundary", cause))?;
        if !self.bosses.is_empty() {
            let bosses = self
                .bosses
                .iter()
                .enumerate()
                .map(|(index, boss)| {
                    boss.oriented(&geometry, self.sources.len() + index, tolerance)
                })
                .collect::<Result<Vec<_>, _>>()?;
            geometry = geometry
                .attach_bosses(&bosses, tolerance)
                .map_err(|cause| warped_error("cylindrical boss attachment", cause))?;
        }
        let offset = self.sources.len() + self.bosses.len();
        let mut cuts = Vec::new();
        let mut open = Vec::new();
        for authored in &self.axial {
            let mut cut = authored.cut.clone();
            cut.source += offset;
            if let Some(recess) = &mut cut.recess {
                recess.source += offset;
            }
            if let Some(side) = authored.open_side {
                open.push((cut, side));
            } else {
                cuts.push(cut);
            }
        }
        if !cuts.is_empty() {
            geometry = geometry
                .cut_axial(&cuts, tolerance)
                .map_err(|cause| warped_error("axial cut boundary", cause))?;
        }
        for (cut, side) in open {
            geometry = geometry
                .cut_open_axial(&cut, side, tolerance)
                .map_err(|cause| warped_error("open-sided bore", cause))?;
        }
        WarpedUnionBody::build(geometry, self, feature, policy)
    }

    fn face_source(&self, source: usize) -> Option<&SourcePart> {
        if let Some(part) = self.sources.get(source) {
            return Some(part);
        }
        let source = source - self.sources.len();
        if let Some(boss) = self.bosses.get(source) {
            return Some(&boss.source);
        }
        self.axial_sources.get(source - self.bosses.len())
    }
}

impl AuthoredBoss {
    fn bounds(&self) -> Bounds {
        let axis = subtract(self.ends[1], self.ends[0]);
        let length = norm(axis);
        let radius: Point = std::array::from_fn(|i| {
            self.radius * (axis[(i + 1) % 3] / length).hypot(axis[(i + 2) % 3] / length)
        });
        let [a, b] = self.ends;
        Bounds {
            min: std::array::from_fn(|i| a[i].min(b[i]) - radius[i]),
            max: std::array::from_fn(|i| a[i].max(b[i]) + radius[i]),
        }
    }

    fn oriented(
        &self,
        shell: &Shell,
        source: usize,
        tolerance: f64,
    ) -> Result<Boss, GeometryError> {
        let axis = subtract(self.ends[1], self.ends[0]);
        let axis = axis.map(|component| component / norm(axis));
        let mut possible = [false; 2];
        for face in shell.faces() {
            let Support::Plane { normal } = face.support else {
                continue;
            };
            let alignment = dot(normal, axis);
            if (1. - alignment.abs()).abs() > 1e-10 {
                continue;
            }
            let [a, b] = self
                .ends
                .map(|end| dot(normal, subtract(end, face.point_mm)));
            if a < -tolerance && b > tolerance {
                possible[1] = true;
            } else if b < -tolerance && a > tolerance {
                possible[0] = true;
            }
        }
        let (start, end) = match possible {
            [true, false] => (1, 0),
            [false, true] => (0, 1),
            _ => {
                return Err(warped_error(
                    "cylindrical boss direction",
                    warped_union::Error::Unsupported,
                ));
            }
        };
        Ok(Boss {
            start_mm: self.ends[start],
            end_mm: self.ends[end],
            radius_mm: self.radius,
            source,
            wall_source_face: 2,
            cap_source_face: end,
        })
    }
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn norm([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

fn rings(face: &warped_union::Face) -> impl Iterator<Item = &[warped_union::OrientedEdge]> {
    std::iter::once(face.boundary.as_slice()).chain(face.holes.iter().map(Vec::as_slice))
}

fn vector(frame: PlaneFrame, p: Point) -> Point {
    position(
        PlaneFrame {
            origin: [0.; 3],
            ..frame
        },
        p,
    )
}

fn source_error(cause: planar_boolean::Error) -> GeometryError {
    error(
        "quarter-round union",
        if cause == planar_boolean::Error::Limit {
            GeometryErrorKind::Limit
        } else {
            GeometryErrorKind::InvalidResult
        },
        format!("Cannot certify the Boolean height sections: {cause:?}"),
    )
}

fn warped_error(operation: &str, cause: warped_union::Error) -> GeometryError {
    let kind = match cause {
        warped_union::Error::Invalid => GeometryErrorKind::InvalidResult,
        warped_union::Error::Unsupported => GeometryErrorKind::Unsupported,
        warped_union::Error::Limit => GeometryErrorKind::Limit,
    };
    error(
        "quarter-round Boolean",
        kind,
        format!("Cannot certify {operation}: {cause:?}"),
    )
}

impl Solid {
    /// Join quarter-round fillets with convex planar stock whose other walls
    /// through the rounded band are vertical.
    pub fn union_quarter_rounds(
        solids: &[&Self],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        if solids.len() < 2
            || policy.boolean_fuzzy_mm != 0.
            || solids
                .iter()
                .any(|solid| solid.policy != policy || !solid.uses_rust_geometry())
            || !solids
                .iter()
                .any(|solid| solid.normal_sweep_body().is_some())
        {
            return None;
        }
        let tolerance = policy.linear_tolerance_mm;
        let fillets: Vec<_> = solids
            .iter()
            .map(|solid| QuarterFillet::recognize(solid, tolerance))
            .collect();
        let first = fillets.iter().flatten().next()?;
        let band = Band {
            center_z_mm: first.center[2],
            radius_mm: first.radius,
        };
        let [center, radius] = [band.center_z_mm, band.radius_mm];

        let roundoff = 32. * f64::EPSILON * center.abs().max(radius).max(1.);
        if fillets.iter().flatten().any(|fillet| {
            (fillet.center[2] - center).abs() > roundoff
                || (fillet.radius - radius).abs() > roundoff
        }) {
            return None;
        }
        let mut chart = Chart::new(Some(band));
        for (&solid, fillet) in solids.iter().zip(fillets) {
            if let Err(cause) = chart
                .append_boss(solid)
                .or_else(|| chart.append(solid, fillet, policy))?
            {
                return Some(Err(cause));
            }
        }
        chart.stock_count = chart.pieces.len();
        Some(
            Union::new(&chart.pieces, tolerance)
                .map_err(source_error)
                .and_then(|union| chart.finish(union, feature, policy)),
        )
    }

    pub(crate) fn warped_union_body(&self) -> Option<(&WarpedUnionBody, PlaneFrame)> {
        match &self.body {
            SolidBody::WarpedUnion(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    /// Place a certified analytic shell and preserve its authored face identities.
    pub fn rust_place_formed_sheet(
        geometry: Shell,
        frame: PlaneFrame,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if feature.is_nil() {
            return Err(input("Formed sheet needs a source feature"));
        }
        let face_origins = geometry
            .faces()
            .iter()
            .map(|face| {
                origins
                    .get(face.source_face)
                    .filter(|origin| !origin.source.is_nil() && !origin.role.trim().is_empty())
                    .map(|origin| vec![origin.clone()])
                    .ok_or_else(|| input("Formed sheet needs an authored identity for each face"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_boundary_shell(geometry, frame, feature, face_origins, policy)
    }

    pub(super) fn from_imported_warped_union(
        geometry: Shell,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let origins: Vec<_> = (0..geometry.faces().len())
            .map(|i| {
                vec![Origin {
                    source: feature,
                    role: format!("step.warped-face.{i}"),
                }]
            })
            .collect();
        Self::from_boundary_shell(geometry, frame, feature, origins, policy)
    }

    fn from_boundary_shell(
        geometry: Shell,
        frame: PlaneFrame,
        feature: EntityId,
        origins: Vec<Vec<Origin>>,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let result = Self {
            body: SolidBody::WarpedUnion(
                Rc::new(WarpedUnionBody {
                    feature,
                    geometry,
                    anchors: vec![Vec::new(); origins.len()],
                    origins,
                    chart: None,
                }),
                frame,
            ),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        };
        let bounds = result.bounds()?;
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(result)
    }
}

impl WarpedUnionBody {
    fn build(
        geometry: Shell,
        chart: Chart,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Solid, GeometryError> {
        if feature.is_nil() {
            return Err(input("Quarter-round union needs a source feature"));
        }
        let mut origins = Vec::with_capacity(geometry.faces().len());
        let mut anchors = Vec::with_capacity(geometry.faces().len());
        for (i, face) in geometry.faces().iter().enumerate() {
            let part = chart
                .face_source(face.source)
                .ok_or_else(|| warped_error("boundary provenance", warped_union::Error::Invalid))?;
            let topology = &chart.topologies[part.operand];
            let source = part
                .faces
                .get(face.source_face)
                .copied()
                .flatten()
                .map(|i| &topology.elements[i])
                .or_else(|| {
                    let mut matches = topology.elements.iter().filter(|element| {
                        element.key.kind == TopologyKind::Face
                            && match (element.geometry, face.support) {
                                (
                                    Some(AnalyticGeometry::Plane { frame }),
                                    Support::Plane { normal },
                                ) => {
                                    dot(frame.normal(), normal) > 1. - 1e-10
                                        && dot(normal, subtract(face.point_mm, frame.origin_mm))
                                            .abs()
                                            <= policy.linear_tolerance_mm
                                }
                                (None, Support::Cylinder { .. }) => true,
                                _ => false,
                            }
                    });
                    let first = matches.next()?;
                    matches.next().is_none().then_some(first)
                });
            let mut ancestry = vec![Origin {
                source: feature,
                role: format!("quarter-round.face.{i}"),
            }];
            if let Some(source) = source {
                ancestry.extend(source.origins.iter().cloned());
            }
            origins.push(ancestry);
            anchors.push(source.map_or_else(Vec::new, |source| source.frames.clone()));
        }
        for p in geometry.bounds_mm() {
            policy.point(p)?;
        }
        Ok(Solid {
            body: SolidBody::WarpedUnion(
                Rc::new(Self {
                    feature,
                    geometry,
                    origins,
                    anchors,
                    chart: Some(chart),
                }),
                PlaneFrame::default(),
            ),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        self.geometry.volume_mm3()
    }

    pub(super) fn solid_count(&self) -> u32 {
        self.geometry.solid_count() as u32
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        if frame == PlaneFrame::default() {
            let [min, max] = self.geometry.bounds_mm();
            return Bounds { min, max };
        }
        let normal = frame.normal();
        let range: [[f64; 2]; 3] = std::array::from_fn(|i| {
            let direction = [frame.u[i], frame.v[i], normal[i]];
            self.geometry
                .vertices()
                .iter()
                .map(|&p| [dot(p, direction); 2])
                .chain(self.geometry.edges().iter().filter_map(|edge| {
                    let Curve::Ellipse(arc) = edge.curve else {
                        return None;
                    };
                    Some(arc.range_along(direction))
                }))
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], [a, b]| {
                    [lo.min(a), hi.max(b)]
                })
                .map(|value| value + frame.origin[i])
        });
        Bounds {
            min: range.map(|r| r[0]),
            max: range.map(|r| r[1]),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.geometry
            .faces()
            .iter()
            .enumerate()
            .map(|(i, face)| {
                let point = position(frame, face.point_mm);
                Face {
                    index: i as u32,
                    plane: match face.support {
                        Support::Plane { normal } => Some((point, vector(frame, normal))),
                        Support::Cylinder { .. } => None,
                    },
                    center_mm: point,
                    area_mm2: face.area_mm2,
                }
            })
            .collect()
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        self.geometry
            .faces()
            .get(face as usize)
            .and_then(|face| {
                matches!(face.support, Support::Plane { .. }).then(|| {
                    rings(face)
                        .map(|ring| {
                            ring.iter()
                                .map(|use_| (use_.edge as u32, !use_.forward))
                                .collect()
                        })
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let mut builder = Builder::new(self.origins.clone());
        let mut edge_faces = vec![Vec::with_capacity(2); self.geometry.edges().len()];
        let mut vertex_faces = vec![BTreeSet::new(); self.geometry.vertices().len()];
        for (i, face) in self.geometry.faces().iter().enumerate() {
            for use_ in rings(face).flatten() {
                edge_faces[use_.edge].push(i);
                for vertex in self.geometry.edges()[use_.edge].vertices {
                    vertex_faces[vertex].insert(i);
                }
            }
        }
        for (i, edge) in self.geometry.edges().iter().enumerate() {
            let geometry = match edge.curve {
                Curve::Line => {
                    let [a_mm, b_mm] = edge
                        .vertices
                        .map(|v| position(frame, self.geometry.vertices()[v]));
                    AnalyticGeometry::Line { a_mm, b_mm }
                }
                Curve::Ellipse(arc) => {
                    let ellipse = arc.ellipse();
                    let placement = ReferenceFrame {
                        origin_mm: position(frame, ellipse.center_mm()),
                        u: vector(frame, ellipse.major_axis()),
                        v: vector(frame, ellipse.minor_axis()),
                    };
                    let [major, minor] = ellipse.semiaxes_mm();
                    if major == minor {
                        AnalyticGeometry::CircularArc {
                            frame: placement,
                            radius_mm: major,
                            start_rad: arc.start_rad(),
                            sweep_rad: arc.sweep_rad(),
                        }
                    } else {
                        AnalyticGeometry::EllipticArc {
                            frame: placement,
                            semiaxes_mm: [major, minor],
                            start_rad: arc.start_rad(),
                            sweep_rad: arc.sweep_rad(),
                        }
                    }
                }
            };
            builder.non_face(geometry, &edge_faces[i]);
        }
        for (i, &point) in self.geometry.vertices().iter().enumerate() {
            let incident: Vec<_> = vertex_faces[i].iter().copied().collect();
            builder.non_face(
                AnalyticGeometry::Point {
                    position_mm: position(frame, point),
                },
                &incident,
            );
        }
        for (i, face) in self.geometry.faces().iter().enumerate() {
            let plane = if let Support::Plane { normal } = face.support {
                let axis = (0..3)
                    .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
                    .unwrap();
                let reference = std::array::from_fn(|i| f64::from(i == axis));
                let u = cross(reference, normal);
                let n = norm(u);
                let u = u.map(|value| value / n);
                Some(ReferenceFrame {
                    origin_mm: position(frame, face.point_mm),
                    u: vector(frame, u),
                    v: vector(frame, cross(normal, u)),
                })
            } else {
                None
            };
            builder.face(
                i,
                plane,
                plane,
                rings(face)
                    .flatten()
                    .flat_map(|use_| edge_faces[use_.edge].iter().copied())
                    .filter(|&neighbor| neighbor != i),
            );
        }
        let mut elements = builder.into_elements();
        for face in elements
            .iter_mut()
            .filter(|element| element.key.kind == TopologyKind::Face)
        {
            face.frames.extend(
                self.anchors[face.key.ordinal as usize]
                    .iter()
                    .map(|anchor| AnchoredFrame {
                        origin: anchor.origin.clone(),
                        frame: ReferenceFrame {
                            origin_mm: position(frame, anchor.frame.origin_mm),
                            u: vector(frame, anchor.frame.u),
                            v: vector(frame, anchor.frame.v),
                        },
                    }),
            );
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: Tessellation,
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(self.geometry.mesh(options), frame, false)
    }
}
