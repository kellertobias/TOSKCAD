//! Exact reflection of a complete circular ring and its authored attachments.
//! Authored U and negative reflected V restore the full basis after two Mirrors.
use super::*;

impl Solid {
    pub(crate) fn mirror_annulus(
        &self,
        plane: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        let (body, frame) = self.annulus_body()?;
        Some((|| {
            if self.policy != policy {
                return Err(error(
                    "mirror",
                    GeometryErrorKind::Unsupported,
                    "Rust Mirror requires the source geometry policy",
                ));
            }
            let normal = plane.normal();
            let squared = dot(normal, normal);
            let vector = |value: [f64; 3]| {
                let distance = 2. * dot(normal, value) / squared;
                std::array::from_fn(|i| value[i] - distance * normal[i])
            };
            let point = |value: [f64; 3]| {
                let reflected = vector(value);
                let distance = 2. * dot(normal, plane.origin) / squared;
                std::array::from_fn(|i| reflected[i] + distance * normal[i])
            };
            // Full circular sections are invariant under a local V reflection.
            // The two orientation reversals keep the placed body right-handed.
            let placed = PlaneFrame {
                origin: point(frame.origin),
                u: vector(frame.u),
                v: vector(frame.v).map(|value| -value),
            };
            placed.checked(policy)?;
            let axes = [placed.u, placed.v, placed.normal()];
            // Checked charts admit tiny metric residuals. Use the actual
            // inverse so far authored UV origins survive a frame round trip.
            let cofactors = [
                cross(axes[1], axes[2]),
                cross(axes[2], axes[0]),
                cross(axes[0], axes[1]),
            ];
            let determinant = dot(axes[0], cofactors[0]);
            let inverse = cofactors.map(|row| row.map(|value| value / determinant));
            let authored_v = frame
                .v
                .map(|value| if body.reverse { -value } else { value });
            let anchors = std::array::from_fn(|cap| {
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
                let anchor = body.cap_anchor(frame, cap, center, authored_v);
                let reflected = ReferenceFrame {
                    origin_mm: point(anchor.origin_mm),
                    u: vector(anchor.u),
                    v: vector(anchor.v).map(|value| -value),
                };
                let delta = std::array::from_fn(|i| reflected.origin_mm[i] - placed.origin[i]);
                ReferenceFrame {
                    origin_mm: inverse.map(|row| dot(row, delta)),
                    u: inverse.map(|row| dot(row, reflected.u)),
                    v: inverse.map(|row| dot(row, reflected.v)),
                }
            });
            let mirrored = AnnulusBody {
                geometry: body.geometry,
                feature: body.feature,
                wall_origins: body.wall_origins.clone(),
                center_uv: body.center_uv,
                reverse: !body.reverse,
                cap_anchors: Some(anchors),
            };
            let bounds = mirrored.bounds(placed);
            policy.point(bounds.min)?;
            policy.point(bounds.max)?;
            Ok(Self {
                body: SolidBody::Annulus(Rc::new(mirrored), placed),
                policy,
                provenance: vec![],
                rust_origins: self.rust_origins.clone(),
            })
        })())
    }
}
