//! Keep STEP representation members independent and their topology identities distinct.
use super::*;
use std::collections::BTreeMap;

fn record(tag: u64, coordinates: impl IntoIterator<Item = f64>) -> [u64; 11] {
    let mut record = [0; 11];
    record[0] = tag;
    for (slot, value) in record[1..].iter_mut().zip(coordinates) {
        *slot = if value == 0. { 0 } else { value.to_bits() };
    }
    record
}

fn unsigned_normal(normal: Point) -> Point {
    if normal
        .into_iter()
        .find(|&value| value != 0.)
        .is_some_and(|value| value < 0.)
    {
        normal.map(|value| -value)
    } else {
        normal
    }
}

// Use world geometry instead of STEP record numbers or the order of other
// members. An update must not bind a face to a different shell just because
// its shell was moved to the same list position.
fn member_signature(readback: &Readback, imported: &ImportSolid) -> ContentHash {
    let lines = readback.edges_mm.iter().map(|&points| {
        let mut points = points;
        points.sort_unstable_by(compare_points);
        record(1, points.into_iter().flatten())
    });
    let circles = readback.circles.iter().map(|rim| {
        record(
            2,
            rim.center_mm
                .into_iter()
                .chain(unsigned_normal(rim.normal))
                .chain([rim.radius_mm]),
        )
    });
    let halves = readback.semicircles.iter().map(|rim| {
        record(
            3,
            rim.center_mm
                .into_iter()
                .chain(unsigned_normal(rim.normal))
                .chain([rim.radius_mm])
                .chain(rim.radial_mid),
        )
    });
    let arcs = readback.arcs.iter().map(|rim| {
        let mut ends = [rim.radial_start, rim.radial_end];
        ends.sort_unstable_by(compare_points);
        let radius = if rim.sweep_rad > PI {
            -rim.radius_mm
        } else {
            rim.radius_mm
        };
        record(
            4,
            rim.center_mm
                .into_iter()
                .chain(ends.into_iter().flatten())
                .chain([radius]),
        )
    });
    let mut edges: Vec<_> = lines.chain(circles).chain(halves).chain(arcs).collect();
    edges.sort_unstable();
    let mut bytes = Vec::with_capacity(64 + edges.len() * 11 * 8);
    bytes.extend((readback.face_count as u64).to_le_bytes());
    bytes.extend(readback.volume_mm3.to_le_bytes());
    for value in readback.bounds_mm.into_iter().flatten() {
        bytes.extend(if value == 0. { 0. } else { value }.to_le_bytes());
    }
    for entry in edges.into_iter().flatten() {
        bytes.extend(entry.to_le_bytes());
    }
    let mut ellipses = readback
        .ellipses
        .iter()
        .map(|arc| {
            let geometry = arc.ellipse();
            let mut ends = arc.endpoints();
            ends.sort_unstable_by(compare_points);
            ends.into_iter()
                .flatten()
                .chain(geometry.center_mm())
                .chain(unsigned_normal(geometry.major_axis()))
                .chain(geometry.semiaxes_mm())
                .chain(geometry.point_at(arc.start_rad() + arc.sweep_rad() * 0.5))
                .chain([arc.sweep_rad().abs()])
                .map(|value| if value == 0. { 0 } else { value.to_bits() })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    ellipses.sort_unstable();
    for entry in ellipses.into_iter().flatten() {
        bytes.extend(entry.to_le_bytes());
    }
    let mut splines = readback
        .splines
        .iter()
        .map(|curve| spline_control_key(curve.poles_mm(), curve.knots()))
        .collect::<Vec<_>>();
    splines.sort_unstable();
    for spline in splines {
        bytes.extend((spline.len() as u64).to_le_bytes());
        for entry in spline {
            bytes.extend(entry.to_le_bytes());
        }
    }
    if let ImportProfile::HelicalTubeCut {
        stock_radius_mm,
        radius_mm,
        pitch_mm,
        start_height_mm,
        start_angle_rad,
        ..
    } = imported.profile
    {
        // The groove's analytic rails and cap spline have no compact
        // public edge records. Their reconstructed parameters identify
        // the removed material even when aggregate dimensions coincide.
        bytes.extend(b"helical-tube-cut");
        for value in imported.u.into_iter().chain(imported.v).chain([
            stock_radius_mm,
            radius_mm,
            pitch_mm,
            start_height_mm,
            start_angle_rad,
        ]) {
            bytes.extend(if value == 0. { 0. } else { value }.to_le_bytes());
        }
    }
    if let ImportProfile::HelicalThread {
        spec,
        stock_radius_mm,
    } = imported.profile
    {
        bytes.extend(b"helical-thread");
        for value in imported
            .origin_mm
            .into_iter()
            .chain(imported.u)
            .chain(imported.v)
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
                stock_radius_mm.unwrap_or(0.),
            ])
        {
            bytes.extend(if value == 0. { 0. } else { value }.to_le_bytes());
        }
    }
    if let ImportProfile::HelicalScrew { spec, screw } = imported.profile {
        bytes.extend(b"helical-screw");
        let head = match screw.head {
            multicad_rust_kernel::headed_screw::ScrewHead::ExternalHex {
                across_flats_mm,
                height_mm,
            } => [across_flats_mm, height_mm, 0., 0.],
            multicad_rust_kernel::headed_screw::ScrewHead::SocketCap {
                radius_mm,
                height_mm,
                socket_across_flats_mm,
                socket_depth_mm,
            } => [
                radius_mm,
                height_mm,
                socket_across_flats_mm,
                socket_depth_mm,
            ],
        };
        for value in imported
            .origin_mm
            .into_iter()
            .chain(imported.u)
            .chain(imported.v)
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
                screw.shank_length_mm,
                screw.shank_radius_mm,
            ])
            .chain(head)
        {
            bytes.extend(if value == 0. { 0. } else { value }.to_le_bytes());
        }
    }
    ContentHash::of(&bytes)
}

