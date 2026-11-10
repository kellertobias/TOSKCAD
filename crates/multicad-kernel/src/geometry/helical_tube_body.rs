//! Named Rust result of a finite circular helical cutter crossing a stock cap.
use super::*;
use crate::rust_prism_topology::{cylinder_snapshot, position};
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind,
};
use multicad_rust_kernel::{
    display::{MeshError, Tessellation},
    helical_tube_cut::{BoundaryEdge, Cut, Error as CutError, FaceProperties, Spec},
    prism::Location,
};
use std::{collections::BTreeSet, f64::consts::FRAC_PI_2};

const OPERATION: &str = "Rust helical tube cut";
const EDGES: [BoundaryEdge; 7] = [
    BoundaryEdge::Bottom,
    BoundaryEdge::TopRim,
    BoundaryEdge::FloorRim,
    BoundaryEdge::FloorCylinder,
    BoundaryEdge::TopCavity,
    BoundaryEdge::Rail { positive: false },
    BoundaryEdge::Rail { positive: true },
];

/// A right-handed helix centered on the cylinder's outer radius. Heights
/// and azimuth use the named stock's local frame, including its authored seam.
#[derive(Clone, Copy, Debug)]
pub struct HelicalTube {
    pub radius_mm: f64,
    pub pitch_mm: f64,
    pub path_height_mm: f64,
    pub start_height_mm: f64,
    pub start_angle_rad: f64,
}

pub(crate) struct HelicalTubeBody {
    pub(super) cut: Cut,
    pub(super) feature: EntityId,
    stock: Rc<AxialCylinder>,
    sketch: Option<Rc<CylinderSketch>>,
    faces: [FaceProperties; 5],
}

fn failure(cause: CutError) -> GeometryError {
    let (kind, message) = match cause {
        CutError::InvalidInput => (
            GeometryErrorKind::Input,
            "Invalid helical tube dimensions or phase",
        ),
        CutError::UnsupportedGeometry => (
            GeometryErrorKind::Unsupported,
            "The tube needs proven turn separation and must start within the stock and exit its top cap",
        ),
        CutError::Limit => (
            GeometryErrorKind::Limit,
            "Helical tube exceeds the length or turn budget",
        ),
    };
    error(OPERATION, kind, message)
}

fn direction(frame: PlaneFrame, [x, y, z]: [f64; 3]) -> [f64; 3] {
    let normal = frame.normal();
    std::array::from_fn(|i| frame.u[i].mul_add(x, frame.v[i].mul_add(y, normal[i] * z)))
}

