//! Conservative identities for partitions not named by semantic ancestry.
use super::*;
use multicad_core::project_file::ContentHash;

// Charge indexing and candidate comparisons cumulatively. No quadratic scan
// over unrelated source surfaces, and no unbounded retry of ambiguous charts.
const MAX_WORK: usize = 24_000_000;
fn charge(work: &mut usize, amount: usize) -> Result<(), GeometryError> {
    *work = work
        .checked_sub(amount)
        .ok_or_else(|| planar_error(Error::Limit))?;
    Ok(())
}

/// Match the actual intent contract: origin subset, then exact boundary when
/// origin ancestry alone is not unique. Equality-only buckets are insufficient.
fn ambiguous(snapshot: &TopologySnapshot, work: &mut usize) -> Result<Vec<usize>, GeometryError> {
    let mut index = BTreeMap::<(TopologyKind, &Origin), Vec<usize>>::new();
    for (i, element) in snapshot.elements.iter().enumerate() {
        if element.origins.len() > MAX_GEOMETRY_ORIGINS
            || element.boundary.len() > MAX_GEOMETRY_ORIGINS
        {
            return Err(planar_error(Error::Limit));
        }
        charge(work, element.origins.len() + 1)?;
        for origin in &element.origins {
            index.entry((element.key.kind, origin)).or_default().push(i);
        }
    }
    let mut ambiguous = Vec::new();
    for (element_index, element) in snapshot.elements.iter().enumerate() {
        let candidates = element
            .origins
            .iter()
            .map(|origin| &index[&(element.key.kind, origin)])
            .min_by_key(|candidates| candidates.len())
            .ok_or_else(|| planar_error(Error::Invalid))?;
        let (mut ancestry_count, mut bounded_count) = (0, 0);
        for &candidate in candidates {
            charge(work, element.origins.len() + 1)?;
            let candidate = &snapshot.elements[candidate];
            if element.origins.is_subset(&candidate.origins) {
                ancestry_count += 1;
                charge(work, element.boundary.len() + 1)?;
                if candidate.boundary == element.boundary {
                    bounded_count += 1;
                }
            }
        }
        if ancestry_count > 1 && bounded_count > 1 {
            ambiguous.push(element_index);
        }
    }
    Ok(ambiguous)
}

pub(super) fn label_ambiguous(
    body: &mut PlanarBooleanBody,
    fallback: &[FaceLabels],
) -> Result<(), GeometryError> {
    let mut work = MAX_WORK;
    let snapshot = body.topology(PlaneFrame::default(), body.feature, 0);
    for index in ambiguous(&snapshot, &mut work)? {
        let key = snapshot.elements[index].key;
        if key.kind == TopologyKind::Face {
            // Preserve inherited source ancestry and anchors. Only the opaque
            // finite partition gets this additional exact region identity.
            body.faces[key.ordinal as usize]
                .origins
                .extend(fallback[key.ordinal as usize].0.iter().cloned());
        }
    }
    // Face labels must settle before edge/vertex provenance is constructed.
    let snapshot = body.topology(PlaneFrame::default(), body.feature, 0);
    for index in ambiguous(&snapshot, &mut work)? {
        let element = &snapshot.elements[index];
        let key = element.key;
        body.finite_origins
            .insert(key, finite_origin(body.feature, element.geometry)?);
    }
    let final_snapshot = body.topology(PlaneFrame::default(), body.feature, 0);
    if !ambiguous(&final_snapshot, &mut work)?.is_empty() {
        return Err(planar_error(Error::Unresolved));
    }
    Ok(())
}