pub(super) fn rebuild(
    members: Vec<(Readback, ImportSolid)>,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    let mut occurrences = BTreeMap::<ContentHash, u32>::new();
    let mut solids = Vec::with_capacity(members.len());
    for (readback, imported) in members {
        let signature = member_signature(&readback, &imported);
        let occurrence = occurrences.entry(signature).or_default();
        let mut bytes = b"multicad.step-member/1".to_vec();
        bytes.extend(feature.uuid_bits().to_le_bytes());
        bytes.extend(signature.0);
        bytes.extend(occurrence.to_le_bytes());
        *occurrence += 1;
        let hash = ContentHash::of(&bytes);
        let child = EntityId::from_uuid_bits(u128::from_le_bytes(hash.0[..16].try_into().unwrap()));
        solids.push(Solid::verified_step_member(
            &readback, imported, child, policy,
        )?);
    }
    let members: Vec<_> = solids.iter().collect();
    Solid::make_rust_compound(&members, feature, policy, None)
}

impl Solid {
    /// Check each emitted STEP shell against the same member's exact face and
    /// edge ordinals in the published compound. The writer preserves member order.
    pub fn step_compound_topology_matches(
        topology: &TopologySnapshot,
        readbacks: &[Readback],
        tolerance: f64,
    ) -> bool {
        let readbacks: Vec<_> = readbacks.iter().collect();
        match_compound(topology, &readbacks, tolerance, None)
    }

    /// Compare exported member shells with the retained solid, including
    /// spline control nets that have no compact public topology descriptor.
    pub fn step_readback_matches(
        &self,
        topology: &TopologySnapshot,
        readbacks: &[(Readback, Option<ImportSolid>)],
        tolerance: f64,
    ) -> bool {
        if !tolerance.is_finite() || tolerance <= 0. || readbacks.is_empty() {
            return false;
        }
        let mut leaves = Vec::with_capacity(readbacks.len());
        self.step_leaf_members(&mut leaves);
        let planar_members = |leaf: &Solid| {
            leaf.planar_boolean_body()
                .map_or(1, |(body, _)| body.geometry.solid_count())
        };
        if readbacks.iter().all(|(_, profile)| profile.is_none())
            && leaves.iter().all(|leaf| planar_members(leaf) == 1)
        {
            let bodies: Vec<_> = leaves
                .iter()
                .map(|solid| solid.spline_sweep_body())
                .collect();
            let readbacks: Vec<_> = readbacks.iter().map(|(readback, _)| readback).collect();
            return match_compound(topology, &readbacks, tolerance, Some(&bodies));
        }

        // A planar leaf can contribute several disconnected solids. Internal
        // void shells belong to their exterior solid's single STEP record.
        // A helical cut can add a seam and split its exact source edges.
        let mut remaining = readbacks;
        for leaf in leaves {
            let Some((members, rest)) = remaining.split_at_checked(planar_members(leaf)) else {
                return false;
            };
            if !match_leaf(leaf, members, tolerance) {
                return false;
            }
            remaining = rest;
        }
        remaining.is_empty()
    }

