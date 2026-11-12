//! Exact cylindrical stock minus the authoritative five-band rounded thread.
use super::rounded_thread_body::{RoundedThreadBody, RoundedThreadMassProperties};
use super::*;
use crate::rust_prism_topology::position;
use multicad_core::references::{
    AnalyticGeometry, Origin, TopologyElement, TopologyKey, TopologyKind,
};
use multicad_rust_kernel::{
    rounded_thread::{Spec, ThroughBore},
    rounded_thread_mesh,
};
use std::collections::BTreeSet;

mod topology;
type Point = [f64; 3];
const OPERATION: &str = "Rust rounded receiving thread";
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedThreadBoreData {
    /// Full-through stock minus this literal rounded thread; never a solid shaft.
    pub bore: ThroughBore,
    pub frame: PlaneFrame,
}
pub(crate) struct RoundedThreadBoreBody {
    pub(crate) geometry: ThroughBore,
    pub(super) feature: EntityId,
    inner: Rc<RoundedThreadBody>,
    inherited: Option<(TopologySnapshot, PlaneFrame, bool)>,
    import_identity: Option<Origin>,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    error(OPERATION, kind, message)
}
impl Solid {
    pub fn rust_rounded_thread_bore(
        spec: Spec,
        radius_mm: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let inner = Self::rust_rounded_thread(spec, feature, frame, policy)?;
        policy.length(radius_mm)?;
        policy.length(radius_mm - spec.major_radius_mm)?;
        frame.checked_metric_extent([radius_mm, radius_mm, spec.length_mm], policy)?;
        let SolidBody::RoundedThread(inner, _) = inner.body else {
            unreachable!()
        };
        let geometry = ThroughBore::new(inner.geometry, radius_mm).map_err(|e| {
            failure(
                GeometryErrorKind::Input,
                format!("Invalid rounded receiving stock: {e:?}"),
            )
        })?;
        let body = RoundedThreadBoreBody {
            geometry,
            feature,
            inner,
            inherited: None,
            import_identity: None,
        };
        let bounds = body.bounds(frame)?;
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::RoundedThreadBore(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
            retained_placement: None,
        })
    }
    pub(crate) fn rounded_thread_bore_body(&self) -> Option<(&RoundedThreadBoreBody, PlaneFrame)> {
        match &self.body {
            SolidBody::RoundedThreadBore(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
    pub fn rust_rounded_thread_bore_data(&self) -> Option<RoundedThreadBoreData> {
        self.rounded_thread_bore_body()
            .map(|(body, frame)| RoundedThreadBoreData {
                bore: body.geometry,
                frame,
            })
    }
    pub fn rust_rounded_thread_bore_mass_properties(
        &self,
    ) -> Result<Option<RoundedThreadMassProperties>, GeometryError> {
        self.rounded_thread_bore_body()
            .map(|(body, frame)| body.mass(frame))
            .transpose()
    }
    pub(super) fn import_rounded_thread_bore(
        spec: Spec,
        radius: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let identity = super::step_import::feature_origin(
            feature,
            "rounded-bore-material.v1",
            super::rounded_thread_body::identity_values(spec, frame).chain([radius]),
        );
        let mut result = Self::rust_rounded_thread_bore(spec, radius, feature, frame, policy)?;
        let SolidBody::RoundedThreadBore(body, _) = &mut result.body else {
            unreachable!()
        };
        Rc::get_mut(body).expect("unshared import").import_identity = Some(identity);
        Ok(result)
    }
    pub(super) fn matches_imported_rounded_thread_bore(
        &self,
        imported: &multicad_rust_kernel::step_read::ImportSolid,
        tolerance: f64,
    ) -> bool {
        let Some((body, frame)) = self.rounded_thread_bore_body() else {
            return false;
        };
        let multicad_rust_kernel::step_read::ImportProfile::RoundedThreadBore {
            spec,
            stock_radius_mm,
        } = imported.profile
        else {
            return false;
        };
        let Ok(thread) = multicad_rust_kernel::rounded_thread::Thread::new(spec) else {
            return false;
        };
        let Ok(bore) = ThroughBore::new(thread, stock_radius_mm) else {
            return false;
        };
        multicad_rust_kernel::step_read::rounded_thread_bore_import_matches(
            &multicad_rust_kernel::step_read::ImportedRoundedThreadBore {
                bore,
                origin_mm: imported.origin_mm,
                u: imported.u,
                v: imported.v,
            },
            &body.geometry,
            [frame.origin, frame.u, frame.v],
            tolerance,
        )
    }
    pub fn rust_cut_rounded_internal_thread(
        &self,
        spec: Spec,
        minimum_wall_mm: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        // Validate the complete literal requested profile before changing chart.
        let tool = Self::rust_rounded_thread(spec, feature, frame, policy)?;
        let SolidBody::AxialCylinder(cylinder, stock, _) = &self.body else {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rounded receiving threads require one analytic stock cylinder",
            ));
        };
        let height = cylinder.height().as_f64();
        let radius = cylinder.circle().radius().as_f64();
        if self.policy != policy || policy.boolean_fuzzy_mm != 0. || spec.length_mm != height {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rounded receiving threads currently require the literal full stock height; partial-depth and overrun closures are not certified",
            ));
        }
        if feature.uuid_bits() == cylinder.feature()
            || !minimum_wall_mm.is_finite()
            || minimum_wall_mm < 0.
            || radius - spec.major_radius_mm < minimum_wall_mm.max(policy.minimum_feature_mm)
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Rounded receiving thread identity or minimum wall is invalid",
            ));
        }
        let alignment = dot(frame.normal(), stock.normal());
        if (alignment.abs() - 1.).abs() > 1e-10 {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rounded receiving thread must enter a coaxial stock cap",
            ));
        }
        let reverse = alignment < 0.;
        let origin = position(*stock, [0., 0., if reverse { height } else { 0. }]);
        let chart = helical_body::stock_frame::Chart::new(*stock);
        let [x, y, _] = chart.vector(frame.u);
        let norm = x.hypot(y);
        let [c, s] = [x / norm, y / norm];
        let sign = if reverse { -1. } else { 1. };
        let placed = PlaneFrame {
            origin,
            u: std::array::from_fn(|i| c * stock.u[i] + s * stock.v[i]),
            v: std::array::from_fn(|i| sign * (-s * stock.u[i] + c * stock.v[i])),
        };
        helical_body::stock_frame::certify_tool_domain(
            frame,
            placed,
            spec.major_radius_mm,
            spec.length_mm,
            policy,
        )?;
        helical_body::stock_frame::certify_circular_stock(
            *stock,
            placed,
            [c, s],
            radius,
            height,
            reverse,
            policy,
        )?;
        let _receipt = helical_body::stock_frame::store(
            placed,
            *stock,
            Bounds {
                min: [-radius, -radius, 0.],
                max: [radius, radius, height],
            },
            policy,
        )?;
        let mut result = Self::rust_rounded_thread_bore(
            tool.rust_rounded_thread_data()
                .expect("checked source")
                .thread
                .spec(),
            radius,
            feature,
            placed,
            policy,
        )?;
        let inherited = self.topology_snapshot(feature, 0)?;
        helical_body::stock_frame::store_anchors(placed, inherited.clone(), policy)?;
        let SolidBody::RoundedThreadBore(body, _) = &mut result.body else {
            unreachable!()
        };
        Rc::get_mut(body)
            .expect("unshared rounded receiving body")
            .inherited = Some((inherited, placed, reverse));
        Ok(result)
    }
}
impl RoundedThreadBoreBody {
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Result<Bounds, GeometryError> {
        let n = frame.normal();
        let mut bounds = Bounds {
            min: [0.; 3],
            max: [0.; 3],
        };
        for axis in 0..3 {
            for (sign, target) in [(-1., &mut bounds.min), (1., &mut bounds.max)] {
                let upper = self
                    .geometry
                    .support_bounds(
                        [sign * frame.u[axis], sign * frame.v[axis], sign * n[axis]],
                        self.inner.tolerance() / 8.,
                    )
                    .map_err(|e| {
                        failure(
                            GeometryErrorKind::Limit,
                            format!("Rounded receiver support: {e:?}"),
                        )
                    })?[1];
                let value = frame.origin[axis] + sign * upper;
                target[axis] = if sign < 0. {
                    value.next_down()
                } else {
                    value.next_up()
                };
            }
        }
        Ok(bounds)
    }
    pub(super) fn contains(&self, frame: PlaneFrame, point: Point) -> bool {
        let n = frame.normal();
        let rows = [cross(frame.v, n), cross(n, frame.u), n];
        let det = dot(frame.u, rows[0]);
        let delta = std::array::from_fn(|i| point[i] - frame.origin[i]);
        self.geometry
            .contains(rows.map(|r| dot(r, delta) / det), self.inner.tolerance())
    }
    pub(super) fn faces(&self, frame: PlaneFrame) -> Result<Vec<Face>, GeometryError> {
        let mut faces = self.inner.faces(frame)?;
        let area = std::f64::consts::PI * self.geometry.radius_mm().powi(2);
        let properties = self.inner.properties()?;
        for (i, face) in faces[..2].iter_mut().enumerate() {
            let (center, hole_area) = properties.faces[i];
            let remaining = area - hole_area;
            // Subtract moments in the retained local chart before adding a
            // potentially distant world origin; translation is not a moment.
            face.center_mm = position(
                frame,
                [
                    -hole_area * center[0] / remaining,
                    -hole_area * center[1] / remaining,
                    i as f64 * self.geometry.length_mm(),
                ],
            );
            face.area_mm2 = remaining;
        }
        faces.push(Face {
            index: 7,
            plane: None,
            center_mm: position(frame, [0., 0., self.geometry.length_mm() / 2.]),
            area_mm2: std::f64::consts::TAU * self.geometry.radius_mm() * self.geometry.length_mm(),
        });
        Ok(faces)
    }
    fn mass(&self, frame: PlaneFrame) -> Result<RoundedThreadMassProperties, GeometryError> {
        let m = self.inner.properties()?;
        let r = self.geometry.radius_mm();
        let h = self.geometry.length_mm();
        let stock = std::f64::consts::PI * r * r * h;
        let hole = self.geometry.thread().volume_mm3();
        let volume = self.geometry.volume_mm3();
        let center: Point = std::array::from_fn(|i| {
            ((if i == 2 { stock * h / 2. } else { 0. }) - hole * m.centroid_mm[i]) / volume
        });
        let second: [[f64; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let outer = if i != j {
                    0.
                } else if i == 2 {
                    stock * h * h / 3.
                } else {
                    stock * r * r / 4.
                };
                outer
                    - m.central_second_moment_mm5[i][j]
                    - hole * m.centroid_mm[i] * m.centroid_mm[j]
                    - volume * center[i] * center[j]
            })
        });
        let axes = [frame.u, frame.v, frame.normal()];
        let covariance: [[f64; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                (0..3)
                    .flat_map(|a| (0..3).map(move |b| axes[a][i] * second[a][b] * axes[b][j]))
                    .sum()
            })
        });
        let trace = (0..3).map(|i| covariance[i][i]).sum::<f64>();
        Ok(RoundedThreadMassProperties {
            volume_mm3: volume,
            centroid_mm: position(frame, center),
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
        let mesh = match rounded_thread_mesh::tessellate_bore(&self.geometry, options) {
            Ok(mesh) => Ok(mesh),
            Err(rounded_thread_mesh::Error::Mesh(e)) => Err(e),
            Err(rounded_thread_mesh::Error::Cancelled) => {
                return Err(failure(
                    GeometryErrorKind::Cancelled,
                    "Rounded receiver mesh superseded",
                ));
            }
        };
        crate::rust_display::placed_mesh(mesh, frame, false)
    }
    pub(super) fn volume_error_bound(
        &self,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<f64, GeometryError> {
        rounded_thread_mesh::bore_volume_error_bound(&self.geometry, options)
            .map_err(|e| failure(GeometryErrorKind::Limit, format!("{e:?}")))
    }
    pub(super) fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        writer.append_rounded_thread_bore(
            &self.geometry,
            frame.origin,
            frame.u,
            frame.v,
            policy.linear_tolerance_mm,
        )
    }
}
