//! Rebuild only an actual constant cross-section, after full protected-tool proof.
use super::*;
use multicad_rust_kernel::{chamfered_bores::Counterbore, polygon::PolygonPrism};

impl Solid {
    pub fn rust_connection_counterbore(
        &self,
        tool: &CircularPocketTool,
        seat_radius_mm: f64,
        seat_depth_mm: f64,
        minimum_wall_mm: f64,
        seat_origin: Origin,
        policy: GeometryPolicy,
    ) -> Result<(Self, [Point; 2]), GeometryError> {
        policy.validate()?;
        policy.length(seat_radius_mm)?;
        policy.length(seat_depth_mm)?;
        if seat_radius_mm - tool.radius_mm < policy.minimum_feature_mm {
            return Err(invalid("A counterbore must be wider than its through bore"));
        }
        // In a constant section the complete enlarged protected disk fits at
        // every station. This also qualifies the original tool's full extent
        // at the largest radius before adopting the stock construction chart.
        let (drilled, mouths) =
            self.connection_through_bore_protected(tool, minimum_wall_mm, seat_radius_mm, policy)?;
        let (body, placement) = drilled.orthogonal_body().expect("receiving stock");
        let bore = body.through_bores[0];
        let [start, end] = bore.centers_mm;
        let axis = (0..3)
            .find(|&i| start[i] != end[i])
            .expect("positive bore length");
        let sign = (end[axis] - start[axis]).signum();
        let transverse = [(axis + 1) % 3, (axis + 2) % 3];
        let source_axes = [placement.u, placement.v, placement.normal()];
        let mut local_origin = [0.; 3];
        local_origin[axis] = start[axis];
        let frame = PlaneFrame {
            origin: transform_point(placement, local_origin),
            u: source_axes[transverse[0]],
            v: source_axes[transverse[1]].map(|x| sign * x),
        };
        let project = |p: Point| [p[transverse[0]], sign * p[transverse[1]]];
        let cap = &body.boundary.faces[bore.faces[0]];
        for (i, face) in body.boundary.faces.iter().enumerate() {
            if face.normal[axis] != 0. && !bore.faces.contains(&i) {
                return Err(unsupported(
                    "Counterbore stock has intermediate axial faces",
                ));
            }
        }
        // The two complete cap wires must be literal translations. An
        // envelope or matching area cannot stand in for the material section.
        let boundary_key = |face: &BoundaryFace| {
            let mut edges = Vec::new();
            for ring in &face.rings {
                for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    let [a, b] = [project(a), project(b)];
                    let mut ends = [a, b].map(|p| p.map(|x| if x == 0. { 0 } else { x.to_bits() }));
                    ends.sort();
                    edges.push(ends);
                }
            }
            edges.sort();
            edges
        };
        if boundary_key(cap) != boundary_key(&body.boundary.faces[bore.faces[1]]) {
            return Err(unsupported(
                "Counterbore stock caps have different complete sections",
            ));
        }
        let dimension = |x| {
            rational_coordinate(x, policy.linear_tolerance_mm / 16.).ok_or_else(|| {
                unsupported(
                    "Counterbore stock cannot retain its section within the construction budget",
                )
            })
        };
        let mut rings = cap
            .rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|&p| {
                        let [x, y] = project(p);
                        Ok([dimension(x)?, dimension(y)?])
                    })
                    .collect::<Result<Vec<_>, GeometryError>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Entry cap wires face out of the solid, opposite the new sweep.
        for ring in &mut rings {
            ring.reverse();
        }
        let center = project(start);
        let circle = Circle::new(
            [dimension(center[0])?, dimension(center[1])?],
            dimension(tool.radius_mm)?,
        )
        .map_err(|cause| invalid(format!("Invalid receiving circle: {cause:?}")))?;
        let prism =
            PolygonPrism::with_circular_holes(rings, vec![circle], dimension(bore.depth_mm())?)
                .map_err(|cause| unsupported(&format!("Counterbore stock section: {cause:?}")))?;
        for (ring, source) in prism.rings().iter().zip(&cap.rings) {
            for (&p, &q) in ring.iter().zip(source.iter().rev()) {
                for end in [0, 1] {
                    let mut original = q;
                    original[axis] = bore.centers_mm[end][axis];
                    let z = if end == 0 {
                        Rational::new(0, 1).expect("zero")
                    } else {
                        prism.height()
                    };
                    prove_point(placement, original, frame, [p[0], p[1], z], policy)?;
                }
            }
        }
        for end in [0, 1] {
            let z = if end == 0 {
                Rational::new(0, 1).expect("zero")
            } else {
                prism.height()
            };
            let c = prism.circular_holes()[0].center();
            prove_point(
                placement,
                bore.centers_mm[end],
                frame,
                [c[0], c[1], z],
                policy,
            )?;
        }
        let radius = prism.circular_holes()[0].radius().as_f64();
        if (radius - tool.radius_mm).abs() + f64::EPSILON * radius
            > policy.linear_tolerance_mm / 64.
        {
            return Err(unsupported(
                "Receiving radius exceeds its construction budget",
            ));
        }
        let mut origins: Vec<_> = (0..prism.rings().iter().map(Vec::len).sum::<usize>())
            .map(|i| Origin {
                source: body.feature,
                role: format!("receiving.section.{i}"),
            })
            .collect();
        origins.push(tool.origin.clone());
        let base = Self::from_polygon_prism(
            prism,
            frame,
            seat_origin.source,
            &origins,
            &vec![false; origins.len()],
            policy,
        )?;
        let result = base.rust_counterbore_polygon(
            &[Counterbore {
                hole: 0,
                end: false,
                radius_mm: seat_radius_mm,
                depth_mm: seat_depth_mm,
            }],
            std::slice::from_ref(&seat_origin),
            policy,
        )?;
        let mut side_sources = Vec::new();
        for ring in &cap.rings {
            let reversed: Vec<_> = ring.iter().copied().rev().collect();
            for (&a, &b) in reversed.iter().zip(reversed.iter().cycle().skip(1)) {
                let mut matches = body
                    .boundary
                    .faces
                    .iter()
                    .enumerate()
                    .filter_map(|(i, face)| {
                        if face.normal[axis] != 0. {
                            return None;
                        }
                        face.rings
                            .iter()
                            .any(|ring| {
                                ring.iter()
                                    .zip(ring.iter().cycle().skip(1))
                                    .any(|(&p, &q)| (p == a && q == b) || (p == b && q == a))
                            })
                            .then_some(i)
                    });
                let source = matches.next().ok_or_else(|| {
                    unsupported("Receiving profile edge has no complete source face")
                })?;
                if matches.next().is_some() {
                    return Err(unsupported(
                        "Receiving profile edge has ambiguous source faces",
                    ));
                }
                side_sources.push(source);
            }
        }
        let result = inherit_receiver_faces(result, &drilled, self, body, bore, &side_sources)?;
        Ok((result, mouths))
    }
}

