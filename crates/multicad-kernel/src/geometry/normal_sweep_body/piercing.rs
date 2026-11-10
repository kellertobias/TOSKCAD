//! Analytic transverse openings through paired sides of a normal sweep.
use super::*;
use multicad_rust_kernel::normal_sweep::{PiercedSweep, Piercing};

type Point = [f64; 3];

/// A planar contour that passes through a pair of named planar sweep walls.
pub struct SweepOpening {
    pub walls: [Origin; 2],
    pub frame: PlaneFrame,
    pub through_mm: Point,
    pub outline: Vec<CurvedEdge2d>,
    pub origins: Vec<Origin>,
}

pub(super) struct Openings {
    pub(super) geometry: PiercedSweep,
    origins: Vec<Vec<Origin>>,
}

struct Open<'a> {
    spec: &'a Piercing,
    origins: &'a [Origin],
    area: f64,
    centroid: [f64; 2],
    face: usize,
    edge: u32,
}

impl Solid {
    /// Add exact through openings. Validation of the combined old and new
    /// openings is atomic and the source sweep is never modified.
    pub fn rust_pierce_normal_profile(
        &self,
        openings: &[SweepOpening],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let (body, placement) = self.normal_sweep_body().ok_or_else(|| {
            error(
                "sweep piercing",
                GeometryErrorKind::Unsupported,
                "Rust piercing requires a normal-swept solid",
            )
        })?;
        if policy != self.policy {
            return Err(input("Sweep piercing requires the source geometry policy"));
        }
        if openings.is_empty() {
            return self.share();
        }
        let mut specs = Vec::new();
        let mut origins = Vec::new();
        if let Some(previous) = &body.openings {
            specs.extend(
                previous
                    .geometry
                    .openings()
                    .iter()
                    .map(|opening| opening.spec().clone()),
            );
            origins.clone_from(&previous.origins);
        }
        let n = placement
            .normal()
            .map(|n| if body.reverse { -n } else { n });
        let local = |v| [placement.u, placement.v, n].map(|axis| dot(v, axis));
        for opening in openings {
            opening.frame.checked(policy)?;
            policy.point(opening.through_mm)?;
            let count = opening.outline.len();
            if opening.origins.len() != count
                || opening.origins.iter().any(|origin| {
                    origin.source.is_nil()
                        || origin.role.trim().is_empty()
                        || origin.role.len() > 256
                })
            {
                return Err(input(
                    "Sweep opening needs an authored origin for each contour edge",
                ));
            }
            let [a, b] = opening.walls.each_ref().map(|origin| {
                body.sides
                    .iter()
                    .position(|side| side.origin == *origin)
                    .ok_or_else(|| input("Sweep opening must reference two current planar walls"))
            });
            let mut outline = opening
                .outline
                .iter()
                .map(convert_edge)
                .collect::<Result<Vec<_>, _>>()?;
            let mut curve_origins = opening.origins.clone();
            if normal_sweep::signed_area(&outline) < 0. {
                outline.reverse();
                outline.iter_mut().for_each(|edge| *edge = edge.reversed());
                curve_origins.reverse();
            }
            specs.push(Piercing {
                sides: [a?, b?],
                origin: local(std::array::from_fn(|i| {
                    opening.frame.origin[i] - placement.origin[i]
                })),
                u: local(opening.frame.u),
                v: local(opening.frame.v),
                through: local(opening.through_mm),
                outline,
            });
            origins.push(curve_origins);
        }
        let geometry = PiercedSweep::new(
            &body.sweep,
            specs,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .map_err(|cause| {
            error(
                "sweep piercing",
                if matches!(cause, normal_sweep::Error::Limit) {
                    GeometryErrorKind::Limit
                } else {
                    GeometryErrorKind::InvalidResult
                },
                format!("Sweep openings do not fit their paired walls: {cause:?}"),
            )
        })?;
        self.with_certified_sweep_openings(geometry, origins)
    }

    pub(in crate::geometry) fn with_certified_sweep_openings(
        &self,
        geometry: PiercedSweep,
        origins: Vec<Vec<Origin>>,
    ) -> Result<Self, GeometryError> {
        let (body, placement) = self
            .normal_sweep_body()
            .ok_or_else(|| input("A certified sweep opening requires a normal-swept solid"))?;
        if geometry.sweep() != &body.sweep
            || geometry.openings().len() != origins.len()
            || geometry
                .openings()
                .iter()
                .zip(&origins)
                .any(|(opening, origins)| opening.spec().outline.len() != origins.len())
        {
            return Err(input(
                "Certified sweep openings must match the exact source stock and contour origins",
            ));
        }
        let mut unique = BTreeSet::new();
        if !origins.iter().flatten().all(|origin| unique.insert(origin)) {
            return Err(input("Sweep openings need distinct authored edge origins"));
        }
        let mut body = body.clone();
        body.openings = Some(Rc::new(Openings { geometry, origins }));
        Ok(Self {
            body: SolidBody::NormalSweep(Rc::new(body), placement),
            policy: self.policy,
            provenance: vec![],
            rust_origins: self.rust_origins.clone(),
        })
    }
}

impl NormalSweepBody {
    fn cut_iter(&self) -> impl Iterator<Item = Open<'_>> {
        let faces = self.sides.len() + 2;
        let edges = 2 * self.sides.len() as u32
            + self.sides.iter().filter(|side| side.rail.is_some()).count() as u32;
        self.openings.iter().flat_map(move |cuts| {
            cuts.geometry.openings().iter().zip(&cuts.origins).scan(
                (faces, edges),
                |(face, edge), (opening, origins)| {
                    let spec = opening.spec();
                    let result = Open {
                        spec,
                        origins,
                        area: opening.area_mm2(),
                        centroid: opening.centroid(),
                        face: *face,
                        edge: *edge,
                    };
                    let count = spec.outline.len();
                    let rails = if matches!(spec.outline.as_slice(), [Edge::Circle { .. }]) {
                        0
                    } else {
                        count
                    };
                    *face += count;
                    *edge += (2 * count + rails) as u32;
                    Some(result)
                },
            )
        })
    }

