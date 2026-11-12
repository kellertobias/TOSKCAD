//! Placed analytic thread body, stable boundary identities and display curves.
use super::*;
use crate::{
    HelicalThread,
    rust_prism_topology::{cylinder_snapshot_dimensions, position},
};
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind,
    TopologySnapshot,
};
use multicad_rust_kernel::{
    headed_screw::Screw,
    helical_receiver::Receiver,
    helical_thread::{AxialPrefix, Spec, Thread, ThreadError},
    helical_thread_mesh,
    hex_nut::Nut,
};
use std::{
    collections::BTreeSet,
    f64::consts::{PI, TAU},
};

mod blind;
mod end_relief;
mod external;
use external::ExternalStock;
mod boundary;
mod nut;
mod screw;
mod stock_frame;
use blind::BlindStock;
pub use blind::{BlindThreadCut, BlindThreadMating};

type Point = [f64; 3];
const OPERATION: &str = "Rust helical thread";
const CAP_NAMES: [&str; 2] = ["cap.start", "cap.end"];
const BAND_NAMES: [&str; 4] = ["lower", "crest", "upper", "root"];
const MAX_DISPLAY_EDGE_POINTS: usize = 500_000;

fn imported_request(spec: Spec) -> HelicalThread {
    HelicalThread {
        major_radius_mm: spec.major_radius_mm,
        pitch_mm: spec.pitch_mm,
        length_mm: spec.length_mm,
        depth_mm: spec.depth_mm,
        crest_width_mm: spec.crest_width_mm,
        lower_flank_deg: spec.lower_flank_deg,
        upper_flank_deg: spec.upper_flank_deg,
        radial_taper: 0.,
        starts: spec.starts,
        left_hand: spec.left_hand,
    }
}

/// Source-tool and occupied material intervals in the retained receiving frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReceivingThreadIntervals {
    pub frame: PlaneFrame,
    pub requested_tool_length_mm: f64,
    pub physical_thread_length_mm: f64,
    pub stock_length_mm: f64,
}

pub(crate) struct HelicalBody {
    geometry: Thread,
    pub(super) feature: EntityId,
    form: HelicalForm,
    // Exact face definitions are retained by phase and trim. These detached
    // area and centroid properties come from smooth per-band integrals.
    local_faces: Vec<(Point, f64)>,
    rails: Vec<f64>,
    import_identity: Option<Origin>,
}

enum HelicalForm {
    Shaft,
    EndRelief(multicad_rust_kernel::thread_end_relief::Relief),
    External(Box<ExternalStock>),
    Bore(BoreStock),
    Blind(BlindStock),
    Nut(Nut),
    Screw(Screw),
}

struct BoreStock {
    source_feature: EntityId,
    radius_mm: f64,
    length_mm: f64,
    frame_in_thread: stock_frame::StoredFrame,
    sketch: Option<Rc<CylinderSketch>>,
    reversed: bool,
    prefix: Option<Box<AxialPrefix>>,
}

impl BoreStock {
    fn length(&self) -> f64 {
        self.length_mm
    }
    fn receiver(&self, thread: Thread) -> Option<Receiver> {
        Receiver::new(thread, self.radius(), self.length()).ok()
    }
    fn radius(&self) -> f64 {
        self.radius_mm
    }

    fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let placed = self.frame_in_thread.placed(frame);
        cylinder_snapshot_dimensions(
            self.source_feature,
            [self.radius(), self.length()],
            placed,
            self.sketch.as_deref(),
            owner,
            revision,
        )
        .map_err(|e| error(OPERATION, GeometryErrorKind::InvalidResult, e))
    }
}

impl Solid {
    /// The exact placed helical profiles retained by a modeled screw and nut.
    pub fn rust_screw_nut_threads(&self, nut: &Self) -> Option<[(Thread, PlaneFrame); 2]> {
        let SolidBody::Helical(screw, screw_frame) = &self.body else {
            return None;
        };
        let SolidBody::Helical(nut, nut_frame) = &nut.body else {
            return None;
        };
        if !matches!(screw.form, HelicalForm::Screw(_)) || !matches!(nut.form, HelicalForm::Nut(_))
        {
            return None;
        }
        Some([(screw.geometry, *screw_frame), (nut.geometry, *nut_frame)])
    }

    /// Build a real thread with finite planar ends and a periodic analytic wall.
    pub fn rust_helical_thread(
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rust_helical_body(spec, root_radius_mm, feature, frame, policy, |_| {
            Ok(HelicalForm::Shaft)
        })
    }