fn finite_origin(
    feature: EntityId,
    geometry: Option<AnalyticGeometry>,
) -> Result<Origin, GeometryError> {
    let (prefix, points) = match geometry {
        Some(AnalyticGeometry::Line { a_mm, b_mm }) => ("derived-segment.v1.", vec![a_mm, b_mm]),
        Some(AnalyticGeometry::Point { position_mm }) => ("derived-point.v1.", vec![position_mm]),
        _ => return Err(planar_error(Error::Unresolved)),
    };
    let mut points: Vec<_> = points
        .into_iter()
        .map(|point| point.map(|v| if v == 0. { 0 } else { v.to_bits() }))
        .collect();
    points.sort_unstable();
    let mut bytes = prefix.as_bytes().to_vec();
    for point in points {
        for value in point {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(Origin {
        source: feature,
        role: format!("{prefix}{}", ContentHash::of(&bytes)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::references::TopologyElement;
    #[test]
    fn opposite_material_sides_of_same_source_patch_do_not_share_fragment_identity() {
        use multicad_core::references::{GeometryIntent, Resolution, resolve};
        let policy = GeometryPolicy::default();
        let stock = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [30, 20, 10].map(|n| multicad_rust_kernel::Rational::new(n, 1).unwrap()),
            PlaneFrame::default(),
            policy,
        )
        .unwrap();
        let tool = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [10, 20, 10].map(|n| multicad_rust_kernel::Rational::new(n, 1).unwrap()),
            PlaneFrame {
                origin: [10., 0., 0.],
                ..Default::default()
            },
            policy,
        )
        .unwrap();
        let feature = EntityId::new();
        let make = |cut: bool| {
            let mut operands = PlanarOperands::default();
            operands.append(&stock, 1., policy).unwrap();
            operands
                .append(&tool, if cut { -1. } else { 1. }, policy)
                .unwrap();
            let geometry = if cut {
                Union::subtract(
                    &operands.pieces[..1],
                    &operands.pieces[1..],
                    policy.linear_tolerance_mm,
                )
            } else {
                Union::intersect(
                    &operands.pieces[..1],
                    &operands.pieces[1..],
                    policy.linear_tolerance_mm,
                )
            }
            .unwrap()
            .unwrap();
            PlanarBooleanBody::build(
                geometry,
                operands.sources,
                &operands.topologies,
                feature,
                policy,
                None,
            )
            .unwrap()
        };
        let cut = make(true);
        let face = cut
            .faces()
            .unwrap()
            .into_iter()
            .find(|f| (f.center_mm[0] - 10.).abs() < 1e-8 && f.plane.unwrap().1[0] > 0.99)
            .unwrap();
        let before = cut.topology_snapshot(feature, 1).unwrap();
        let element = before
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == face.index)
            .unwrap();
        let intent = GeometryIntent::capture(&before, element.key).unwrap();
        let common = make(false);
        assert!(
            common
                .faces()
                .unwrap()
                .iter()
                .any(|f| (f.center_mm[0] - 10.).abs() < 1e-8 && f.plane.unwrap().1[0] < -0.99)
        );
        assert!(matches!(
            resolve(&intent, &common.topology_snapshot(feature, 2).unwrap(), 2),
            Resolution::Missing
        ));
    }
    #[test]
    fn exact_fallback_names_opaque_regions_and_survives_post_scale_factor_edits() {
        use multicad_core::references::{GeometryIntent, Resolution, resolve};
        let policy = GeometryPolicy::default();
        let feature = EntityId::new();
        let piece = ConvexPiece::from_box([0.; 3], [2.; 3], policy.linear_tolerance_mm).unwrap();
        let geometry =
            Union::new(std::slice::from_ref(&piece), policy.linear_tolerance_mm).unwrap();
        let fallback =
            imported_identity::labels_with_prefix(&geometry, feature, "derived-plane.v1.").unwrap();
        // Test the bounded identity fallback directly, with deliberately coarse
        // lineage on a fully certified exact cube; no topology is fabricated.
        let mut source = PlanarBooleanBody::build_labeled(
            geometry,
            feature,
            policy,
            Some(Material::Positive(vec![piece])),
            PlaneFrame::default(),
            |_, _, _| {
                (
                    vec![Origin {
                        source: feature,
                        role: "coarse.surface".into(),
                    }],
                    vec![],
                )
            },
        )
        .unwrap();
        let SolidBody::PlanarBoolean(body, _) = &mut source.body else {
            unreachable!()
        };
        label_ambiguous(Rc::get_mut(body).unwrap(), &fallback).unwrap();
        let scale = EntityId::new();
        let apply = |factor: f64| {
            source
                .affine_transformed_named(
                    crate::AffineTransform {
                        matrix: [[factor, 0., 0.], [0., factor, 0.], [0., 0., factor]],
                        translation_mm: [0.; 3],
                    },
                    scale,
                    policy,
                )
                .unwrap()
        };
        let first = apply(2.);
        let before = first.topology_snapshot(feature, 1).unwrap();
        assert!(
            before
                .elements
                .iter()
                .flat_map(|e| &e.origins)
                .any(|o| o.role.starts_with("derived-plane.v1."))
        );
        let second = apply(3.);
        let after = second.topology_snapshot(feature, 2).unwrap();
        for element in &before.elements {
            let intent = GeometryIntent::capture(&before, element.key).unwrap();
            let Resolution::Resolved { key, .. } = resolve(&intent, &after, 2) else {
                panic!("lost opaque one-to-one identity")
            };
            assert_eq!(key, element.key);
            assert_eq!(
                after
                    .elements
                    .iter()
                    .find(|e| e.key == key)
                    .unwrap()
                    .origins,
                element.origins
            );
        }
        assert!((second.volume_mm3().unwrap() - 216.).abs() < 1e-8);
    }
    #[test]
    fn selector_uses_subset_ancestry_and_exact_boundary_with_shared_budget_and_recovery() {
        let owner = EntityId::new();
        let origin = |role: &str| Origin {
            source: owner,
            role: role.into(),
        };
        let element = |ordinal, origins: &[&str], boundary: &[&str]| TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Edge,
                ordinal,
            },
            origins: origins.iter().map(|s| origin(s)).collect(),
            boundary: boundary.iter().map(|s| origin(s)).collect(),
            frames: vec![],
            geometry: None,
        };
        let mut snapshot = TopologySnapshot {
            owner,
            revision: 0,
            elements: vec![
                element(0, &["a"], &["x"]),
                element(1, &["a", "b"], &["x"]),
                element(2, &["a"], &["y"]),
            ],
        };
        assert_eq!(ambiguous(&snapshot, &mut { MAX_WORK }).unwrap(), vec![0]);
        assert_eq!(
            ambiguous(&snapshot, &mut 0).unwrap_err().kind,
            GeometryErrorKind::Limit
        );
        snapshot.elements[0].origins.clear();
        assert!(ambiguous(&snapshot, &mut { MAX_WORK }).is_err());
        snapshot.elements[0].origins = (0..=MAX_GEOMETRY_ORIGINS)
            .map(|i| origin(&format!("too-many.{i}")))
            .collect();
        assert_eq!(
            ambiguous(&snapshot, &mut { MAX_WORK }).unwrap_err().kind,
            GeometryErrorKind::Limit
        );
        snapshot.elements.clear();
        assert!(ambiguous(&snapshot, &mut { MAX_WORK }).unwrap().is_empty());
        // Independent small groups do not require scans across unrelated
        // geometry, and still share one cumulative comparison/index budget.
        snapshot.elements = (0..256)
            .map(|i| element(i, &[&format!("group.{}", i / 2)], &["same"]))
            .collect();
        assert_eq!(ambiguous(&snapshot, &mut { MAX_WORK }).unwrap().len(), 256);
        assert_eq!(
            ambiguous(&snapshot, &mut 2_000).unwrap_err().kind,
            GeometryErrorKind::Limit
        );
    }
    #[test]
    fn opaque_segment_and_point_stamps_are_exact_unordered_and_signed_zero_canonical() {
        let owner = EntityId::new();
        let line =
            |a_mm, b_mm| finite_origin(owner, Some(AnalyticGeometry::Line { a_mm, b_mm })).unwrap();
        assert_eq!(line([0.; 3], [1.; 3]), line([1.; 3], [-0.; 3]));
        assert_ne!(line([0.; 3], [1.; 3]), line([0.; 3], [1. + 1e-12, 1., 1.]));
        let point =
            |p| finite_origin(owner, Some(AnalyticGeometry::Point { position_mm: p })).unwrap();
        assert_eq!(point([0.; 3]), point([-0.; 3]));
        assert_ne!(point([0.; 3]), point([1e-12, 0., 0.]));
        assert_ne!(point([0.; 3]).role, line([0.; 3], [1.; 3]).role);
        assert!(finite_origin(owner, None).is_err());
    }
}
