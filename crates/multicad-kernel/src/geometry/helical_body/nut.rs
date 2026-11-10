//! Planar exterior and native feature ancestry for a threaded hex nut.

use super::{
    boundary::{Boundary, edge_midpoint, hex_wall_plane},
    *,
};

impl Solid {
    /// Build a hex nut with six planar wrench faces and a modeled through-thread.
    pub fn rust_hex_nut(
        spec: HelicalThread,
        across_flats_mm: f64,
        minimum_wall_mm: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        policy.length(across_flats_mm)?;
        if !minimum_wall_mm.is_finite()
            || minimum_wall_mm < 0.
            || across_flats_mm / 2. - spec.major_radius_mm
                < minimum_wall_mm.max(policy.minimum_feature_mm)
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Input,
                "Hex nut flats leave insufficient wall around the thread",
            ));
        }
        Self::rust_helical_body(spec, None, feature, frame, policy, |thread| {
            Nut::new(thread, across_flats_mm)
                .map(HelicalForm::Nut)
                .map_err(|cause| match cause {
                    ThreadError::Invalid => error(
                        OPERATION,
                        GeometryErrorKind::Input,
                        "Invalid threaded hex nut geometry",
                    ),
                    ThreadError::Limit => error(
                        OPERATION,
                        GeometryErrorKind::Limit,
                        "Hex nut exceeds the geometry envelope",
                    ),
                })
        })
    }
}

pub(super) fn extend_face_properties(nut: &Nut, faces: &mut Vec<(Point, f64)>) {
    let width = nut.across_flats_mm();
    let length = nut.thread().spec().length_mm;
    let area = width * length / 3_f64.sqrt();
    faces.extend((0..6).map(|side| (edge_midpoint(width, side, length / 2.), area)));
}

pub(super) fn plane(nut: &Nut, side: Option<usize>, frame: PlaneFrame) -> Option<ReferenceFrame> {
    side.map(|side| hex_wall_plane(frame, nut.across_flats_mm(), side as u32, 0., 1.))
}

pub(super) fn extend_topology(
    body: &HelicalBody,
    nut: &Nut,
    frame: PlaneFrame,
    elements: &mut Vec<TopologyElement>,
) {
    let first_wall = 2 + body.rails.len() as u32;
    let width = nut.across_flats_mm();
    for side in 0..6 {
        let curve = body.origin(format!("hardware.outer/{side}"));
        let origin = body.origin(format!("side/{}/{}", curve.source, curve.role));
        let wall = hex_wall_plane(frame, width, side, 0., 1.);
        elements.push(TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: first_wall + side,
            },
            origins: BTreeSet::from([curve, origin.clone()]),
            boundary: BTreeSet::new(),
            frames: vec![AnchoredFrame {
                origin,
                frame: wall,
            }],
            geometry: Some(AnalyticGeometry::Plane { frame: wall }),
        });
    }

    let roles = (0..first_wall + 6).collect::<Vec<_>>();
    Boundary::new(body, frame, &roles, elements, true).hex_prism(
        "hardware.outer",
        width,
        [0., nut.thread().spec().length_mm],
        [0, 1],
        |side| first_wall + side,
    );
}
