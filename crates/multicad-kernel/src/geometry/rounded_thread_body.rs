//! Literal double-rounded thread material with stable semantic boundaries.
use super::*;
use crate::rust_prism_topology::position;
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind,
};
use multicad_rust_kernel::{
    rounded_thread::{Spec, Thread},
    rounded_thread_mesh,
};
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeSet,
};
mod display;
mod metrics;
mod topology;

type Point = [f64; 3];
const OPERATION: &str = "Rust rounded thread";
const CAPS: [&str; 2] = ["cap.start", "cap.end"];
const BANDS: [&str; 5] = ["crest.start", "lower", "root", "upper", "crest.end"];
const RAILS: [&str; 5] = [
    "crest.peak",
    "crest.lower",
    "root.lower",
    "root.upper",
    "crest.upper",
];

/// Authored or independently proved standalone rounded-thread source geometry.
/// Receiving, nut and headed closures have distinct future data contracts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedThreadData {
    pub thread: Thread,
    pub frame: PlaneFrame,
}
/// Unit-density physical properties; inertia is about the world centroid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedThreadMassProperties {
    pub volume_mm3: f64,
    pub centroid_mm: [f64; 3],
    pub inertia_mm5: [[f64; 3]; 3],
}

pub(crate) struct RoundedThreadBody {
    pub(crate) geometry: Thread,
    pub(super) feature: EntityId,
    tolerance: f64,
    import_identity: Option<Origin>,
    metrics: OnceCell<Result<metrics::Metrics, GeometryError>>,
    bounds: RefCell<Option<(PlaneFrame, Bounds)>>,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    error(OPERATION, kind, message)
}
fn source_error(e: multicad_rust_kernel::rounded_thread::Error) -> GeometryError {
    let kind = match e {
        multicad_rust_kernel::rounded_thread::Error::Invalid => GeometryErrorKind::Input,
        multicad_rust_kernel::rounded_thread::Error::Limit => GeometryErrorKind::Limit,
        multicad_rust_kernel::rounded_thread::Error::Unresolved => GeometryErrorKind::Unsupported,
    };
    failure(
        kind,
        format!("Rounded thread source cannot be proved: {e:?}"),
    )
}
impl Solid {
    pub fn rust_rounded_thread(
        spec: Spec,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rounded_thread_geometry(
            Thread::new(spec).map_err(source_error)?,
            feature,
            frame,
            policy,
            None,
        )
    }
    fn rounded_thread_geometry(
        geometry: Thread,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
        import_identity: Option<Origin>,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if feature.is_nil() {
            return Err(input("Rounded thread identity must not be nil"));
        }
        let s = geometry.spec();
        for x in [
            s.major_radius_mm,
            s.pitch_mm,
            s.length_mm,
            s.depth_mm,
            s.crest_radius_mm,
            s.root_radius_mm,
            geometry.root_radius_mm(),
        ] {
            policy.length(x)?;
        }
        for bounds in geometry.profile_breaks().windows(2) {
            policy.length(bounds[1] - bounds[0])?;
        }
        frame.checked_metric_extent([s.major_radius_mm, s.major_radius_mm, s.length_mm], policy)?;
        let body = RoundedThreadBody {
            geometry,
            feature,
            tolerance: policy.linear_tolerance_mm,
            import_identity,
            metrics: OnceCell::new(),
            bounds: RefCell::new(None),
        };
        let bounds = body.bounds(frame)?;
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::RoundedThread(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
            retained_placement: None,
        })
    }
    pub(crate) fn rounded_thread_body(&self) -> Option<(&RoundedThreadBody, PlaneFrame)> {
        match &self.body {
            SolidBody::RoundedThread(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
    /// Exact source data for typed analytic consumers, including drawing providers.
    pub fn rust_rounded_thread_data(&self) -> Option<RoundedThreadData> {
        self.rounded_thread_body()
            .map(|(body, frame)| RoundedThreadData {
                thread: body.geometry,
                frame,
            })
    }
    /// Heavy derived properties are evaluated once, only when requested.
    pub fn rust_rounded_thread_mass_properties(
        &self,
    ) -> Result<Option<RoundedThreadMassProperties>, GeometryError> {
        self.rounded_thread_body()
            .map(|(body, frame)| body.mass(frame))
            .transpose()
    }
    pub(super) fn import_rounded_thread(
        spec: Spec,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let identity = super::step_import::feature_origin(
            feature,
            "rounded-thread-material.v1",
            identity_values(spec, frame),
        );
        Self::rounded_thread_geometry(
            Thread::new(spec).map_err(source_error)?,
            feature,
            frame,
            policy,
            Some(identity),
        )
    }
    pub(super) fn matches_imported_rounded_thread(
        &self,
        imported: &multicad_rust_kernel::step_read::ImportSolid,
        tolerance: f64,
    ) -> bool {
        let Some((body, frame)) = self.rounded_thread_body() else {
            return false;
        };
        let multicad_rust_kernel::step_read::ImportProfile::RoundedThread { spec } =
            imported.profile
        else {
            return false;
        };
        let Ok(thread) = Thread::new(spec) else {
            return false;
        };
        multicad_rust_kernel::step_read::rounded_thread_import_matches(
            &multicad_rust_kernel::step_read::ImportedRoundedThread {
                thread,
                origin_mm: imported.origin_mm,
                u: imported.u,
                v: imported.v,
            },
            &body.geometry,
            [frame.origin, frame.u, frame.v],
            tolerance,
        )
    }
}
pub(super) fn identity_values(spec: Spec, frame: PlaneFrame) -> impl Iterator<Item = f64> {
    frame
        .origin
        .into_iter()
        .chain(frame.u)
        .chain(frame.v)
        .chain([
            spec.major_radius_mm,
            spec.pitch_mm,
            spec.length_mm,
            spec.depth_mm,
            spec.crest_radius_mm,
            spec.root_radius_mm,
            spec.flank_angle_deg,
            spec.starts as f64,
            f64::from(spec.left_hand),
        ])
}
impl RoundedThreadBody {
    pub(super) fn tolerance(&self) -> f64 {
        self.tolerance
    }
    pub(super) fn properties(&self) -> Result<&metrics::Metrics, GeometryError> {
        self.metrics
            .get_or_init(|| metrics::compute(&self.geometry))
            .as_ref()
            .map_err(Clone::clone)
    }
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Result<Bounds, GeometryError> {
        if let Some((key, bounds)) = *self.bounds.borrow()
            && key == frame
        {
            return Ok(bounds);
        }
        let n = frame.normal();
        let mut min = [0.; 3];
        let mut max = [0.; 3];
        for i in 0..3 {
            for (sign, target) in [(-1., &mut min), (1., &mut max)] {
                let direction = [frame.u[i] * sign, frame.v[i] * sign, n[i] * sign];
                let upper = self
                    .geometry
                    .support_bounds(direction, self.tolerance / 8.)
                    .map_err(source_error)?[1];
                let value = frame.origin[i] + sign * upper;
                target[i] = if sign < 0. {
                    value.next_down()
                } else {
                    value.next_up()
                };
            }
        }
        let result = Bounds { min, max };
        *self.bounds.borrow_mut() = Some((frame, result));
        Ok(result)
    }
    pub(super) fn contains(&self, frame: PlaneFrame, point: Point) -> bool {
        let n = frame.normal();
        let rows = [cross(frame.v, n), cross(n, frame.u), n];
        let det = dot(frame.u, rows[0]);
        let delta = std::array::from_fn(|i| point[i] - frame.origin[i]);
        self.geometry
            .contains(rows.map(|r| dot(r, delta) / det), self.tolerance)
    }
    fn cap_frame(&self, frame: PlaneFrame, end: usize) -> ReferenceFrame {
        ReferenceFrame {
            origin_mm: position(frame, [0., 0., end as f64 * self.geometry.spec().length_mm]),
            u: frame.u,
            v: frame.v.map(|v| if end == 0 { -v } else { v }),
        }
    }
    pub(super) fn faces(&self, frame: PlaneFrame) -> Result<Vec<Face>, GeometryError> {
        Ok(self
            .properties()?
            .faces
            .iter()
            .enumerate()
            .map(|(i, &(center, area))| Face {
                index: i as u32,
                plane: (i < 2).then(|| {
                    let f = self.cap_frame(frame, i);
                    (f.origin_mm, f.normal())
                }),
                center_mm: position(frame, center),
                area_mm2: area,
            })
            .collect())
    }
    pub(super) fn mass(
        &self,
        frame: PlaneFrame,
    ) -> Result<RoundedThreadMassProperties, GeometryError> {
        let m = self.properties()?;
        let basis = [frame.u, frame.v, frame.normal()];
        let covariance: [[f64; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                (0..3)
                    .flat_map(|a| {
                        (0..3).map(move |b| {
                            basis[a][i] * m.central_second_moment_mm5[a][b] * basis[b][j]
                        })
                    })
                    .sum()
            })
        });
        let trace = (0..3).map(|i| covariance[i][i]).sum::<f64>();
        Ok(RoundedThreadMassProperties {
            volume_mm3: self.geometry.volume_mm3(),
            centroid_mm: position(frame, m.centroid_mm),
            inertia_mm5: std::array::from_fn(|i| {
                std::array::from_fn(|j| {
                    if i == j {
                        trace - covariance[i][j]
                    } else {
                        -covariance[i][j]
                    }
                })
            }),
        })
    }
    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<Mesh, GeometryError> {
        let mesh = match rounded_thread_mesh::tessellate(&self.geometry, options) {
            Ok(mesh) => Ok(mesh),
            Err(rounded_thread_mesh::Error::Mesh(error)) => Err(error),
            Err(rounded_thread_mesh::Error::Cancelled) => {
                return Err(failure(
                    GeometryErrorKind::Cancelled,
                    "Rounded thread mesh superseded",
                ));
            }
        };
        crate::rust_display::placed_mesh(mesh, frame, false)
    }
    pub(super) fn volume_error_bound(
        &self,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<f64, GeometryError> {
        rounded_thread_mesh::volume_error_bound(&self.geometry, options)
            .map_err(|e| failure(GeometryErrorKind::Limit, format!("{e:?}")))
    }
    pub(super) fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        writer.append_rounded_thread(
            &self.geometry,
            frame.origin,
            frame.u,
            frame.v,
            policy.linear_tolerance_mm,
        )
    }
    fn origin(&self, role: impl Into<String>) -> Origin {
        Origin {
            source: self.feature,
            role: role.into(),
        }
    }
    fn point(&self, phase: f64, z: f64) -> Point {
        let angle = (z - phase) / self.geometry.helical_rate();
        let r = self.geometry.radius_at_phase(phase);
        [r * angle.cos(), r * angle.sin(), z]
    }
}