    fn cut_frame(&self, placement: PlaneFrame, spec: &Piercing, end: bool) -> ReferenceFrame {
        let convert = |[x, y, z]: Point| -> Point {
            let z = if self.reverse { -z } else { z };
            let normal = placement.normal();
            std::array::from_fn(|i| x * placement.u[i] + y * placement.v[i] + z * normal[i])
        };
        let origin = convert(std::array::from_fn(|i| {
            spec.origin[i] + if end { spec.through[i] } else { 0. }
        }));
        ReferenceFrame {
            origin_mm: std::array::from_fn(|i| origin[i] + placement.origin[i]),
            u: convert(spec.u),
            v: convert(spec.v),
        }
    }

    pub(super) fn pierced_volume(&self) -> Option<f64> {
        self.openings
            .as_ref()
            .map(|openings| openings.geometry.volume_mm3())
    }

    pub(super) fn pierced_mesh(
        &self,
        placement: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Option<Result<ffi::Mesh, GeometryError>> {
        self.openings.as_ref().map(|openings| {
            crate::rust_display::placed_mesh(
                multicad_rust_kernel::display::pierced_normal_sweep(&openings.geometry, options),
                placement,
                self.reverse,
            )
        })
    }

    pub(super) fn cut_wall_neighbors(&self, side: usize) -> impl Iterator<Item = usize> + '_ {
        self.cut_iter()
            .filter(move |opening| opening.spec.sides.contains(&side))
            .flat_map(|opening| opening.face..opening.face + opening.origins.len())
    }

