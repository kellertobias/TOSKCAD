//! Conservative identities from complete finite physical boundaries.
use super::*;
use multicad_rust_kernel::normal_sweep::{Edge, Sweep};

fn numbers(values: impl IntoIterator<Item = f64>) -> Vec<u64> {
    values
        .into_iter()
        .map(|v| if v == 0. { 0 } else { v.to_bits() })
        .collect()
}

fn curve_key(curve: Curve, forward: bool, frame: PlaneFrame) -> Vec<u64> {
    let curve = match curve {
        Curve::Line { a_mm, b_mm } => {
            let ends = if forward { [a_mm, b_mm] } else { [b_mm, a_mm] };
            return std::iter::once(0)
                .chain(numbers(
                    ends.map(|p| position(frame, p)).into_iter().flatten(),
                ))
                .collect();
        }
        Curve::Profile { edge, z_mm } => (if forward { edge } else { edge.reversed() }, z_mm),
    };
    let (edge, z) = curve;
    let point = |p: [f64; 2]| position(frame, [p[0], p[1], z]);
    match edge {
        Edge::Line { a, b } => std::iter::once(0)
            .chain(numbers([point(a), point(b)].into_iter().flatten()))
            .collect(),
        Edge::Arc {
            center,
            a,
            b,
            clockwise,
        } => std::iter::once(1)
            .chain(numbers(
                [
                    point(center),
                    point(a),
                    point(b),
                    vector(frame, [0., 0., if clockwise { -1. } else { 1. }]),
                ]
                .into_iter()
                .flatten(),
            ))
            .collect(),
        Edge::Circle {
            center,
            radius_mm,
            clockwise,
        } => std::iter::once(2)
            .chain(numbers(
                point(center)
                    .into_iter()
                    .chain(vector(frame, [0., 0., if clockwise { -1. } else { 1. }]))
                    .chain([radius_mm]),
            ))
            .collect(),
    }
}

fn face_key(
    graph: &multicad_rust_kernel::normal_relief::Boundary,
    index: usize,
    frame: PlaneFrame,
) -> Vec<u64> {
    let face = &graph.faces[index];
    let mut key = match face.surface {
        Surface::Plane { origin_mm, u, v } => std::iter::once(0)
            .chain(numbers(
                [position(frame, origin_mm), vector(frame, cross(u, v))]
                    .into_iter()
                    .flatten(),
            ))
            .collect::<Vec<_>>(),
        Surface::Cylinder {
            origin_mm,
            axis,
            u,
            radius_mm,
            outward,
        } => std::iter::once(1)
            .chain(numbers(
                [
                    position(frame, origin_mm),
                    vector(frame, axis),
                    vector(frame, u),
                ]
                .into_iter()
                .flatten()
                .chain([radius_mm]),
            ))
            .chain([u64::from(outward)])
            .collect(),
    };
    let mut loops = face
        .loops
        .iter()
        .map(|wire| {
            let mut edges = wire
                .iter()
                .map(|e| curve_key(graph.edges[e.edge].curve, e.forward, frame))
                .collect::<Vec<_>>();
            // Rotate oriented wires, retaining direction and every curve. Tied
            // starting edges are compared over the complete cyclic sequence.
            let start = (0..edges.len())
                .min_by(|&a, &b| {
                    edges[a..]
                        .iter()
                        .chain(&edges[..a])
                        .cmp(edges[b..].iter().chain(&edges[..b]))
                })
                .unwrap_or(0);
            edges.rotate_left(start);
            let mut encoded = vec![edges.len() as u64];
            for edge in edges {
                encoded.push(edge.len() as u64);
                encoded.extend(edge);
            }
            encoded
        })
        .collect::<Vec<_>>();
    loops.sort();
    key.push(loops.len() as u64);
    for wire in loops {
        key.push(wire.len() as u64);
        key.extend(wire);
    }
    key
}

pub(in crate::geometry) fn imported_signature(relief: &Relief, frame: PlaneFrame) -> ContentHash {
    let graph = relief.boundary();
    let mut faces = (0..graph.faces.len())
        .map(|i| face_key(graph, i, frame))
        .collect::<Vec<_>>();
    faces.sort();
    let mut bytes = Vec::new();
    for face in faces {
        bytes.extend((face.len() as u64).to_le_bytes());
        for value in face {
            bytes.extend(value.to_le_bytes());
        }
    }
    ContentHash::of(&bytes)
}

impl Solid {
    pub(in crate::geometry) fn from_imported_normal_relief(
        relief: Relief,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        let invalid = || {
            error(
                "STEP import",
                GeometryErrorKind::InvalidResult,
                "Relief exceeds the active geometry policy",
            )
        };
        if feature.is_nil() {
            return Err(invalid());
        }
        let rebuild = |sweep: &Sweep| {
            policy.length(sweep.height_mm())?;
            Sweep::new(
                sweep.rings().to_vec(),
                sweep.height_mm(),
                policy.linear_tolerance_mm,
                policy.minimum_feature_mm,
            )
            .map_err(|_| invalid())
        };
        let base = rebuild(relief.base())?;
        let motifs = relief
            .motifs()
            .iter()
            .map(rebuild)
            .collect::<Result<Vec<_>, _>>()?;
        let geometry = Relief::new(
            base,
            motifs,
            relief.kind(),
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .map_err(|_| invalid())?;
        let graph = geometry.boundary();
        let whole = imported_signature(&geometry, frame);
        let labels = (0..graph.faces.len())
            .map(|i| {
                let bytes = face_key(graph, i, frame)
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>();
                Labels {
                    origins: BTreeSet::from([Origin {
                        source: feature,
                        role: format!("step.relief/{whole}/{}", ContentHash::of(&bytes)),
                    }]),
                    anchors: vec![],
                }
            })
            .collect();
        let edge_map = physical_edge_map(&geometry)?;
        let body = NormalReliefBody {
            geometry,
            feature,
            labels,
            edge_map,
        };
        frame.checked_metric_bounds(body.bounds(PlaneFrame::default()), policy)?;
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::NormalRelief(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }
}