fn world_with_error(frame: PlaneFrame, p: Point) -> (Point, Point) {
    use crate::geometry::arithmetic::{dot_error, sum_error};
    let axes = [frame.u, frame.v, frame.normal()];
    let terms: [(f64, f64); 3] = std::array::from_fn(|i| {
        let (dot, error) = dot_error(axes.map(|axis| axis[i]), p);
        let (sum, addition) = sum_error(frame.origin[i], dot);
        (sum, (error + addition).next_up())
    });
    (terms.map(|v| v.0), terms.map(|v| v.1))
}

fn prove_point(
    original_frame: PlaneFrame,
    original: Point,
    frame: PlaneFrame,
    p: [Rational; 3],
    policy: GeometryPolicy,
) -> Result<(), GeometryError> {
    use crate::geometry::arithmetic::difference_with_error;
    let (a, ae) = world_with_error(original_frame, original);
    let rounded = p.map(Rational::as_f64);
    let (b, be) = world_with_error(frame, rounded);
    let (difference, error) = difference_with_error(a, b);
    // as_f64 rounds each exact rational once. Include its rounding reserve
    // as well as the true affine dot/add and subtraction errors above.
    let input = (rounded.into_iter().fold(0_f64, |sum, x| {
        (sum + (f64::EPSILON * x.abs()).next_up()).next_up()
    }) * 2.)
        .next_up();
    let deviation = (0..3).fold(input, |sum, i| {
        (sum + difference[i].abs() + error[i] + ae[i] + be[i]).next_up()
    });
    if !deviation.is_finite() || deviation > policy.linear_tolerance_mm / 16. {
        return Err(unsupported(
            "Receiving section cannot retain its complete physical boundary within the construction budget",
        ));
    }
    Ok(())
}

fn inherit_receiver_faces(
    mut result: Solid,
    source: &Solid,
    original: &Solid,
    body: &OrthogonalBody,
    bore: OrthogonalThroughBore,
    side_sources: &[usize],
) -> Result<Solid, GeometryError> {
    let owner = EntityId::new();
    let old = source.topology_snapshot(owner, 0)?;
    let original = original.topology_snapshot(owner, 0)?;
    let mut anchors = BTreeMap::<Origin, Option<ReferenceFrame>>::new();
    for frame in original.elements.iter().flat_map(|e| &e.frames) {
        anchors
            .entry(frame.origin.clone())
            .and_modify(|old| {
                if *old != Some(frame.frame) {
                    *old = None;
                }
            })
            .or_insert(Some(frame.frame));
    }
    let mut mapped = BTreeMap::from([(0, bore.faces[0]), (1, bore.faces[1])]);
    mapped.extend(
        side_sources
            .iter()
            .enumerate()
            .map(|(i, &source)| (i + 2, source)),
    );
    mapped.insert(side_sources.len() + 2, body.faces.len());
    let snapshot = result.topology_snapshot(owner, 0)?;
    let mut names = snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .map(|e| e.origins.iter().cloned().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    for (&target, &source) in &mapped {
        let original = old
            .elements
            .iter()
            .find(|e| {
                e.key
                    == TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: source as u32,
                    }
            })
            .ok_or_else(|| invalid("Receiving face source is absent"))?;
        names[target] = original.origins.iter().cloned().collect();
    }
    let SolidBody::PolygonPrism(placed) = &mut result.body else {
        unreachable!("constructed receiving prism")
    };
    let placed = Rc::make_mut(placed);
    Rc::make_mut(placed.bore_treatments.as_mut().expect("new counterbore")).face_origins =
        Some(names);
    result.provenance = result
        .topology_snapshot(owner, 0)?
        .elements
        .into_iter()
        .map(|mut e| {
            if e.key.kind == TopologyKind::Face
                && let Some(&source) = mapped.get(&(e.key.ordinal as usize))
            {
                let key = TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: source as u32,
                };
                e.frames.clone_from(
                    &old.elements
                        .iter()
                        .find(|e| e.key == key)
                        .expect("checked source face")
                        .frames,
                );
                for frame in &mut e.frames {
                    if let Some(Some(original)) = anchors.get(&frame.origin) {
                        frame.frame = *original;
                    }
                }
            }
            e.geometry = None;
            e
        })
        .collect();
    result.rust_origins.clear();
    result.topology_snapshot(owner, 0)?;
    Ok(result)
}