impl Solid {
    /// Subtract an exactly modeled circular helical tube from an analytic cylinder.
    pub fn rust_cut_helical_tube(
        &self,
        tool: HelicalTube,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if feature.is_nil() {
            return Err(input("Helical tube cut feature identity must not be nil"));
        }
        if policy != self.policy || policy.boolean_fuzzy_mm != 0. {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Helical tube cut requires matching geometry policies without Boolean fuzz",
            ));
        }
        let SolidBody::AxialCylinder(stock, frame, sketch) = &self.body else {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Helical tube cut requires an exact axial cylinder",
            ));
        };
        for dimension in [tool.radius_mm, tool.pitch_mm, tool.path_height_mm] {
            policy.length(dimension)?;
        }
        let cut = Cut::new(
            Spec {
                stock_radius_mm: stock.circle().radius().as_f64(),
                stock_height_mm: stock.height().as_f64(),
                tube_radius_mm: tool.radius_mm,
                pitch_mm: tool.pitch_mm,
                path_height_mm: tool.path_height_mm,
                start_height_mm: tool.start_height_mm,
                start_angle_rad: tool.start_angle_rad,
            },
            policy.linear_tolerance_mm,
        )
        .map_err(failure)?;
        let faces = cut.face_properties();
        let volumes = [cut.removed_volume_mm3(), cut.volume_mm3()];
        if volumes
            .iter()
            .any(|volume| !volume.is_finite() || *volume <= 0.)
            || faces.iter().any(|face| {
                !face.area_mm2.is_finite()
                    || face.area_mm2 <= 0.
                    || face
                        .center_mm
                        .iter()
                        .any(|coordinate| !coordinate.is_finite())
            })
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::InvalidResult,
                "Helical tube cut did not certify positive material and boundary areas",
            ));
        }
        let body = HelicalTubeBody {
            cut,
            feature,
            stock: Rc::clone(stock),
            sketch: sketch.clone(),
            faces,
        };
        body.bounds(*frame)?;
        let result = Self {
            body: SolidBody::HelicalTubeCut(Rc::new(body), *frame),
            policy,
            provenance: self.provenance.clone(),
            rust_origins: self.rust_origins.clone(),
        };
        result
            .topology_snapshot(feature, 0)?
            .validate()
            .map_err(|message| error(OPERATION, GeometryErrorKind::InvalidResult, message))?;
        Ok(result)
    }

    pub fn rust_helical_tube_material(&self, world: [f64; 3]) -> Result<Location, GeometryError> {
        self.policy.point(world)?;
        let (body, frame) = self.helical_tube_cut_body().ok_or_else(|| {
            error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Select a certified helical tube cut",
            )
        })?;
        let delta: [f64; 3] = std::array::from_fn(|i| world[i] - frame.origin[i]);
        let local = [frame.u, frame.v, frame.normal()]
            .map(|axis| axis.into_iter().zip(delta).map(|(a, b)| a * b).sum());
        body.cut.classify(local).map_err(failure)
    }

    pub(crate) fn helical_tube_cut_body(&self) -> Option<(&HelicalTubeBody, PlaneFrame)> {
        match &self.body {
            SolidBody::HelicalTubeCut(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    /// Verify a published STEP boundary against this retained cylinder.
    pub fn verify_helical_tube_cut_step(
        &self,
        source: &str,
    ) -> Option<Result<f64, multicad_rust_kernel::step_read::ReadError>> {
        use multicad_rust_kernel::step_read::{self, ReadError};
        self.helical_tube_cut_body()?;
        let tolerance = self.policy.linear_tolerance_mm;
        Some((|| {
            let (readback, imported) = step_read::read_import_solid(source, tolerance)?;
            if !self.matches_helical_cut_profile(&imported) {
                return Err(ReadError::Invalid);
            }
            Ok(readback.volume_mm3)
        })())
    }

    pub(super) fn matches_helical_cut_profile(
        &self,
        imported: &multicad_rust_kernel::step_read::ImportSolid,
    ) -> bool {
        use multicad_rust_kernel::step_read::ImportProfile;
        let Some((body, frame)) = self.helical_tube_cut_body() else {
            return false;
        };
        let ImportProfile::HelicalTubeCut {
            stock_radius_mm,
            radius_mm,
            pitch_mm,
            start_height_mm,
            start_angle_rad,
            ..
        } = imported.profile
        else {
            return false;
        };
        let spec = body.cut.spec();
        let stock_extent = spec.stock_height_mm + spec.stock_radius_mm;
        let distance =
            |a: [f64; 3], b: [f64; 3]| (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]);
        let start_error = (start_height_mm - spec.start_height_mm).abs();
        let pitch_error = (pitch_mm - spec.pitch_mm).abs();
        let turns = (spec.stock_height_mm - spec.start_height_mm) / spec.pitch_mm;
        let angular_error = (start_angle_rad - spec.start_angle_rad) * 0.5;
        let phase_error = 2. * spec.stock_radius_mm * angular_error.sin().abs();
        let azimuth_drift =
            std::f64::consts::TAU * spec.stock_radius_mm * (start_error + turns * pitch_error)
                / spec.pitch_mm;
        let errors = [
            distance(frame.origin, imported.origin_mm),
            stock_extent * distance(frame.u, imported.u),
            stock_extent * distance(frame.v, imported.v),
            (imported.height_mm - spec.stock_height_mm).abs(),
            (stock_radius_mm - spec.stock_radius_mm).abs(),
            (radius_mm - spec.tube_radius_mm).abs(),
            start_error,
            pitch_error,
            phase_error,
            azimuth_drift,
        ];
        // Leave the remaining budget for the decoded trim and support.
        errors.into_iter().sum::<f64>() <= self.policy.linear_tolerance_mm * 0.25
    }
}

impl HelicalTubeBody {
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Result<Bounds, GeometryError> {
        let normal = frame.normal();
        let mut bounds = Bounds {
            min: frame.origin,
            max: frame.origin,
        };
        for (i, normal) in normal.into_iter().enumerate() {
            let axis = [frame.u[i], frame.v[i], normal];
            bounds.max[i] += self.cut.support_upper_mm(axis).map_err(failure)?;
            bounds.min[i] -= self
                .cut
                .support_upper_mm(axis.map(|coordinate| -coordinate))
                .map_err(failure)?;
        }
        Ok(bounds)
    }

    fn floor_frame(&self, stock: PlaneFrame) -> ReferenceFrame {
        let (origin, u, v) = self.cut.floor_frame();
        ReferenceFrame {
            origin_mm: position(stock, origin),
            u: direction(stock, u),
            v: direction(stock, v),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let z = frame.normal();
        let floor = self.floor_frame(frame);
        let planes = [
            Some((frame.origin, z.map(|v| -v))),
            Some((
                position(frame, [0., 0., self.cut.spec().stock_height_mm]),
                z,
            )),
            None,
            None,
            Some((floor.origin_mm, floor.normal())),
        ];
        self.faces
            .iter()
            .zip(planes)
            .enumerate()
            .map(|(i, (face, plane))| Face {
                index: i as u32,
                plane,
                center_mm: position(frame, face.center_mm),
                area_mm2: face.area_mm2,
            })
            .collect()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let mut snapshot =
            cylinder_snapshot(&self.stock, frame, self.sketch.as_deref(), owner, revision)
                .map_err(|message| error(OPERATION, GeometryErrorKind::InvalidResult, message))?;
        let origin = |role: &str| Origin {
            source: self.feature,
            role: role.into(),
        };
        let floor = self.floor_frame(frame);
        for (ordinal, name, support) in [
            (3, "helical_tube.wall", None),
            (4, "helical_tube.start", Some(floor)),
        ] {
            let name = origin(name);
            snapshot.elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal,
                },
                origins: BTreeSet::from([name.clone()]),
                frames: support
                    .map(|frame| AnchoredFrame {
                        origin: name,
                        frame,
                    })
                    .into_iter()
                    .collect(),
                geometry: support.map(|frame| AnalyticGeometry::Plane { frame }),
                boundary: BTreeSet::new(),
            });
        }
        let face_origins = |a, b| {
            snapshot
                .elements
                .iter()
                .filter(|element| {
                    element.key.kind == TopologyKind::Face && [a, b].contains(&element.key.ordinal)
                })
                .flat_map(|element| element.origins.iter().cloned())
                .collect::<BTreeSet<_>>()
        };
        let [a, b] = self.cut.edge_domain(BoundaryEdge::TopRim);
        let spec = self.cut.spec();
        let top_rim = AnalyticGeometry::CircularArc {
            frame: ReferenceFrame {
                origin_mm: position(frame, [0., 0., spec.stock_height_mm]),
                u: frame.u,
                v: frame.v,
            },
            radius_mm: spec.stock_radius_mm,
            start_rad: a,
            sweep_rad: b - a,
        };
        let [a, b] = self.cut.edge_domain(BoundaryEdge::FloorRim);
        let floor_rim = AnalyticGeometry::CircularArc {
            frame: floor,
            radius_mm: spec.tube_radius_mm,
            start_rad: a,
            sweep_rad: b - a,
        };
        let [a, b] = self.cut.edge_domain(BoundaryEdge::FloorCylinder);
        let floor_cylinder = AnalyticGeometry::EllipticArc {
            frame: ReferenceFrame {
                origin_mm: position(frame, [0., 0., spec.start_height_mm]),
                u: floor.v.map(|coordinate| -coordinate),
                v: floor.u,
            },
            semiaxes_mm: [
                self.cut
                    .edge_second_derivative_bound(BoundaryEdge::FloorCylinder),
                spec.stock_radius_mm,
            ],
            start_rad: FRAC_PI_2 + a,
            sweep_rad: b - a,
        };
        let new_edges = [
            (
                "helical_tube.floor_rim",
                face_origins(3, 4),
                Some(floor_rim),
            ),
            (
                "helical_tube.floor_cylinder",
                face_origins(2, 4),
                floor_cylinder.validate().is_ok().then_some(floor_cylinder),
            ),
            ("helical_tube.top_opening", face_origins(1, 3), None),
            ("helical_tube.negative_rail", face_origins(2, 3), None),
            ("helical_tube.positive_rail", face_origins(2, 3), None),
        ];
        let top = snapshot
            .elements
            .iter_mut()
            .find(|element| {
                element.key
                    == (TopologyKey {
                        kind: TopologyKind::Edge,
                        ordinal: 1,
                    })
            })
            .expect("stock cylinder top rim");
        top.geometry = Some(top_rim);
        for (i, (role, boundary, geometry)) in new_edges.into_iter().enumerate() {
            snapshot.elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Edge,
                    ordinal: i as u32 + 2,
                },
                origins: BTreeSet::from([origin(role)]),
                geometry,
                boundary,
                frames: vec![],
            });
        }
        for (i, (edge, end, rail, first, second)) in [
            (EDGES[5], 0, "negative_rail", "floor_rim", "floor_cylinder"),
            (EDGES[6], 0, "positive_rail", "floor_rim", "floor_cylinder"),
            (EDGES[5], 1, "negative_rail", "top_opening", "top_rim"),
            (EDGES[6], 1, "positive_rail", "top_opening", "top_rim"),
        ]
        .into_iter()
        .enumerate()
        {
            let point = self.cut.edge_point(edge, self.cut.edge_domain(edge)[end]);
            let mut boundary = BTreeSet::from([
                origin(&format!("helical_tube.{rail}")),
                origin(&format!("helical_tube.{first}")),
            ]);
            if second == "top_rim" {
                boundary.extend(
                    snapshot
                        .elements
                        .iter()
                        .filter(|element| {
                            element.key
                                == (TopologyKey {
                                    kind: TopologyKind::Edge,
                                    ordinal: 1,
                                })
                        })
                        .flat_map(|element| element.origins.iter().cloned()),
                );
            } else {
                boundary.insert(origin(&format!("helical_tube.{second}")));
            }
            snapshot.elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Vertex,
                    ordinal: i as u32,
                },
                origins: BTreeSet::from([origin(&format!("helical_tube.join.{i}"))]),
                geometry: Some(AnalyticGeometry::Point {
                    position_mm: position(frame, point),
                }),
                boundary,
                frames: vec![],
            });
        }
        Ok(snapshot)
    }

    pub(super) fn sample_edge(
        &self,
        frame: PlaneFrame,
        ordinal: u32,
        options: Tessellation,
    ) -> Option<Result<Vec<[f64; 3]>, MeshError>> {
        let &edge = EDGES.get(ordinal as usize)?;
        let [a, b] = self.cut.edge_domain(edge);
        let bound = self.cut.edge_second_derivative_bound(edge);
        let step = (8. * options.deflection / bound).sqrt().min(options.angle);
        let steps = ((b - a) / step).ceil() as usize;
        if steps == 0 || steps >= options.triangle_limit {
            return Some(Err(MeshError::BudgetExceeded));
        }
        Some(Ok((0..=steps)
            .map(|i| {
                let t = if i == steps {
                    b
                } else {
                    (b - a).mul_add(i as f64 / steps as f64, a)
                };
                position(frame, self.cut.edge_point(edge, t))
            })
            .collect()))
    }
}
