//! Exact component extraction from the retained occupied-cell graph.
use super::*;

impl Solid {
    pub(crate) fn rust_orthogonal_components(
        &self,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Option<Result<Vec<Self>, GeometryError>> {
        let SolidBody::Orthogonal(body, frame) = &self.body else {
            return None;
        };
        let frame = *frame;
        Some((|| {
            if body.has_curved_cuts() || !body.through_bores.is_empty() {
                return Err(unsupported(
                    "Component extraction has not certified curved orthogonal machining",
                ));
            }
            if !self.provenance.is_empty()
                && (!self.has_authored_metadata_override()
                    || self
                        .provenance
                        .iter()
                        .any(|element| !element.frames.is_empty()))
            {
                return Err(unsupported(
                    "Component extraction requires direct retained orthogonal source metadata",
                ));
            }
            let checkpoint = || {
                if interrupt.is_some_and(|f| f()) {
                    Err(error(
                        "solid components",
                        GeometryErrorKind::Cancelled,
                        "Component extraction superseded",
                    ))
                } else {
                    Ok(())
                }
            };
            checkpoint()?;
            let published = self.topology_snapshot(body.feature, 0)?;
            let visible: BTreeSet<_> = published
                .elements
                .iter()
                .flat_map(|element| element.origins.iter().cloned())
                .collect();
            let anchors: BTreeSet<_> = published
                .elements
                .iter()
                .flat_map(|element| element.frames.iter().map(|anchor| anchor.origin.clone()))
                .collect();
            let source = Self {
                body: SolidBody::Orthogonal(Rc::clone(body), PlaneFrame::default()),
                policy: self.policy,
                provenance: self.provenance.clone(),
                rust_origins: self.rust_origins.clone(),
                retained_placement: None,
            };
            let mut sources = Sources::default();
            source
                .orthogonal_operand(false, &mut sources)?
                .ok_or_else(|| {
                    invalid("Retained orthogonal source cannot provide its exact components")
                })?;
            let mut output = Vec::with_capacity(body.geometry.solid_count() as usize);
            let mut ancestry_budget = 2_000_000;
            for geometry in body.geometry.components() {
                checkpoint()?;
                let component = OrthogonalBody::build_with_budget(
                    body.feature,
                    geometry,
                    Some(&sources),
                    &mut ancestry_budget,
                )?;
                let mut component = Self {
                    body: SolidBody::Orthogonal(Rc::new(component), frame),
                    policy: self.policy,
                    provenance: vec![],
                    rust_origins: vec![],
                    retained_placement: None,
                };
                // Extraction copies existing boundary; it does not author
                // new feature identities. Do not publish builder fallback
                // names that were private or absent on the source.
                component.provenance = component
                    .topology_snapshot(body.feature, 0)?
                    .elements
                    .into_iter()
                    .map(|mut element| {
                        element.origins.retain(|origin| visible.contains(origin));
                        element.boundary.retain(|origin| visible.contains(origin));
                        element
                            .frames
                            .retain(|anchor| anchors.contains(&anchor.origin));
                        for origin in &self.rust_origins {
                            element.origins.insert(origin.clone());
                            if !element.boundary.is_empty() {
                                element.boundary.insert(origin.clone());
                            }
                            if !element.frames.iter().any(|anchor| anchor.origin == *origin)
                                && let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                            {
                                element.frames.push(AnchoredFrame {
                                    origin: origin.clone(),
                                    frame,
                                });
                            }
                        }
                        element.geometry = None;
                        element
                    })
                    .collect();
                output.push(component);
            }
            checkpoint()?;
            Ok(output)
        })())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copied_children_share_one_ancestry_allowance_and_leave_source_untouched() {
        let policy = GeometryPolicy::default();
        let stock = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [10; 3].map(|v| Rational::new(v, 1).unwrap()),
            PlaneFrame::default(),
            policy,
        )
        .unwrap();
        let tool = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [2, 12, 12].map(|v| Rational::new(v, 1).unwrap()),
            PlaneFrame {
                origin: [4., -1., -1.],
                ..Default::default()
            },
            policy,
        )
        .unwrap();
        let split = stock
            .rust_subtract_orthogonal_tools(&[tool], policy)
            .unwrap()
            .unwrap();
        let before = split.topology_snapshot(EntityId::new(), 0).unwrap();
        let (body, _) = split.orthogonal_body().unwrap();
        let children = body.geometry.components();
        let mut sources = Sources::default();
        split
            .orthogonal_operand(false, &mut sources)
            .unwrap()
            .unwrap();
        let mut allowance = 2_000_000;
        OrthogonalBody::build_with_budget(
            body.feature,
            children[0].clone(),
            Some(&sources),
            &mut allowance,
        )
        .unwrap();
        let mut exact_first_cost = 2_000_000 - allowance;
        assert!(exact_first_cost > 0);
        OrthogonalBody::build_with_budget(
            body.feature,
            children[0].clone(),
            Some(&sources),
            &mut exact_first_cost,
        )
        .unwrap();
        assert_eq!(exact_first_cost, 0);
        assert_eq!(
            OrthogonalBody::build_with_budget(
                body.feature,
                children[1].clone(),
                Some(&sources),
                &mut exact_first_cost
            )
            .err()
            .unwrap()
            .kind,
            GeometryErrorKind::Limit
        );
        assert_eq!(split.topology_snapshot(before.owner, 0).unwrap(), before);
        assert_eq!(split.solid_components(policy).unwrap().len(), 2);
    }
    #[test]
    fn component_extraction_does_not_restore_private_names_of_anonymous_material() {
        let policy = GeometryPolicy::default();
        let stock = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [10; 3].map(|v| Rational::new(v, 1).unwrap()),
            PlaneFrame::default(),
            policy,
        )
        .unwrap();
        let tool = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [2, 12, 12].map(|v| Rational::new(v, 1).unwrap()),
            PlaneFrame {
                origin: [4., -1., -1.],
                ..Default::default()
            },
            policy,
        )
        .unwrap();
        let split = stock
            .rust_subtract_orthogonal_tools(&[tool], policy)
            .unwrap()
            .unwrap()
            .without_authored_provenance()
            .unwrap();
        let owner = EntityId::new();
        let member = EntityId::new();
        for source in [
            split.share().unwrap(),
            split.with_component_origin(owner, member).unwrap(),
        ] {
            let expected: BTreeSet<_> = source.rust_origins.iter().cloned().collect();
            for component in source.solid_components(policy).unwrap() {
                let snapshot = component.topology_snapshot(owner, 0).unwrap();
                assert!(snapshot.elements.iter().all(|element| {
                    element.origins == expected
                        && element
                            .frames
                            .iter()
                            .all(|anchor| expected.contains(&anchor.origin))
                }));
            }
        }
    }
}
