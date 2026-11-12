//! Exact reflection of circular stock and its authored attachments.
//! Authored U and negative reflected V restore the full basis after two Mirrors.
use super::*;

struct CircularReflection {
    normal: [f64; 3],
    squared: f64,
    offset: f64,
    placed: PlaneFrame,
    inverse: [[f64; 3]; 3],
}

impl CircularReflection {
    fn new(
        frame: PlaneFrame,
        plane: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let normal = plane.normal();
        let squared = dot(normal, normal);
        let mut reflection = Self {
            normal,
            squared,
            offset: 2. * dot(normal, plane.origin) / squared,
            placed: frame,
            inverse: [[0.; 3]; 3],
        };
        // Full circular sections are invariant under a local V reflection.
        // The two orientation reversals keep the placed body right-handed.
        reflection.placed = PlaneFrame {
            origin: reflection.point(frame.origin),
            u: reflection.vector(frame.u),
            v: reflection.vector(frame.v).map(|value| -value),
        };
        reflection.placed.checked(policy)?;
        let axes = [
            reflection.placed.u,
            reflection.placed.v,
            reflection.placed.normal(),
        ];
        // Checked charts admit tiny metric residuals. Use the actual
        // inverse so far authored UV origins survive a frame round trip.
        let cofactors = [
            cross(axes[1], axes[2]),
            cross(axes[2], axes[0]),
            cross(axes[0], axes[1]),
        ];
        let determinant = dot(axes[0], cofactors[0]);
        reflection.inverse = cofactors.map(|row| row.map(|value| value / determinant));
        Ok(reflection)
    }

    fn vector(&self, value: [f64; 3]) -> [f64; 3] {
        let distance = 2. * dot(self.normal, value) / self.squared;
        std::array::from_fn(|i| value[i] - distance * self.normal[i])
    }

    fn point(&self, value: [f64; 3]) -> [f64; 3] {
        let reflected = self.vector(value);
        std::array::from_fn(|i| reflected[i] + self.offset * self.normal[i])
    }

    fn anchor(
        &self,
        anchor: ReferenceFrame,
        policy: GeometryPolicy,
    ) -> Result<ReferenceFrame, GeometryError> {
        let reflected = ReferenceFrame {
            origin_mm: self.point(anchor.origin_mm),
            u: self.vector(anchor.u),
            v: self.vector(anchor.v).map(|value| -value),
        };
        let delta = std::array::from_fn(|i| reflected.origin_mm[i] - self.placed.origin[i]);
        let stored = ReferenceFrame {
            origin_mm: self.inverse.map(|row| dot(row, delta)),
            u: self.inverse.map(|row| dot(row, reflected.u)),
            v: self.inverse.map(|row| dot(row, reflected.v)),
        };
        let axes = [self.placed.u, self.placed.v, self.placed.normal()];
        let vector =
            |local: [f64; 3]| std::array::from_fn(|i| (0..3).map(|j| axes[j][i] * local[j]).sum());
        let recovered = ReferenceFrame {
            origin_mm: position(self.placed, stored.origin_mm),
            u: vector(stored.u),
            v: vector(stored.v),
        };
        if [
            (reflected.origin_mm, recovered.origin_mm),
            (reflected.u, recovered.u),
            (reflected.v, recovered.v),
        ]
        .into_iter()
        .any(|(expected, actual)| {
            let residual = std::array::from_fn::<_, 3, _>(|i| actual[i] - expected[i]);
            let distance = residual[0].hypot(residual[1]).hypot(residual[2]);
            !distance.is_finite() || distance > policy.linear_tolerance_mm
        }) {
            return Err(error(
                "mirror",
                GeometryErrorKind::Unsupported,
                "Circular Mirror cannot preserve the reflected attachment within model tolerance",
            ));
        }
        Ok(stored)
    }
}

impl Solid {
    pub(crate) fn mirror_annulus(
        &self,
        plane: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        let (body, frame) = self.annulus_body()?;
        Some((|| {
            self.circular_mirror_policy(policy)?;
            let reflection = CircularReflection::new(frame, plane, policy)?;
            let authored_v = frame
                .v
                .map(|value| if body.reverse { -value } else { value });
            let anchors: [_; 2] = std::array::from_fn(|cap| {
                let center = position(
                    frame,
                    [
                        0.,
                        0.,
                        if cap == 0 {
                            0.
                        } else {
                            body.geometry.height_mm()
                        },
                    ],
                );
                reflection.anchor(body.cap_anchor(frame, cap, center, authored_v), policy)
            });
            let [start, end] = anchors;
            let mirrored = AnnulusBody {
                geometry: body.geometry,
                feature: body.feature,
                wall_origins: body.wall_origins.clone(),
                center_uv: body.center_uv,
                reverse: !body.reverse,
                cap_anchors: Some([start?, end?]),
            };
            let result = Self {
                body: SolidBody::Annulus(Rc::new(mirrored), reflection.placed),
                policy,
                provenance: vec![],
                rust_origins: self.rust_origins.clone(),
                retained_placement: None,
            };
            result.circular_mirror_bounds(policy)?;
            Ok(result)
        })())
    }

    pub(crate) fn mirror_cylinder(
        &self,
        plane: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        let SolidBody::AxialCylinder(cylinder, frame, sketch) = &self.body else {
            return None;
        };
        Some((|| {
            self.circular_mirror_policy(policy)?;
            let reflection = CircularReflection::new(*frame, plane, policy)?;
            let sketch = sketch
                .as_ref()
                .map(|sketch| {
                    let snapshot =
                        self.topology_snapshot(EntityId::from_uuid_bits(cylinder.feature()), 0)?;
                    let mut anchors = [ReferenceFrame {
                        origin_mm: [0.; 3],
                        u: [0.; 3],
                        v: [0.; 3],
                    }; 2];
                    for (cap, anchor) in anchors.iter_mut().enumerate() {
                        let source = snapshot
                            .elements
                            .iter()
                            .find(|element| {
                                element.key.kind == TopologyKind::Face
                                    && element.key.ordinal == cap as u32
                            })
                            .expect("validated circular cap");
                        *anchor = reflection.anchor(source.frames[0].frame, policy)?;
                    }
                    Ok(Rc::new(crate::geometry::CylinderSketch {
                        origin: sketch.origin.clone(),
                        center_uv: sketch.center_uv,
                        reverse: !sketch.reverse,
                        cap_anchors: Some(anchors),
                    }))
                })
                .transpose()?;
            // None deliberately stays None: primitive aliases and subsequent
            // owned circular machining must not become authored-sketch aliases.
            // Its unauthored full-circle rim reverses parameter direction in
            // this proper chart, retaining the exact circle, start and radius.
            let result = Self {
                body: SolidBody::AxialCylinder(cylinder.clone(), reflection.placed, sketch),
                policy,
                provenance: vec![],
                rust_origins: self.rust_origins.clone(),
                retained_placement: None,
            };
            result.circular_mirror_bounds(policy)?;
            Ok(result)
        })())
    }

    fn circular_mirror_policy(&self, policy: GeometryPolicy) -> Result<(), GeometryError> {
        if self.policy != policy {
            return Err(error(
                "mirror",
                GeometryErrorKind::Unsupported,
                "Rust Mirror requires the source geometry policy",
            ));
        }
        Ok(())
    }

    fn circular_mirror_bounds(&self, policy: GeometryPolicy) -> Result<(), GeometryError> {
        let bounds = self.bounds()?;
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(())
    }
}