    /// Verify a flat compound containing authored twisted gears and analytic
    /// members, preserving the STEP writer's order and source placements.
    pub fn verify_twisted_step_compound(
        &self,
        input: &str,
        tolerance: f64,
    ) -> Option<std::result::Result<StepCompoundMetrics, ReadError>> {
        use step_read::{ExpectedStepMember, VerifiedStepMember};
        self.rust_compound()?;
        let mut leaves = Vec::new();
        self.step_leaf_members(&mut leaves);
        if !leaves.iter().any(|leaf| leaf.herringbone_body().is_some()) {
            return None;
        }
        Some((|| {
            let mut expected = Vec::new();
            let mut member_counts = Vec::with_capacity(leaves.len());
            for leaf in &leaves {
                if let Some((body, frame)) = leaf.herringbone_body() {
                    member_counts.push(1);
                    expected.push(ExpectedStepMember::TwistedGear {
                        gear: &body.geometry,
                        origin: frame.origin,
                        u: frame.u,
                        v: frame.v,
                    });
                } else {
                    let count = leaf.planar_boolean_body().map_or_else(
                        || {
                            leaf.solid_count()
                                .map(|count| count as usize)
                                .map_err(|_| ReadError::Invalid)
                        },
                        |(body, _)| Ok(body.geometry.solid_count()),
                    )?;
                    member_counts.push(count);
                    expected.extend((0..count).map(|_| ExpectedStepMember::Analytic));
                }
            }
            let mut decoded =
                step_read::verify_authored_compound(input, &expected, tolerance)?.into_iter();
            let mut result = (0_u32, 0_f64, [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]]);
            for (leaf, member_count) in leaves.into_iter().zip(member_counts) {
                let (count, volume, bounds) = if leaf.herringbone_body().is_some() {
                    let Some(VerifiedStepMember::TwistedGear { volume_mm3 }) = decoded.next()
                    else {
                        return Err(ReadError::Invalid);
                    };
                    let bounds = leaf.bounds().map_err(|_| ReadError::Invalid)?;
                    (1, volume_mm3, [bounds.min, bounds.max])
                } else {
                    let mut members = Vec::with_capacity(member_count);
                    for _ in 0..member_count {
                        let Some(VerifiedStepMember::Analytic { readback, imported }) =
                            decoded.next()
                        else {
                            return Err(ReadError::Invalid);
                        };
                        members.push((*readback, imported.map(|value| *value)));
                    }
                    let topology = leaf
                        .rust_topology()
                        .ok_or(ReadError::Invalid)?
                        .map_err(|_| ReadError::Invalid)?;
                    if !leaf.step_readback_matches(&topology, &members, tolerance * 2.) {
                        return Err(ReadError::Invalid);
                    }
                    members.into_iter().fold(
                        (0_u32, 0., [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]]),
                        |(count, volume, [min, max]), (member, _)| {
                            (
                                count + member.solid_count,
                                volume + member.volume_mm3,
                                [
                                    std::array::from_fn(|i| min[i].min(member.bounds_mm[0][i])),
                                    std::array::from_fn(|i| max[i].max(member.bounds_mm[1][i])),
                                ],
                            )
                        },
                    )
                };
                result.0 = result.0.checked_add(count).ok_or(ReadError::Limit)?;
                result.1 += volume;
                if !result.1.is_finite() {
                    return Err(ReadError::Invalid);
                }
                result.2 = [
                    std::array::from_fn(|i| result.2[0][i].min(bounds[0][i])),
                    std::array::from_fn(|i| result.2[1][i].max(bounds[1][i])),
                ];
            }
            if decoded.next().is_some() {
                return Err(ReadError::Invalid);
            }
            Ok(result)
        })())
    }

    fn step_leaf_members<'a>(&'a self, leaves: &mut Vec<&'a Solid>) {
        if let Some(compound) = self.rust_compound() {
            for member in compound.members() {
                member.step_leaf_members(leaves);
            }
        } else {
            leaves.push(self);
        }
    }
}

