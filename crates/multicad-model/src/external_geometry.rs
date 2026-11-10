//! Detached exact external-reference catalogues. Canonical source-local native
//! geometry is placed exactly once into the receiving sketch context, then
//! resolved again by ancestry; cached topology ordinals never cross that seam.
use crate::{ModelError, error};
use multicad_core::{
    constraint_sketch::SplineBasis,
    document::Document,
    model::RigidPose,
    project_file::ContentHash,
    references::{self, Origin, Resolution, TopologyKind, TopologySnapshot},
    sketch::Frame,
    sketch_external::{
        ExternalBranch, ExternalCatalog, ExternalChoice, ExternalGeometry, ExternalQuery,
        QueryMethod, ResultSelector,
    },
};
use multicad_kernel::{
    BRepSelection, CurvedEdge2d, GeometryPolicy, PlaneFrame, RigidTransform,
    SketchReferenceGeometry, SketchReferenceMethod, SketchReferenceResult, Solid,
};
use std::collections::BTreeSet;
#[derive(Clone, Copy)]
pub(crate) struct Inputs<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub query: &'a ExternalQuery,
    pub source: &'a Solid,
    pub topology: &'a TopologySnapshot,
    pub source_to_context: RigidPose,
    pub frame: Frame,
    pub policy: GeometryPolicy,
}
fn local_geometry(native: SketchReferenceGeometry) -> ExternalGeometry {
    match native {
        SketchReferenceGeometry::Point(position_mm) => ExternalGeometry::Point { position_mm },
        SketchReferenceGeometry::Curve(curve) => match curve {
            CurvedEdge2d::Line { a, b } => ExternalGeometry::Line { a_mm: a, b_mm: b },
            CurvedEdge2d::Circle { center, radius_mm } => ExternalGeometry::Circle {
                center_mm: center,
                radius_mm,
            },
            CurvedEdge2d::Arc {
                center,
                a,
                b,
                clockwise,
            } => ExternalGeometry::Arc {
                center_mm: center,
                a_mm: a,
                b_mm: b,
                clockwise,
            },
            CurvedEdge2d::BSpline(s) => ExternalGeometry::BSpline {
                poles_mm: s.poles,
                basis: SplineBasis {
                    degree: s.degree,
                    knots: s.knots,
                    multiplicities: s.multiplicities,
                    weights: s.weights,
                },
            },
        },
    }
}
impl Inputs<'_> {
    fn fail(&self, message: impl std::fmt::Display) -> ModelError {
        error(Some(self.query.reference), message)
    }
    fn named_support(
        &self,
        native: &SketchReferenceResult,
        topology: &TopologySnapshot,
    ) -> Result<Vec<Origin>, ModelError> {
        let mut names = BTreeSet::new();
        for key in &native.support {
            let element=topology.elements.iter().find(|e|e.key==*key).ok_or_else(||self.fail("Native reference support is absent from the current transformed source topology"))?;
            names.extend(element.origins.iter().cloned());
        }
        if names.is_empty() {
            return Err(self.fail("Native reference result has no stable source ancestry; select named geometry or repair the source"));
        }
        Ok(names.into_iter().collect())
    }
}
pub(crate) fn geometry(input: Inputs<'_>) -> Result<ResultSet, ModelError> {
    input.query.validate().map_err(|e| input.fail(e))?;
    input
        .source_to_context
        .validate()
        .map_err(|e| input.fail(e))?;
    input.topology.validate().map_err(|e| input.fail(e))?;
    if input.topology.owner != input.query.source.source
        || input.topology.revision != input.revision
    {
        return Err(
            input.fail("External source topology belongs to another source or stale revision")
        );
    }
    let current = input
        .source
        .topology_snapshot(input.query.source.source, input.revision)
        .map_err(|e| input.fail(e))?;
    if current != *input.topology {
        return Err(
            input.fail("External source topology does not match its exact current native solid")
        );
    }
    let native_frame = PlaneFrame {
        origin: input.frame.origin,
        u: input.frame.u,
        v: input.frame.v,
    };
    let normal = native_frame.normal();
    if normal
        .iter()
        .zip(input.frame.n)
        .any(|(a, b)| !b.is_finite() || (a - b).abs() > 1e-9)
    {
        return Err(input.fail("External consumer plane has an inconsistent right-handed normal"));
    }
    let transformed = input
        .source
        .transformed(RigidTransform {
            rotation: input.source_to_context.rotation,
            translation_mm: input.source_to_context.translation_mm,
        })
        .map_err(|e| input.fail(e))?;
    let topology = transformed
        .topology_snapshot(input.query.source.source, input.revision)
        .map_err(|e| input.fail(e))?;
    let selected = match references::resolve(&input.query.intent, &topology, input.revision) {
        Resolution::Resolved { key, .. } => key,
        diagnostic => return Err(input.fail(format!(
            "External source needs explicit repair after its occurrence transform: {diagnostic:?}"
        ))),
    };
    let selection = match selected.kind {
        TopologyKind::Face => BRepSelection::Face(selected.ordinal),
        TopologyKind::Edge => BRepSelection::Edge(selected.ordinal),
        TopologyKind::Vertex => BRepSelection::Vertex(selected.ordinal),
    };
    let method = match input.query.method {
        QueryMethod::Coplanar => SketchReferenceMethod::Coplanar,
        QueryMethod::Project => SketchReferenceMethod::Project,
        QueryMethod::Intersect => SketchReferenceMethod::Intersect,
    };
    let native = transformed
        .sketch_reference_geometry(selection, native_frame, method, input.policy)
        .map_err(|e| input.fail(e))?;
    let count = native.results.len();
    let mut choices = Vec::with_capacity(count);
    for result in &native.results {
        let geometry = local_geometry(result.geometry.clone());
        geometry.validate().map_err(|e| input.fail(e))?;
        choices.push(ExternalChoice {
            selector: ResultSelector {
                support: input.named_support(result, &topology)?,
                family: geometry.family(),
                branch: None,
                expected_count: count,
            },
            geometry,
            source_parameter: result.source_parameter,
            source_interval: result.source_interval,
            tangent: result.tangent,
        });
    }
    // Exact source parameters, not output ordering or a closest witness, divide
    // multiple contacts of ONE finite edge into non-overlapping native bands.
    for i in 0..choices.len() {
        let Some(parameter) = choices[i].source_parameter else {
            continue;
        };
        let domain = choices[i].source_interval.ok_or_else(|| {
            input.fail("Native point contact is missing its exact finite input domain")
        })?;
        let mut peers=choices.iter().enumerate().filter(|(_,c)|c.selector.support==choices[i].selector.support&&c.selector.family==choices[i].selector.family).map(|(j,c)|{
            if c.source_interval!=Some(domain)||native.results[j].support!=native.results[i].support {return Err(input.fail("Coincident semantic source branches do not share one provable finite input edge; explicitly repair"));}
            c.source_parameter.map(|p|(j,p)).ok_or_else(||input.fail("Multiple source branches have no exact native parameter identity"))
        }).collect::<Result<Vec<_>,_>>()?;
        peers.sort_by(|a, b| a.1.total_cmp(&b.1));
        if peers
            .windows(2)
            .any(|p| (p[1].1 - p[0].1).abs() <= 1e-12 * (domain[1] - domain[0]).max(1.))
        {
            return Err(input.fail("Native source contacts have indistinguishable parameter branches; explicitly repair"));
        }
        let position = peers
            .iter()
            .position(|(j, _)| *j == i)
            .ok_or_else(|| input.fail("Native parameter branch was lost"))?;
        let low = if position == 0 {
            domain[0]
        } else {
            peers[position - 1].1 + (parameter - peers[position - 1].1) * 0.5
        };
        let high = if position + 1 == peers.len() {
            domain[1]
        } else {
            parameter + (peers[position + 1].1 - parameter) * 0.5
        };
        if parameter < low || parameter > high || low >= high {
            return Err(
                input.fail("Native reference parameter lies outside its finite branch band")
            );
        }
        choices[i].selector.branch = Some(ExternalBranch {
            source_interval: [low, high],
            crossing: native.results[i].crossing,
        });
    }
    for (i, c) in choices.iter().enumerate() {
        if choices[..i].iter().any(|old| old.selector == c.selector) {
            return Err(input.fail("Native section has multiple indistinguishable semantic results; select a more specific named face/edge or explicitly repair. Result ordinals are not persistent identities"));
        }
    }
    let mut diagnostics = Vec::new();
    if native.coincident {
        diagnostics.push("The finite source is coincident with the sketch plane; choose Coplanar to reference its explicit geometry".into());
    } else if choices.is_empty() {
        diagnostics.push("The finite source has no intersection with this sketch plane".into());
    }
    if choices.iter().any(|c| c.tangent) {
        diagnostics.push("Tangent contacts are shown explicitly and cannot be captured as stable transverse references".into());
    }
    Ok(ResultSet {
        results: choices,
        coincident: native.coincident,
        diagnostics,
    })
}
pub(crate) struct ResultSet {
    pub results: Vec<ExternalChoice>,
    pub coincident: bool,
    pub diagnostics: Vec<String>,
}
/// Public-query wrapper only: callers resolve the real absolute consumer pose.
/// Internal feature evaluation uses `geometry` and has no world-pose dependency.
pub(crate) fn query(
    input: Inputs<'_>,
    sketch_world_pose: RigidPose,
) -> Result<ExternalCatalog, ModelError> {
    sketch_world_pose.validate().map_err(|e| input.fail(e))?;
    let result = geometry(input)?;
    let catalog = ExternalCatalog {
        document: input.document.id(),
        revision: input.revision,
        source_hash: ContentHash::document(input.document).map_err(|e| input.fail(e))?,
        query_hash: input.query.hash().map_err(|e| input.fail(e))?,
        query: input.query.clone(),
        sketch_world_pose,
        results: result.results,
        coincident: result.coincident,
        diagnostics: result.diagnostics,
    };
    catalog.validate().map_err(|e| input.fail(e))?;
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        document::EntityId,
        model::OccurrencePath,
        references::{AnalyticGeometry, GeometryIntent},
        sketch_external::{OccurrenceSource, select_external_choice},
    };
    fn p() -> GeometryPolicy {
        GeometryPolicy::default()
    }
    #[cfg(feature = "occt")]
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 2e-7, "{a} != {b}");
    }
    fn request(
        source: EntityId,
        topology: &TopologySnapshot,
        predicate: impl Fn(&AnalyticGeometry) -> bool,
        method: QueryMethod,
    ) -> ExternalQuery {
        let element = topology
            .elements
            .iter()
            .find(|e| e.geometry.as_ref().is_some_and(&predicate))
            .unwrap();
        ExternalQuery {
            reference: EntityId::new(),
            sketch: EntityId::new(),
            source: OccurrenceSource {
                component: EntityId::new(),
                source,
                path: OccurrencePath {
                    root: EntityId::new(),
                    members: vec![],
                },
            },
            sketch_context: None,
            intent: GeometryIntent::capture(topology, element.key).unwrap(),
            method,
        }
    }
    #[cfg(feature = "occt")]
    #[test]
    fn exact_source_is_transformed_once_and_catalog_echoes_real_consumer_pose() {
        let doc = Document::new("query").unwrap();
        let source = EntityId::new();
        let solid = Solid::box_named([10., 20., 5.], source, p()).unwrap();
        let topology = solid.topology_snapshot(source, 4).unwrap();
        let request = request(
            source,
            &topology,
            |g| matches!(g,AnalyticGeometry::Plane{frame}if(frame.origin_mm[2]-5.).abs()<1e-7),
            QueryMethod::Coplanar,
        );
        let relative = RigidPose {
            rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
            translation_mm: [100., 200., 30.],
        };
        let frame = Frame {
            origin: [0., 0., 35.],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            n: [0., 0., 1.],
        };
        let input = Inputs {
            document: &doc,
            revision: 4,
            query: &request,
            source: &solid,
            topology: &topology,
            source_to_context: relative,
            frame,
            policy: p(),
        };
        let display = RigidPose {
            translation_mm: [-500., 42., 7.],
            ..Default::default()
        };
        let catalogue = query(input, display).unwrap();
        assert_eq!(catalogue.sketch_world_pose, display);
        assert_eq!(catalogue.query, request);
        assert_eq!(catalogue.source_hash, ContentHash::document(&doc).unwrap());
        assert_eq!(catalogue.results.len(), 4);
        for r in &catalogue.results {
            let ExternalGeometry::Line { a_mm, b_mm } = r.geometry else {
                panic!()
            };
            for at in [a_mm, b_mm] {
                assert!((80. - 1e-7..=100. + 1e-7).contains(&at[0]));
                assert!((200. - 1e-7..=210. + 1e-7).contains(&at[1]));
            }
            assert!(!r.selector.support.is_empty());
        }
        near(solid.volume_mm3().unwrap(), 1000.);
        assert_eq!(solid.topology_snapshot(source, 4).unwrap(), topology);
    }
    #[cfg(feature = "occt")]
    fn circle(source: EntityId) -> Solid {
        Solid::extrude_curved_profile_named(
            &multicad_kernel::CurvedProfile {
                frame: Default::default(),
                outer: vec![CurvedEdge2d::Circle {
                    center: [0., 0.],
                    radius_mm: 5.,
                }],
                holes: vec![],
            },
            3.,
            source,
            &[Origin {
                source,
                role: "circle".into(),
            }],
            p(),
        )
        .unwrap()
    }
    fn rational(value: i64) -> multicad_kernel::rust_kernel::Rational {
        multicad_kernel::rust_kernel::Rational::new(value, 1).unwrap()
    }

    #[test]
    fn finite_parameter_bands_preserve_two_explicit_branches_without_ordinals() {
        use multicad_kernel::rust_kernel::{Circle, cylinder::AxialCylinder};

        let source = EntityId::new();
        let cylinder = AxialCylinder::new(
            source.uuid_bits(),
            Circle::new([rational(0); 2], rational(5)).unwrap(),
            rational(0),
            rational(3),
        )
        .unwrap();
        let rust = Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), p()).unwrap();
        assert_finite_branches(source, &rust);
        #[cfg(feature = "occt")]
        assert_finite_branches(source, &circle(source));
    }

    fn assert_finite_branches(source: EntityId, solid: &Solid) {
        let doc = Document::new("circle query").unwrap();
        let topology = solid.topology_snapshot(source, 9).unwrap();
        let request = request(
            source,
            &topology,
            |g| matches!(g,AnalyticGeometry::CircularArc{frame,..}if frame.origin_mm[2].abs()<1e-7),
            QueryMethod::Intersect,
        );
        let frame = Frame {
            origin: [0., 0., 0.],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
            n: [1., 0., 0.],
        };
        let input = Inputs {
            document: &doc,
            revision: 9,
            query: &request,
            source: solid,
            topology: &topology,
            source_to_context: Default::default(),
            frame,
            policy: p(),
        };
        let catalogue = query(input, Default::default()).unwrap();
        assert_eq!(catalogue.results.len(), 2);
        let first = &catalogue.results[0];
        let second = &catalogue.results[1];
        assert_ne!(first.selector, second.selector);
        assert_eq!(first.selector.support, second.selector.support);
        assert_eq!(first.selector.family, second.selector.family);
        for c in &catalogue.results {
            let b = c.selector.branch.as_ref().unwrap();
            assert!(b.crossing.is_some());
            assert!(
                c.source_parameter.unwrap() >= b.source_interval[0]
                    && c.source_parameter.unwrap() <= b.source_interval[1]
            );
            assert_eq!(c.selector.expected_count, 2);
        }
        let moved = Inputs {
            frame: Frame {
                origin: [1., 0., 0.],
                ..frame
            },
            ..input
        };
        let moved = query(moved, Default::default()).unwrap();
        for chosen in &catalogue.results {
            let resolved = select_external_choice(&moved, &chosen.selector).unwrap();
            assert_eq!(
                resolved.selector.branch.as_ref().unwrap().crossing,
                chosen.selector.branch.as_ref().unwrap().crossing
            );
        }
        let empty = query(
            Inputs {
                frame: Frame {
                    origin: [6., 0., 0.],
                    ..frame
                },
                ..input
            },
            Default::default(),
        )
        .unwrap();
        assert!(empty.results.is_empty());
        assert!(!empty.diagnostics.is_empty());
        assert!(select_external_choice(&empty, &first.selector).is_err());
    }
    #[test]
    fn stale_mismatched_geometry_and_invalid_frames_fail_before_publication() {
        let doc = Document::new("guard").unwrap();
        let source = EntityId::new();
        let box_at = |width| {
            Solid::rust_box(
                source.uuid_bits(),
                [width, 20, 5].map(rational),
                PlaneFrame::default(),
                p(),
            )
            .unwrap()
        };
        let solid = box_at(10);
        let topology = solid.topology_snapshot(source, 1).unwrap();
        let request = request(
            source,
            &topology,
            |g| matches!(g, AnalyticGeometry::Line { a_mm, b_mm } if a_mm[2] == 0. && b_mm[2] == 0.),
            QueryMethod::Coplanar,
        );
        let input = Inputs {
            document: &doc,
            revision: 1,
            query: &request,
            source: &solid,
            topology: &topology,
            source_to_context: Default::default(),
            frame: multicad_core::sketch::Datum::Xy.frame(),
            policy: p(),
        };
        assert_eq!(query(input, Default::default()).unwrap().results.len(), 1);
        assert!(
            geometry(Inputs {
                revision: 2,
                ..input
            })
            .is_err()
        );
        let other = box_at(11);
        assert!(
            geometry(Inputs {
                source: &other,
                ..input
            })
            .is_err()
        );
        assert!(
            geometry(Inputs {
                frame: Frame {
                    n: [0., 0., -1.],
                    ..input.frame
                },
                ..input
            })
            .is_err()
        );
        assert!(
            query(
                input,
                RigidPose {
                    translation_mm: [f64::NAN, 0., 0.],
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(solid.topology_snapshot(source, 1).unwrap(), topology);
    }
}