    pub(super) fn import_helical_thread(
        spec: Spec,
        stock: Option<f64>,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::import_thread_stock(spec, stock, spec.length_mm, frame, feature, policy)
    }
    pub(super) fn import_helical_receiver(
        spec: Spec,
        radius: f64,
        length: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.length(length - spec.length_mm)?;
        Self::import_thread_stock(spec, Some(radius), length, frame, feature, policy)
    }
    fn import_thread_stock(
        spec: Spec,
        stock: Option<f64>,
        stock_length: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let request = imported_request(spec);
        let mut solid =
            Self::rust_helical_body(request, spec.root_radius_mm, feature, frame, policy, |_| {
                if let Some(radius) = stock {
                    policy.length(radius - spec.major_radius_mm)?;
                    // The inferred through topology couples stock H to the exact
                    // retained thread interval. Decimal approximation would invent
                    // an unproved floor when the inferred scalar is one ulp lower.
                    let stock_bits = feature.uuid_bits() ^ u128::MAX;
                    Ok(HelicalForm::Bore(BoreStock {
                        source_feature: EntityId::from_uuid_bits(if stock_bits == 0 {
                            1
                        } else {
                            stock_bits
                        }),
                        radius_mm: radius,
                        length_mm: stock_length,
                        frame_in_thread: stock_frame::StoredFrame::identity(),
                        sketch: None,
                        reversed: false,
                        prefix: None,
                    }))
                } else {
                    Ok(HelicalForm::Shaft)
                }
            })?;
        let identity = super::step_import::feature_origin(
            feature,
            if stock_length == spec.length_mm {
                "helical-material"
            } else {
                "helical-blind-material"
            },
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
                    spec.crest_width_mm,
                    spec.lower_flank_deg,
                    spec.upper_flank_deg,
                    spec.starts as f64,
                    f64::from(spec.left_hand),
                    spec.root_radius_mm.unwrap_or(0.),
                    stock.unwrap_or(0.),
                ])
                .chain((stock_length != spec.length_mm).then_some(stock_length)),
        );
        let SolidBody::Helical(body, _) = &mut solid.body else {
            unreachable!()
        };
        Rc::get_mut(body)
            .expect("new unshared imported body")
            .import_identity = Some(identity);
        Ok(solid)
    }
    pub(super) fn import_helical_screw(
        spec: Spec,
        screw: multicad_rust_kernel::headed_screw::ScrewSpec,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let mut solid = Self::rust_machine_screw_at_thread_frame(
            imported_request(spec),
            spec.root_radius_mm,
            screw,
            feature,
            frame,
            policy,
        )?;
        use multicad_rust_kernel::headed_screw::ScrewHead;
        let (tag, head) = match screw.head {
            ScrewHead::ExternalHex {
                across_flats_mm,
                height_mm,
            } => (
                "helical-hex-material",
                vec![across_flats_mm, height_mm, screw.shank_length_mm],
            ),
            ScrewHead::SocketCap {
                radius_mm,
                height_mm,
                socket_across_flats_mm,
                socket_depth_mm,
            } => (
                "helical-socket-material",
                vec![
                    radius_mm,
                    height_mm,
                    socket_across_flats_mm,
                    socket_depth_mm,
                    screw.shank_length_mm,
                ],
            ),
        };
        let identity = super::step_import::feature_origin(
            feature,
            tag,
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
                    spec.crest_width_mm,
                    spec.lower_flank_deg,
                    spec.upper_flank_deg,
                    spec.starts as f64,
                    f64::from(spec.left_hand),
                    spec.root_radius_mm.unwrap_or(0.),
                ])
                .chain(head),
        );
        let SolidBody::Helical(body, _) = &mut solid.body else {
            unreachable!()
        };
        Rc::get_mut(body)
            .expect("new unshared imported screw")
            .import_identity = Some(identity);
        Ok(solid)
    }

    pub(super) fn import_helical_nut(
        spec: Spec,
        across_flats_mm: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.length(across_flats_mm / 2. - spec.major_radius_mm)?;
        let mut solid = Self::rust_helical_body(
            imported_request(spec),
            spec.root_radius_mm,
            feature,
            frame,
            policy,
            |thread| {
                Nut::new(thread, across_flats_mm)
                    .map(HelicalForm::Nut)
                    .map_err(|_| input("Invalid imported threaded hex nut"))
            },
        )?;
        let identity = super::step_import::feature_origin(
            feature,
            "helical-nut-material",
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
                    spec.crest_width_mm,
                    spec.lower_flank_deg,
                    spec.upper_flank_deg,
                    spec.starts as f64,
                    f64::from(spec.left_hand),
                    spec.root_radius_mm.unwrap_or(0.),
                    across_flats_mm,
                ]),
        );
        let SolidBody::Helical(body, _) = &mut solid.body else {
            unreachable!()
        };
        Rc::get_mut(body)
            .expect("new unshared imported nut")
            .import_identity = Some(identity);
        Ok(solid)
    }

    pub(super) fn matches_imported_helical_thread(
        &self,
        imported: &multicad_rust_kernel::step_read::ImportSolid,
        tolerance: f64,
    ) -> bool {
        if self.rounded_thread_body().is_some() {
            return self.matches_imported_rounded_thread(imported, tolerance);
        }
        let Some((body, frame)) = self.helical_body() else {
            return false;
        };
        if let HelicalForm::External(stock) = &body.form
            && let Some(shaft) = &stock.partial
        {
            return multicad_rust_kernel::step_read::threaded_shaft_import_matches(
                imported,
                shaft,
                [frame.origin, frame.u, frame.v],
                tolerance,
            );
        }
        if let HelicalForm::EndRelief(relief) = &body.form {
            return multicad_rust_kernel::step_read::thread_end_relief_import_matches(
                imported,
                relief,
                [frame.origin, frame.u, frame.v],
                tolerance,
            );
        }
        if let HelicalForm::Screw(screw) = &body.form {
            return multicad_rust_kernel::step_read::helical_screw_import_matches(
                imported,
                screw,
                [frame.origin, frame.u, frame.v],
                tolerance,
            );
        }
        if let HelicalForm::Nut(nut) = &body.form {
            return multicad_rust_kernel::step_read::helical_nut_import_matches(
                imported,
                nut,
                [frame.origin, frame.u, frame.v],
                tolerance,
            );
        }
        if let HelicalForm::Bore(stock) = &body.form
            && let Some(receiver) = stock.receiver(body.geometry)
        {
            return multicad_rust_kernel::step_read::helical_receiver_import_matches(
                imported,
                &receiver,
                [frame.origin, frame.u, frame.v],
                tolerance,
            );
        }
        let Some(stock) = body.step_stock() else {
            return false;
        };
        multicad_rust_kernel::step_read::helical_import_matches(
            imported,
            &body.geometry,
            stock,
            [frame.origin, frame.u, frame.v],
            tolerance,
        )
    }

    /// For cylindrical receivers, report tool length separately from occupied
    /// material. STEP imports recover only material, so their tool length equals it.
    pub fn rust_receiving_thread_intervals(&self) -> Option<ReceivingThreadIntervals> {
        let (body, frame) = self.helical_body()?;
        if let HelicalForm::External(stock) = &body.form {
            return Some(stock.intervals(body.geometry, frame));
        }
        let HelicalForm::Bore(stock) = &body.form else {
            return None;
        };
        Some(ReceivingThreadIntervals {
            frame,
            requested_tool_length_mm: stock
                .prefix
                .as_ref()
                .map_or(body.geometry.spec().length_mm, |prefix| {
                    prefix.tool().spec().length_mm
                }),
            physical_thread_length_mm: body.geometry.spec().length_mm,
            stock_length_mm: stock.length(),
        })
    }

    /// Cut the requested cap-entry thread interval in coaxial cylindrical stock.
    /// A partial-depth cut retains its solid floor; an overrun uses the tool's
    /// certified prefix inside stock. The original outer wall and cap attachment
    /// frames retain their authored origins.
    pub fn rust_cut_internal_thread(
        &self,
        spec: HelicalThread,
        minimum_wall_mm: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        let tool = checked_thread(spec, None, policy)?;
        let SolidBody::AxialCylinder(cylinder, stock_frame, sketch) = &self.body else {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "A cylindrical receiving thread requires one analytic stock cylinder",
            ));
        };
        let stock_frame = *stock_frame;
        let height = cylinder.height().as_f64();
        let normal = frame.normal();
        let stock_normal = stock_frame.normal();
        let alignment = dot(normal, stock_normal);
        let reversed = alignment < 0.;
        let cap = if reversed { height } else { 0. };
        let start = position(stock_frame, [0., 0., cap]);
        let requested_end = position(frame, [0., 0., spec.length_mm]);
        let distance = |a: Point, b: Point| {
            let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
            x.hypot(y).hypot(z)
        };
        if self.policy != policy
            || policy.boolean_fuzzy_mm != 0.
            || !spec.length_mm.is_finite()
            || spec.length_mm <= 0.
            || (alignment.abs() - 1.).abs() > 1e-10
            || distance(start, frame.origin) > policy.linear_tolerance_mm
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "A cylindrical receiving thread must enter a coaxial stock cap under the same exact geometry policy",
            ));
        }
        policy.length(spec.length_mm)?;
        if spec.length_mm < height {
            policy.length(height - spec.length_mm).map_err(|cause| {
                error(
                    OPERATION,
                    cause.kind,
                    format!(
                        "Receiving stock height {height:.17} mm and tool length {:.17} mm leave an inadmissible floor: {}",
                        spec.length_mm, cause.message
                    ),
                )
            })?;
        }
        if !minimum_wall_mm.is_finite()
            || minimum_wall_mm < 0.
            || cylinder.circle().radius().as_f64() - spec.major_radius_mm
                < minimum_wall_mm.max(policy.minimum_feature_mm)
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Input,
                "The receiving cylinder has insufficient wall outside the thread crest",
            ));
        }
        if cylinder.feature() == feature.uuid_bits() {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Input,
                "The thread and receiving feature identities must be distinct",
            ));
        }

        // Clock in the retained stock chart. Its admitted metric residual
        // makes a transpose projection or world-axis normalization lossy.
        let chart = stock_frame::Chart::new(stock_frame);
        let [x, y, _] = chart.vector(frame.u);
        let length = x.hypot(y);
        let [c, s] = [x / length, y / length];
        let sign = if reversed { -1. } else { 1. };
        let placed = PlaneFrame {
            origin: start,
            u: std::array::from_fn(|i| c * stock_frame.u[i] + s * stock_frame.v[i]),
            v: std::array::from_fn(|i| sign * (-s * stock_frame.u[i] + c * stock_frame.v[i])),
        };
        if distance(position(placed, [0., 0., spec.length_mm]), requested_end)
            > policy.linear_tolerance_mm
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "The receiving thread cannot retain its declared axial interval",
            ));
        }
        stock_frame::certify_tool_placement(frame, placed, &tool, policy)?;
        let radius = cylinder.circle().radius().as_f64();
        let prefix = if spec.length_mm > height {
            Some(tool.axial_prefix(height).map(Box::new).map_err(|_| {
                error(
                    OPERATION,
                    GeometryErrorKind::Limit,
                    "Receiving stock is too short for the certified thread prefix",
                )
            })?)
        } else {
            None
        };
        let stock = BoreStock {
            source_feature: EntityId::from_uuid_bits(cylinder.feature()),
            radius_mm: radius,
            length_mm: height,
            frame_in_thread: stock_frame::store(
                placed,
                stock_frame,
                Bounds {
                    min: [-radius, -radius, 0.],
                    max: [radius, radius, height],
                },
                policy,
            )?,
            sketch: sketch.clone(),
            reversed,
            prefix,
        };
        if let Some(prefix) = &stock.prefix {
            // Publish the retained prefix data directly. No normalized axial
            // parameter or recomputed profile can alter the source root law.
            Self::publish_helical_geometry(*prefix.material(), feature, placed, policy, |_| {
                Ok(HelicalForm::Bore(stock))
            })
        } else {
            Self::publish_helical_geometry(tool, feature, placed, policy, |_| {
                Ok(HelicalForm::Bore(stock))
            })
        }
    }

    fn rust_helical_body(
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
        form: impl FnOnce(Thread) -> Result<HelicalForm, GeometryError>,
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
        let geometry = checked_thread(spec, root_radius_mm, policy)?;
        Self::publish_helical_geometry(geometry, feature, frame, policy, form)
    }

    fn publish_helical_geometry(
        geometry: Thread,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
        form: impl FnOnce(Thread) -> Result<HelicalForm, GeometryError>,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if feature.is_nil() {
            return Err(input("Thread feature identity must not be nil"));
        }
        let placed = HelicalBody::new(geometry, feature, form(geometry)?);
        frame.checked_metric_bounds(placed.bounds(PlaneFrame::default()), policy)?;
        let bounds = placed.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Helical(Rc::new(placed), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
            retained_placement: None,
        })
    }

    pub(crate) fn helical_body(&self) -> Option<(&HelicalBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Helical(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    /// Validate the closed helical STEP against its source certificate and
    /// integrate its decoded surface volume in mm³.
    pub fn verify_helical_step(
        &self,
        source: &str,
    ) -> Option<Result<f64, multicad_rust_kernel::step_read::ReadError>> {
        if let Some((body, frame)) = self.rounded_thread_body() {
            return Some(multicad_rust_kernel::step_read::verify_rounded_thread(
                source,
                &body.geometry,
                frame.origin,
                frame.u,
                frame.v,
                self.policy.linear_tolerance_mm,
            ));
        }
        let (body, frame) = self.helical_body()?;
        if let HelicalForm::External(stock) = &body.form
            && let Some(shaft) = &stock.partial
        {
            return Some(multicad_rust_kernel::step_read::verify_threaded_shaft(
                source,
                shaft,
                frame.origin,
                frame.u,
                frame.v,
                self.policy.linear_tolerance_mm,
            ));
        }
        if let HelicalForm::EndRelief(relief) = &body.form {
            return Some(multicad_rust_kernel::step_read::verify_thread_end_relief(
                source,
                relief,
                frame.origin,
                frame.u,
                frame.v,
                self.policy.linear_tolerance_mm,
            ));
        }
        if let HelicalForm::Screw(screw) = &body.form {
            return Some(multicad_rust_kernel::step_read::verify_helical_screw(
                source,
                screw,
                frame.origin,
                frame.u,
                frame.v,
                self.policy.linear_tolerance_mm,
            ));
        }
        if let HelicalForm::Nut(nut) = &body.form {
            return Some(multicad_rust_kernel::step_read::verify_helical_nut(
                source,
                nut,
                frame.origin,
                frame.u,
                frame.v,
                self.policy.linear_tolerance_mm,
            ));
        }
        if let HelicalForm::Bore(stock) = &body.form
            && let Some(receiver) = stock.receiver(body.geometry)
        {
            return Some(multicad_rust_kernel::step_read::verify_helical_receiver(
                source,
                &receiver,
                frame.origin,
                frame.u,
                frame.v,
                self.policy.linear_tolerance_mm,
            ));
        }
        let radius = body.step_stock()?;
        Some(multicad_rust_kernel::step_read::verify_helical_thread(
            source,
            &body.geometry,
            radius,
            frame.origin,
            frame.u,
            frame.v,
            self.policy.linear_tolerance_mm,
        ))
    }

    /// Query the modeled helical material in world coordinates.
    pub fn rust_thread_contains(&self, point_mm: Point) -> Result<bool, GeometryError> {
        self.policy.point(point_mm)?;
        if let Some((body, frame)) = self.rounded_thread_body() {
            return Ok(body.contains(frame, point_mm));
        }
        let (body, frame) = self.helical_body().ok_or_else(|| {
            error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Select an analytic Rust thread body",
            )
        })?;
        let offset = std::array::from_fn(|i| point_mm[i] - frame.origin[i]);
        let local = [frame.u, frame.v, frame.normal()].map(|axis| dot(axis, offset));
        Ok(body.contains(local, self.policy.linear_tolerance_mm))
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

fn checked_thread(
    spec: HelicalThread,
    root_radius_mm: Option<f64>,
    policy: GeometryPolicy,
) -> Result<Thread, GeometryError> {
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
    Thread::new(Spec {
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
    })
}

impl HelicalBody {
    fn new(geometry: Thread, feature: EntityId, form: HelicalForm) -> Self {
        let spec = geometry.spec();
        let breaks = geometry.profile_breaks();
        let height = spec.length_mm;
        let rate = geometry.helical_rate();
        let hole_area = geometry.volume_mm3() / height;
        let cap_area = match &form {
            HelicalForm::Bore(stock) => PI * stock.radius().powi(2) - hole_area,
            HelicalForm::Nut(nut) => nut.outer_area_mm2() - hole_area,
            _ => hole_area,
        };
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
            let sign = if matches!(&form, HelicalForm::Bore(_) | HelicalForm::Nut(_)) {
                -1.
            } else {
                1.
            };
            let [x, y] = cap_moment.map(|moment| sign * moment / (3. * rate.abs() * cap_area));
            let (sin, cos) = (z / rate).sin_cos();
            [x * cos - y * sin, x * sin + y * cos, z]
        };
        let (sin, cos) = half_twist.sin_cos();
        let partial =
            matches!(&form,HelicalForm::Bore(stock) if stock.receiver(geometry).is_some());
        let end_cap = if partial {
            let [x, y, z] = cap_center(height);
            (
                [-x * cap_area / hole_area, -y * cap_area / hole_area, z],
                hole_area,
            )
        } else {
            (cap_center(height), cap_area)
        };
        let mut local_faces = vec![(cap_center(0.), cap_area), end_cap];
        local_faces.extend(walls.into_iter().map(|[area, x, y]| {
            let x_mid = (x * cos - y * sin) * spin_average / area;
            let y_mid = (x * sin + y * cos) * spin_average / area;
            ([x_mid, y_mid, height / 2.], area * height / rate.abs())
        }));
        match &form {
            HelicalForm::Bore(stock) => {
                local_faces.push((
                    [0., 0., stock.length() / 2.],
                    TAU * stock.radius() * stock.length(),
                ));
                if partial {
                    local_faces.push(([0., 0., stock.length()], PI * stock.radius().powi(2)));
                }
            }
            HelicalForm::Blind(stock) => stock.extend_properties(
                &mut local_faces,
                hole_area,
                cap_moment.map(|moment| moment / (3. * rate.abs())),
            ),
            HelicalForm::Nut(nut) => nut::extend_face_properties(nut, &mut local_faces),
            HelicalForm::Screw(screw) => screw::extend_face_properties(screw, &mut local_faces),
            HelicalForm::Shaft => {}
            HelicalForm::EndRelief(relief) => {
                end_relief::extend_properties(relief, &mut local_faces)
            }
            HelicalForm::External(stock) => stock.extend_properties(geometry, &mut local_faces),
        }
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
            form,
            local_faces,
            rails,
            import_identity: None,
        }
    }

    /// Shape forms covered by the certified helical STEP shell.
    fn step_stock(&self) -> Option<Option<f64>> {
        match &self.form {
            HelicalForm::Shaft => Some(None),
            HelicalForm::External(stock) if stock.partial.is_none() => Some(None),
            HelicalForm::External(_) | HelicalForm::EndRelief(_) => None,
            HelicalForm::Bore(stock) if stock.receiver(self.geometry).is_none() => {
                Some(Some(stock.radius()))
            }
            HelicalForm::Bore(_) => None,
            HelicalForm::Blind(_) | HelicalForm::Nut(_) | HelicalForm::Screw(_) => None,
        }
    }

    pub(super) fn has_importable_step_closure(&self) -> bool {
        self.step_stock().is_some()
            || matches!(
                self.form,
                HelicalForm::Nut(_)
                    | HelicalForm::Bore(_)
                    | HelicalForm::External(_)
                    | HelicalForm::EndRelief(_)
            )
            || matches!(&self.form, HelicalForm::Screw(screw)
            if match screw.spec().head {
                multicad_rust_kernel::headed_screw::ScrewHead::ExternalHex { .. } => screw.spec().shank_length_mm == 0.,
                multicad_rust_kernel::headed_screw::ScrewHead::SocketCap { .. } => true,
            })
    }
    pub(super) fn supports_step(&self, frame: PlaneFrame, policy: GeometryPolicy) -> bool {
        if let HelicalForm::External(stock) = &self.form
            && let Some(shaft) = &stock.partial
        {
            return multicad_rust_kernel::step::supports_threaded_shaft(
                shaft,
                frame.origin,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::EndRelief(relief) = &self.form {
            return multicad_rust_kernel::step::supports_thread_end_relief(
                relief,
                frame.origin,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::Screw(screw) = &self.form {
            return multicad_rust_kernel::step::supports_helical_screw(
                screw,
                frame.origin,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::Nut(nut) = &self.form {
            return multicad_rust_kernel::step::supports_helical_nut(
                nut,
                frame.origin,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::Bore(stock) = &self.form
            && let Some(receiver) = stock.receiver(self.geometry)
        {
            return multicad_rust_kernel::step::supports_helical_receiver(
                &receiver,
                frame.origin,
                policy.linear_tolerance_mm,
            );
        }
        self.step_stock().is_some_and(|radius| {
            multicad_rust_kernel::step::supports_helical_thread(
                &self.geometry,
                radius,
                frame.origin,
                policy.linear_tolerance_mm,
            )
        })
    }

    pub(super) fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        if let HelicalForm::External(stock) = &self.form
            && let Some(shaft) = &stock.partial
        {
            return writer.append_threaded_shaft(
                shaft,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::EndRelief(relief) = &self.form {
            return writer.append_thread_end_relief(
                relief,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::Screw(screw) = &self.form {
            return writer.append_helical_screw(
                screw,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::Nut(nut) = &self.form {
            return writer.append_helical_nut(
                nut,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            );
        }
        if let HelicalForm::Bore(stock) = &self.form
            && let Some(receiver) = stock.receiver(self.geometry)
        {
            return writer.append_helical_receiver(
                &receiver,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            );
        }
        let radius = self
            .step_stock()
            .ok_or(multicad_rust_kernel::step::StepError::Unsupported)?;
        writer.append_helical_thread(
            &self.geometry,
            radius,
            frame.origin,
            frame.u,
            frame.v,
            policy.linear_tolerance_mm,
        )
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        match &self.form {
            HelicalForm::Shaft => self.geometry.volume_mm3(),
            HelicalForm::EndRelief(relief) => relief.volume_mm3(),
            HelicalForm::External(stock) => stock.volume(self.geometry),
            HelicalForm::Bore(stock) => {
                PI * stock.radius() * stock.radius() * stock.length() - self.geometry.volume_mm3()
            }
            HelicalForm::Blind(stock) => stock.volume_mm3(self.geometry),
            HelicalForm::Nut(nut) => nut.volume_mm3(),
            HelicalForm::Screw(screw) => screw.volume_mm3(),
        }
    }

    fn contains(&self, local: Point, tolerance: f64) -> bool {
        let stock = match &self.form {
            HelicalForm::Shaft => return self.geometry.contains(local, tolerance),
            HelicalForm::EndRelief(relief) => return relief.contains(local, tolerance),
            HelicalForm::External(stock) => return stock.contains(self.geometry, local, tolerance),
            HelicalForm::Screw(screw) => return screw.contains(local, tolerance),
            HelicalForm::Nut(nut) => return nut.contains(local, tolerance),
            HelicalForm::Blind(stock) => {
                let [x, y, z] = local;
                let length = self.geometry.spec().length_mm;
                return stock.contains(local, tolerance)
                    && (z >= length - tolerance
                        || z < -tolerance
                        || x.hypot(y) >= self.geometry.radius(y.atan2(x), z.max(0.)) - tolerance);
            }
            HelicalForm::Bore(stock) => stock,
        };
        let [x, y, z] = local;
        let length = self.geometry.spec().length_mm;
        let radius = x.hypot(y);
        z >= -tolerance
            && z <= stock.length() + tolerance
            && radius <= stock.radius() + tolerance
            && ((stock.length() > length && z >= length - tolerance)
                || radius >= self.geometry.radius(y.atan2(x), z.clamp(0., length)) - tolerance)
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        if let HelicalForm::Blind(stock) = &self.form {
            return stock.bounds(frame);
        }
        let normal = frame.normal();
        let support = |i: usize, sign: f64| {
            let direction = [frame.u[i], frame.v[i], normal[i]].map(|x| x * sign);
            match &self.form {
                HelicalForm::Shaft => self.geometry.support(direction),
                HelicalForm::EndRelief(relief) => relief.support(direction),
                HelicalForm::External(stock) => stock.support(self.geometry, direction),
                HelicalForm::Bore(stock) => {
                    stock.radius() * frame.u[i].hypot(frame.v[i])
                        + (sign * normal[i] * stock.length()).max(0.)
                }
                HelicalForm::Screw(screw) => screw.support(direction),
                HelicalForm::Nut(nut) => nut.support(direction),
                HelicalForm::Blind(_) => unreachable!("blind tap uses stock bounds"),
            }
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
            v: if matches!(&self.form,HelicalForm::External(stock) if stock.partial.is_some())
                || matches!(&self.form, HelicalForm::Blind(_))
                || matches!(&self.form,HelicalForm::Bore(stock) if stock.receiver(self.geometry).is_some())
                || (index == 0 && !matches!(&self.form, HelicalForm::Screw(_)))
            {
                frame.v.map(|v| -v)
            } else {
                frame.v
            },
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let screw_roles = match &self.form {
            HelicalForm::Screw(screw) => screw.faces(),
            _ => Vec::new(),
        };
        let blind_faces = match &self.form {
            HelicalForm::Blind(stock) => stock.stock_faces(frame),
            _ => Vec::new(),
        };
        self.local_faces
            .iter()
            .enumerate()
            .map(|(index, &(center, area_mm2))| {
                let plane = if index < 2 {
                    let cap = self.cap_frame(frame, index);
                    Some((cap.origin_mm, cap.normal()))
                } else if let HelicalForm::EndRelief(relief) = &self.form {
                    end_relief::plane(relief, index, frame)
                        .map(|plane| (plane.origin_mm, plane.normal()))
                } else if let HelicalForm::External(stock) = &self.form {
                    stock
                        .plane(index, frame)
                        .map(|plane| (plane.origin_mm, plane.normal()))
                } else if let HelicalForm::Screw(screw) = &self.form {
                    screw::plane(screw, screw_roles[index], frame)
                        .map(|plane| (plane.origin_mm, plane.normal()))
                } else if let HelicalForm::Bore(stock) = &self.form
                    && stock.receiver(self.geometry).is_some()
                    && index == 3 + self.rails.len()
                {
                    Some((position(frame, [0., 0., stock.length()]), frame.normal()))
                } else if let HelicalForm::Nut(nut) = &self.form {
                    nut::plane(nut, index.checked_sub(2 + self.rails.len()), frame)
                        .map(|plane| (plane.origin_mm, plane.normal()))
                } else if let HelicalForm::Blind(stock) = &self.form
                    && index >= 2 + self.rails.len()
                {
                    blind_faces[stock.stock_face_index(index, 2 + self.rails.len())].plane
                } else {
                    None
                };
                Face {
                    index: index as u32,
                    plane,
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

    fn cap_name(&self, cap: usize) -> &'static str {
        if matches!(&self.form, HelicalForm::EndRelief(_)) {
            ["cap.start", "thread.end-relief/shoulder"][cap]
        } else if matches!(&self.form, HelicalForm::External(_)) {
            [
                "connection.external/thread.entry",
                "connection.external/thread.limit",
            ][cap]
        } else if matches!(&self.form, HelicalForm::Screw(_)) {
            ["screw.thread/start", "screw.thread/end"][cap]
        } else if matches!(&self.form, HelicalForm::Blind(_))
            || matches!(&self.form,HelicalForm::Bore(stock) if stock.receiver(self.geometry).is_some())
        {
            [
                "connection.receiver/thread.entry",
                "connection.receiver/thread.floor",
            ][cap]
        } else {
            CAP_NAMES[cap]
        }
    }

    fn rim_origin(&self, cap: usize, rail: usize) -> Origin {
        self.origin(format!(
            "{}/rim/{}",
            self.cap_name(cap),
            self.rail_name(rail)
        ))
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
        let rim_origin = |cap, index| self.rim_origin(cap, index);
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
                self.cap_name(index).into()
            } else {
                let band = index - 2;
                format!("thread/start/{}/{}", band / 4, BAND_NAMES[band % 4])
            });
            let plane = (index < 2).then(|| self.cap_frame(frame, index));
            let frames = plane
                .into_iter()
                .map(|mut anchor| {
                    if matches!(&self.form, HelicalForm::Nut(_)) {
                        // Stock cap attachment coordinates stay in the
                        // authored extrusion frame even on the outward -Z face.
                        anchor.v = frame.v;
                    }
                    AnchoredFrame {
                        origin: origin.clone(),
                        frame: anchor,
                    }
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
                geometry: plane.map(|frame| AnalyticGeometry::Plane { frame }),
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
        for cap in 0..2 {
            let name = self.cap_name(cap);
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
        if let HelicalForm::Bore(stock) = &self.form {
            let authored = stock.topology(frame, owner, revision)?;
            for mut element in authored.elements {
                match element.key {
                    TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: 0 | 1,
                    } => {
                        let index = element.key.ordinal as usize ^ usize::from(stock.reversed);
                        let index = if index == 1 && stock.receiver(self.geometry).is_some() {
                            3 + count
                        } else {
                            index
                        };
                        if index == 3 + count {
                            element.key.ordinal = index as u32;
                            elements.push(element);
                            continue;
                        }
                        let cap = &mut elements[index];
                        cap.origins.extend(element.origins);
                        cap.boundary.extend(element.boundary);
                        cap.frames.extend(element.frames);
                    }
                    TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: 2,
                    } => {
                        element.key.ordinal = (2 + count) as u32;
                        elements.push(element);
                    }
                    TopologyKey {
                        kind: TopologyKind::Edge,
                        ..
                    } => {
                        element.key.ordinal += (3 * count) as u32;
                        elements.push(element);
                    }
                    _ => {
                        return Err(error(
                            OPERATION,
                            GeometryErrorKind::InvalidResult,
                            "Unexpected analytic receiving cylinder topology",
                        ));
                    }
                }
            }
        }
        if let HelicalForm::Blind(stock) = &self.form {
            stock.extend_topology(frame, owner, revision, count, &mut elements)?;
        }
        if let HelicalForm::Screw(screw) = &self.form {
            screw::extend_topology(self, screw, frame, &mut elements);
        }
        if let HelicalForm::Nut(nut) = &self.form {
            nut::extend_topology(self, nut, frame, &mut elements);
        }
        if let HelicalForm::External(stock) = &self.form {
            stock.extend_topology(self, frame, owner, revision, count, &mut elements)?;
        }
        if let HelicalForm::EndRelief(relief) = &self.form {
            end_relief::extend_topology(self, relief, frame, &mut elements);
        }
        if let Some(identity) = &self.import_identity {
            let stamp = |old: &Origin| Origin {
                source: self.feature,
                role: format!("{}/{}/{}", identity.role, old.source, old.role),
            };
            for element in &mut elements {
                element.origins = element.origins.iter().map(stamp).collect();
                element.boundary = element.boundary.iter().map(stamp).collect();
                for anchor in &mut element.frames {
                    anchor.origin = stamp(&anchor.origin);
                }
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
        match &self.form {
            HelicalForm::Bore(stock) => {
                let authored = stock.topology(frame, self.feature, 0)?;
                let rims = crate::rust_display::edges(None, &authored, deflection)?;
                edges.extend(rims.into_iter().map(|mut edge| {
                    edge.index += (3 * count) as u32;
                    edge
                }));
            }
            HelicalForm::Blind(stock) => {
                let authored = stock.topology(frame, self.feature, 0)?;
                let stock_edges = crate::rust_display::edges(None, &authored, deflection)?;
                edges.extend(stock_edges.into_iter().map(|mut edge| {
                    edge.index += (3 * count) as u32;
                    edge
                }));
            }
            HelicalForm::Screw(_)
            | HelicalForm::Nut(_)
            | HelicalForm::External(_)
            | HelicalForm::EndRelief(_) => {
                let mut topology = self.topology(frame, self.feature, 0)?;
                topology.elements.retain(|element| {
                    element.key.kind == TopologyKind::Edge
                        && element.key.ordinal >= (3 * count) as u32
                });
                edges.extend(crate::rust_display::edges(None, &topology, deflection)?);
            }
            HelicalForm::Shaft => {}
        }
        Ok(edges)
    }

    pub(super) fn volume_error_bound(
        &self,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> f64 {
        match &self.form {
            HelicalForm::EndRelief(relief) => {
                helical_thread_mesh::end_relief_volume_error_bound(relief, options)
            }
            HelicalForm::External(stock) => stock.partial.map_or_else(
                || helical_thread_mesh::volume_error_bound(&self.geometry, options),
                |shaft| helical_thread_mesh::shaft_volume_error_bound(&shaft, options),
            ),
            HelicalForm::Shaft | HelicalForm::Nut(_) | HelicalForm::Blind(_) => {
                helical_thread_mesh::volume_error_bound(&self.geometry, options)
            }
            HelicalForm::Bore(stock) => stock.receiver(self.geometry).map_or_else(
                || {
                    helical_thread_mesh::bore_volume_error_bound(
                        &self.geometry,
                        stock.radius(),
                        options,
                    )
                },
                |receiver| helical_thread_mesh::receiver_volume_error_bound(&receiver, options),
            ),
            HelicalForm::Screw(screw) => {
                helical_thread_mesh::screw_volume_error_bound(screw, options)
            }
        }
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<Mesh, GeometryError> {
        let mesh = match &self.form {
            HelicalForm::Shaft => helical_thread_mesh::tessellate(&self.geometry, options),
            HelicalForm::EndRelief(relief) => {
                helical_thread_mesh::tessellate_end_relief(relief, options)
            }
            HelicalForm::External(stock) => stock.partial.map_or_else(
                || helical_thread_mesh::tessellate(&self.geometry, options),
                |shaft| helical_thread_mesh::tessellate_shaft(&shaft, options),
            ),
            HelicalForm::Bore(stock) => stock.receiver(self.geometry).map_or_else(
                || helical_thread_mesh::tessellate_bore(&self.geometry, stock.radius(), options),
                |receiver| helical_thread_mesh::tessellate_receiver(&receiver, options),
            ),
            HelicalForm::Blind(stock) => stock.mesh(&self.geometry, options),
            HelicalForm::Screw(screw) => helical_thread_mesh::tessellate_screw(screw, options),
            HelicalForm::Nut(nut) => helical_thread_mesh::tessellate_nut(nut, options),
        };
        crate::rust_display::placed_mesh(mesh, frame, false)
    }
}