    pub(super) fn cut_face_origins(&self) -> impl Iterator<Item = Vec<Origin>> + '_ {
        self.cut_iter()
            .flat_map(|opening| opening.origins.iter().map(|origin| vec![origin.clone()]))
    }

    pub(super) fn apply_cut_faces(&self, placement: PlaneFrame, faces: &mut Vec<Face>) {
        for opening in self.cut_iter() {
            let frames = [false, true].map(|end| self.cut_frame(placement, opening.spec, end));
            for (&index, frame) in opening.spec.sides.iter().zip(frames) {
                let mouth = &mut faces[index + 2];
                let center = crate::geometry::rounded_slot_body::point(frame, opening.centroid);
                mouth.area_mm2 -= opening.area;
                let ratio = opening.area / mouth.area_mm2;
                for (value, center) in mouth.center_mm.iter_mut().zip(center) {
                    *value += (*value - center) * ratio;
                }
            }
            let thickness = opening.spec.through.into_iter().fold(0_f64, f64::hypot);
            for (index, &edge) in opening.spec.outline.iter().enumerate() {
                let center = frames
                    .map(|frame| crate::geometry::rounded_slot_body::point(frame, edge.centroid()));
                let plane = cavity_plane(frames[0], edge);
                faces.push(Face {
                    index: (opening.face + index) as u32,
                    plane: plane.map(|frame| (frame.origin_mm, frame.normal())),
                    center_mm: std::array::from_fn(|i| center[0][i].midpoint(center[1][i])),
                    area_mm2: edge.length_mm() * thickness,
                });
            }
        }
    }

    pub(super) fn extra_loops(
        &self,
        face: u32,
        mut loops: Vec<Vec<(u32, bool)>>,
    ) -> Vec<Vec<(u32, bool)>> {
        for opening in self.cut_iter() {
            let count = opening.spec.outline.len() as u32;
            for (end, &side) in opening.spec.sides.iter().enumerate() {
                if side + 2 == face as usize {
                    let first = opening.edge + end as u32 * count;
                    loops.push((first..first + count).map(|edge| (edge, false)).collect());
                }
            }
            let ordinal = face as usize;
            if (opening.face..opening.face + count as usize).contains(&ordinal) {
                let i = (ordinal - opening.face) as u32;
                if !matches!(opening.spec.outline[i as usize], Edge::Line { .. }) {
                    return vec![];
                }
                let next = (i + 1) % count;
                let first = opening.edge;
                return vec![vec![
                    (first + i, false),
                    (first + 2 * count + next, false),
                    (first + count + i, true),
                    (first + 2 * count + i, true),
                ]];
            }
        }
        loops
    }

    pub(super) fn append_cut_topology(&self, placement: PlaneFrame, build: &mut Builder) {
        for opening in self.cut_iter() {
            let wall = opening.spec.sides.map(|side| side + 2);
            let count = opening.spec.outline.len();
            let full_circle = matches!(opening.spec.outline.as_slice(), [Edge::Circle { .. }]);
            let frames = [false, true].map(|end| self.cut_frame(placement, opening.spec, end));
            for (end, frame) in frames.into_iter().enumerate() {
                for (i, &edge) in opening.spec.outline.iter().enumerate() {
                    build.non_face(rim_in_frame(edge, frame), &[wall[end], opening.face + i]);
                    if !full_circle {
                        let previous = opening.face + (i + count - 1) % count;
                        let position_mm =
                            crate::geometry::rounded_slot_body::point(frame, edge.start());
                        build.non_face(
                            AnalyticGeometry::Point { position_mm },
                            &[wall[end], previous, opening.face + i],
                        );
                    }
                }
            }
            for (i, &edge) in opening.spec.outline.iter().enumerate() {
                let plane = cavity_plane(frames[0], edge);
                let face = opening.face + i;
                if full_circle {
                    build.face(face, None, None, wall);
                } else {
                    let previous = opening.face + (i + count - 1) % count;
                    let next = opening.face + (i + 1) % count;
                    build.face(face, plane, plane, [wall[0], wall[1], previous, next]);
                    let [a_mm, b_mm] = frames.map(|frame| {
                        crate::geometry::rounded_slot_body::point(frame, edge.start())
                    });
                    build.non_face(AnalyticGeometry::Line { a_mm, b_mm }, &[previous, face]);
                }
            }
        }
    }
}

fn cavity_plane(frame: ReferenceFrame, edge: Edge) -> Option<ReferenceFrame> {
    let Edge::Line { a, b } = edge else {
        return None;
    };
    let [dx, dy] = std::array::from_fn(|i| b[i] - a[i]);
    let length = dx.hypot(dy);
    Some(ReferenceFrame {
        origin_mm: crate::geometry::rounded_slot_body::point(frame, a),
        u: std::array::from_fn(|i| (dx * frame.u[i] + dy * frame.v[i]) / length),
        v: frame.normal().map(|n| -n),
    })
}