fn match_leaf(leaf: &Solid, members: &[(Readback, Option<ImportSolid>)], tolerance: f64) -> bool {
    let [(readback, profile)] = members else {
        let Some((body, frame)) = leaf.planar_boolean_body() else {
            return false;
        };
        return body.geometry.shell_count() == body.geometry.solid_count()
            && body
                .geometry
                .shells()
                .iter()
                .zip(members)
                .all(|(shell, (readback, profile))| {
                    profile.is_none() && match_planar_shell(body, frame, shell, readback, tolerance)
                });
    };
    if readback.solid_count != 1 || verify_step_metrics(leaf, readback, tolerance).is_err() {
        return false;
    }
    if let Some(profile) = profile {
        return leaf.matches_imported_helical_thread(profile, tolerance)
            || leaf.matches_helical_cut_profile(profile);
    }
    leaf.rust_topology().is_some_and(|topology| {
        topology.is_ok_and(|topology| {
            compare_topology(
                &topology.elements,
                readback,
                tolerance,
                false,
                leaf.spline_sweep_body(),
            )
        })
    })
}

fn match_planar_shell(
    body: &PlanarBooleanBody,
    frame: PlaneFrame,
    shell: &multicad_rust_kernel::planar_boolean::Shell,
    readback: &Readback,
    tolerance: f64,
) -> bool {
    if readback.solid_count != 1
        || readback.face_count != shell.faces.len()
        || !readback.circles.is_empty()
        || !readback.semicircles.is_empty()
        || !readback.arcs.is_empty()
        || !readback.ellipses.is_empty()
        || !readback.splines.is_empty()
    {
        return false;
    }
    let segments = body.step_shell_segments(frame, &shell.faces);
    let mut bounds = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for point in segments.iter().flatten() {
        for (axis, &value) in point.iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(value);
            bounds.max[axis] = bounds.max[axis].max(value);
        }
    }
    verify_shell_metrics(bounds, shell.signed_volume_mm3, readback, tolerance).is_ok()
        && same_segments(&readback.edges_mm, &segments, readback.bounds_mm, tolerance)
}

fn match_compound(
    topology: &TopologySnapshot,
    readbacks: &[&Readback],
    tolerance: f64,
    spline_bodies: Option<&[Option<(&SplineSweepBody, PlaneFrame)>]>,
) -> bool {
    if !tolerance.is_finite()
        || tolerance <= 0.
        || readbacks.is_empty()
        || spline_bodies.is_some_and(|bodies| bodies.len() != readbacks.len())
    {
        return false;
    }
    let spline_body = |index| spline_bodies.and_then(|bodies| bodies[index]);
    if let [readback] = readbacks {
        return readback.solid_count == 1
            && compare_topology(
                &topology.elements,
                readback,
                tolerance,
                false,
                spline_body(0),
            );
    }
    let mut ends = Vec::with_capacity(readbacks.len());
    let mut end = [0usize; 2];
    for readback in readbacks {
        if readback.solid_count != 1 || readback.face_count == 0 {
            return false;
        }
        end[0] += readback.face_count;
        end[1] += readback.edges_mm.len()
            + readback.circles.len()
            + readback.semicircles.len()
            + readback.arcs.len()
            + readback.ellipses.len()
            + readback.splines.len();
        ends.push(end);
    }
    let mut members = vec![Vec::new(); readbacks.len()];
    for element in &topology.elements {
        let kind = match element.key.kind {
            TopologyKind::Face => 0,
            TopologyKind::Edge => 1,
            TopologyKind::Vertex => continue,
        };
        let index = ends.partition_point(|end| end[kind] <= element.key.ordinal as usize);
        let Some(member) = members.get_mut(index) else {
            return false;
        };
        member.push(element);
    }
    members
        .into_iter()
        .zip(readbacks)
        .enumerate()
        .all(|(index, (elements, readback))| {
            compare_topology(elements, readback, tolerance, false, spline_body(index))
        })
}
