//! Placed analytic thread body, stable boundary identities and display curves.
use super::*;
use crate::{HelicalThread, rust_prism_topology::position};
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind,
    TopologySnapshot,
};
use multicad_rust_kernel::{
    helical_thread::{Spec, Thread, ThreadError},
    helical_thread_mesh,
};
use std::{collections::BTreeSet, f64::consts::TAU};

type Point = [f64; 3];
const OPERATION: &str = "Rust helical thread";
const CAP_NAMES: [&str; 2] = ["cap.start", "cap.end"];
const BAND_NAMES: [&str; 4] = ["lower", "crest", "upper", "root"];
const MAX_DISPLAY_EDGE_POINTS: usize = 500_000;

pub(crate) struct HelicalBody {
    pub(super) geometry: Thread,
    pub(super) feature: EntityId,
    // Exact face definitions are retained by phase and trim. These detached
    // area and centroid properties come from smooth per-band integrals.
    local_faces: Vec<(Point, f64)>,
    rails: Vec<f64>,
}

impl Solid {
    /// Build a real thread with finite planar ends and a periodic analytic wall.
    pub fn rust_helical_thread(
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if feature.is_nil() {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Input,
                "Thread feature identity must not be nil",
            ));
        }
        if spec.radial_taper != 0. {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Rust helical bodies require a cylindrical profile",
            ));
        }
        for length in [
            spec.major_radius_mm,
            spec.pitch_mm,
            spec.length_mm,
            spec.depth_mm,
            spec.crest_width_mm,
        ] {
            policy.length(length)?;
        }
        policy.length(spec.major_radius_mm - spec.depth_mm)?;
        if let Some(radius) = root_radius_mm {
            policy.length(radius)?;
        }
        let geometry = Thread::new(Spec {
            major_radius_mm: spec.major_radius_mm,
            pitch_mm: spec.pitch_mm,
            length_mm: spec.length_mm,
            depth_mm: spec.depth_mm,
            crest_width_mm: spec.crest_width_mm,
            lower_flank_deg: spec.lower_flank_deg,
            upper_flank_deg: spec.upper_flank_deg,
            starts: spec.starts,
            left_hand: spec.left_hand,
            root_radius_mm,
        })
        .map_err(|cause| match cause {
            ThreadError::Invalid => error(
                OPERATION,
                GeometryErrorKind::Input,
                "Invalid finite helical profile",
            ),
            ThreadError::Limit => error(
                OPERATION,
                GeometryErrorKind::Limit,
                "Helical thread exceeds the turn budget",
            ),
        })?;
        let placed = HelicalBody::new(geometry, feature);
        let bounds = placed.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Helical(Rc::new(placed), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn helical_body(&self) -> Option<(&HelicalBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Helical(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    /// Query the modeled helical material in world coordinates.
    pub fn rust_thread_contains(&self, point_mm: Point) -> Result<bool, GeometryError> {
        self.policy.point(point_mm)?;
        let (body, frame) = self.helical_body().ok_or_else(|| {
            error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Select an analytic Rust thread body",
            )
        })?;
        let offset = std::array::from_fn(|i| point_mm[i] - frame.origin[i]);
        let local = [frame.u, frame.v, frame.normal()].map(|axis| dot(axis, offset));
        Ok(body
            .geometry
            .contains(local, self.policy.linear_tolerance_mm))
    }
}

fn integral<const N: usize>(a: f64, b: f64, function: impl Fn(f64) -> [f64; N]) -> [f64; N] {
    // Five-point Gauss-Legendre, applied to each smooth piece and subdivided
    // so the same rule also resolves phase-dependent centroid terms.
    let root = (10. / 7_f64).sqrt();
    let nodes = [
        0.,
        ((5. - 2. * root) / 9.).sqrt(),
        ((5. + 2. * root) / 9.).sqrt(),
    ];
    let weight_term = 13. * 70_f64.sqrt();
    let weights = [
        128. / 225.,
        (322. + weight_term) / 900.,
        (322. - weight_term) / 900.,
    ];
    let half = (b - a) / 16.;
    let mut result = [0.; N];
    for i in 0..8 {
        let center = a + (2. * f64::from(i) + 1.) * half;
        for (node, weight) in nodes.into_iter().zip(weights) {
            let p = function(node.mul_add(half, center));
            let q = if node == 0. {
                [0.; N]
            } else {
                function((-node).mul_add(half, center))
            };
            for j in 0..N {
                result[j] += (p[j] + q[j]) * weight * half;
            }
        }
    }
    result
}

impl HelicalBody {
    fn new(geometry: Thread, feature: EntityId) -> Self {
        let spec = geometry.spec();
        let breaks = geometry.profile_breaks();
        let height = spec.length_mm;
        let rate = geometry.helical_rate();
        let cap_area = geometry.volume_mm3() / height;
        let half_twist = height / (2. * rate);
        let spin_average = half_twist.sin() / half_twist;
        let mut cap_moment = [0.; 2];
        let mut walls = vec![[0.; 3]; 4 * spec.starts as usize];
        for period in 0..i64::from(spec.starts) {
            let shift = period as f64 * spec.pitch_mm;
            for (band, pair) in breaks.windows(2).enumerate() {
                let [area, x, y, cap_x, cap_y] = integral(pair[0], pair[1], |phase| {
                    let [r, slope] = geometry.radius_and_slope_at_phase(phase);
                    let area = r.hypot(slope * r.hypot(rate));
                    let (sin, cos) = (-(shift + phase) / rate).sin_cos();
                    [
                        area,
                        area * r * cos,
                        area * r * sin,
                        r.powi(3) * cos,
                        r.powi(3) * sin,
                    ]
                });
                let slot = &mut walls[geometry.face_index(period, band) as usize - 2];
                slot[0] += area;
                slot[1] += x;
                slot[2] += y;
                cap_moment[0] += cap_x;
                cap_moment[1] += cap_y;
            }
        }
        let cap_center = |z: f64| {
            let [x, y] = cap_moment.map(|moment| moment / (3. * rate.abs() * cap_area));
            let (sin, cos) = (z / rate).sin_cos();
            [x * cos - y * sin, x * sin + y * cos, z]
        };
        let (sin, cos) = half_twist.sin_cos();
        let mut local_faces = vec![(cap_center(0.), cap_area), (cap_center(height), cap_area)];
        local_faces.extend(walls.into_iter().map(|[area, x, y]| {
            let x_mid = (x * cos - y * sin) * spin_average / area;
            let y_mid = (x * sin + y * cos) * spin_average / area;
            ([x_mid, y_mid, height / 2.], area * height / rate.abs())
        }));
        let rails = (0..spec.starts)
            .flat_map(|period| {
                breaks[1..5]
                    .iter()
                    .map(move |phase| f64::from(period) * spec.pitch_mm + phase)
            })
            .collect();
        Self {
            geometry,
            feature,
            local_faces,
            rails,
        }
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |i: usize, sign: f64| {
            self.geometry
                .support([frame.u[i], frame.v[i], normal[i]].map(|x| x * sign))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    fn cap_frame(&self, frame: PlaneFrame, index: usize) -> ReferenceFrame {
        let z = index as f64 * self.geometry.spec().length_mm;
        ReferenceFrame {
            origin_mm: position(frame, [0., 0., z]),
            u: frame.u,
            v: if index == 0 {
                frame.v.map(|v| -v)
            } else {
                frame.v
            },
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.local_faces
            .iter()
            .enumerate()
            .map(|(index, &(center, area_mm2))| {
                let plane = (index < 2).then(|| self.cap_frame(frame, index));
                Face {
                    index: index as u32,
                    plane: plane.map(|plane| (plane.origin_mm, plane.normal())),
                    center_mm: position(frame, center),
                    area_mm2,
                }
            })
            .collect()
    }

    fn origin(&self, role: String) -> Origin {
        Origin {
            source: self.feature,
            role,
        }
    }

    fn band(&self, rail: usize) -> u32 {
        self.geometry.face_index((rail / 4) as i64, rail % 4 + 1)
    }

    fn rail_name(&self, rail: usize) -> String {
        let band = (self.band(rail) - 2) as usize;
        format!("start/{}/{}", band / 4, BAND_NAMES[band % 4])
    }

    fn next_phase(&self, rail: usize) -> f64 {
        self.rails
            .get(rail + 1)
            .copied()
            .unwrap_or(self.rails[0] + self.geometry.lead_mm())
    }

    fn point(&self, phase: f64, z: f64) -> Point {
        let (sin, cos) = ((z - phase) / self.geometry.helical_rate()).sin_cos();
        let r = self.geometry.radius_at_phase(phase);
        [r * cos, r * sin, z]
    }

    fn circular_rim(&self, frame: PlaneFrame, cap: usize, rail: usize) -> Option<AnalyticGeometry> {
        let spec = self.geometry.spec();
        let band = (self.band(rail) - 2) % 4;
        if band != 1 && (band != 3 || spec.root_radius_mm.is_some()) {
            return None;
        }
        let z = cap as f64 * spec.length_mm;
        let a = self.rails[rail];
        let b = self.next_phase(rail);
        let rate = self.geometry.helical_rate();
        Some(AnalyticGeometry::CircularArc {
            frame: ReferenceFrame {
                origin_mm: position(frame, [0., 0., z]),
                u: frame.u,
                v: frame.v,
            },
            radius_mm: self.geometry.radius_at_phase((a + b) / 2.),
            start_rad: (z - a) / rate,
            sweep_rad: (a - b) / rate,
        })
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let count = self.rails.len();
        let rail_origin =
            |index| self.origin(format!("thread/{}/leading_rail", self.rail_name(index)));
        let rim_origin =
            |cap, index| self.origin(format!("{}/rim/{}", CAP_NAMES[cap], self.rail_name(index)));
        let mut boundaries = vec![BTreeSet::new(); 2 + count];
        for i in 0..count {
            let band = self.band(i) as usize;
            let next = (i + 1) % count;
            boundaries[band].extend([rail_origin(i), rail_origin(next)]);
            for cap in 0..2 {
                let rim = rim_origin(cap, i);
                boundaries[cap].insert(rim.clone());
                boundaries[band].insert(rim);
            }
        }
        let mut elements = Vec::with_capacity(2 + 6 * count);
        for (index, boundary) in boundaries.into_iter().enumerate() {
            let origin = self.origin(if index < 2 {
                CAP_NAMES[index].into()
            } else {
                let band = index - 2;
                format!("thread/start/{}/{}", band / 4, BAND_NAMES[band % 4])
            });
            let frame = (index < 2).then(|| self.cap_frame(frame, index));
            let frames = frame
                .into_iter()
                .map(|frame| AnchoredFrame {
                    origin: origin.clone(),
                    frame,
                })
                .collect();
            let mut origins = BTreeSet::from([origin]);
            if index >= 2 && (index - 2) % 4 == 3 {
                origins.insert(self.origin("thread.root".into()));
            }
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: index as u32,
                },
                origins,
                boundary,
                frames,
                geometry: frame.map(|frame| AnalyticGeometry::Plane { frame }),
            });
        }
        // Rails and variable-radius rims are exact curves of the body.
        // Publish the circular rims through the existing analytic descriptor.
        for i in 0..3 * count {
            let origin = if i < count {
                rail_origin(i)
            } else {
                rim_origin(i / count - 1, i % count)
            };
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Edge,
                    ordinal: i as u32,
                },
                origins: BTreeSet::from([origin]),
                boundary: BTreeSet::new(),
                frames: vec![],
                geometry: (i >= count)
                    .then(|| self.circular_rim(frame, i / count - 1, i % count))
                    .flatten(),
            });
        }
        for (cap, name) in CAP_NAMES.into_iter().enumerate() {
            let z = cap as f64 * self.geometry.spec().length_mm;
            for (i, &phase) in self.rails.iter().enumerate() {
                elements.push(TopologyElement {
                    key: TopologyKey {
                        kind: TopologyKind::Vertex,
                        ordinal: (cap * count + i) as u32,
                    },
                    origins: BTreeSet::from([
                        self.origin(format!("{name}/corner/{}", self.rail_name(i)))
                    ]),
                    boundary: BTreeSet::new(),
                    frames: vec![],
                    geometry: Some(AnalyticGeometry::Point {
                        position_mm: position(frame, self.point(phase, z)),
                    }),
                });
            }
        }
        let snapshot = TopologySnapshot {
            owner,
            revision,
            elements,
        };
        snapshot
            .validate()
            .map_err(|e| error(OPERATION, GeometryErrorKind::InvalidResult, e))?;
        Ok(snapshot)
    }

    pub(super) fn display_edges(
        &self,
        frame: PlaneFrame,
        deflection: f64,
    ) -> Result<Vec<DisplayEdge>, GeometryError> {
        let spec = self.geometry.spec();
        let rate = self.geometry.helical_rate().abs();
        let slope = self.geometry.maximum_profile_slope();
        let second = spec
            .root_radius_mm
            .map_or(0., |rounding| (1. + slope * slope).powf(1.5) / rounding);
        let cap_curvature_bound = spec.major_radius_mm + 2. * rate * slope + rate * rate * second;
        let step =
            |second_derivative: f64| (8. * deflection / second_derivative).sqrt().min(TAU / 8.);
        let count = self.rails.len();
        let mut used = 0usize;
        let mut edges = Vec::with_capacity(3 * count);
        for index in 0..3 * count {
            let i = index % count;
            let phase = self.rails[i];
            let (segments, phase_span, z, depth_span) = if index < count {
                (
                    (spec.length_mm / (rate * step(spec.major_radius_mm))).ceil(),
                    0.,
                    0.,
                    spec.length_mm,
                )
            } else {
                let span = self.next_phase(i) - phase;
                (
                    (span / (rate * step(cap_curvature_bound))).ceil(),
                    span,
                    (index / count - 1) as f64 * spec.length_mm,
                    0.,
                )
            };
            if !segments.is_finite() || segments + 1. > (MAX_DISPLAY_EDGE_POINTS - used) as f64 {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Limit,
                    "Thread edge display exceeds its point budget",
                ));
            }
            let segments = (segments as usize).max(1);
            used += segments + 1;
            let points_mm = (0..=segments)
                .map(|n| {
                    let t = n as f64 / segments as f64;
                    position(
                        frame,
                        self.point(phase_span.mul_add(t, phase), depth_span.mul_add(t, z)),
                    )
                })
                .collect();
            edges.push(DisplayEdge {
                index: index as u32,
                points_mm,
            });
        }
        Ok(edges)
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<ffi::Mesh, GeometryError> {
        crate::rust_display::placed_mesh(
            helical_thread_mesh::tessellate(&self.geometry, options),
            frame,
            false,
        )
    }
}
