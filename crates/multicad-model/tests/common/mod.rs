use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Curve, CurveKind, source::Source},
    document::{Document, EntityId, Mutation},
    model::{ExtrudeOutput, ExtrudeParameters, ExtrudeRequest, prepare_extrude},
    sketch::Datum,
    sketch_regions,
};

pub const WIDTH: f64 = 80.;
pub const HEIGHT: f64 = 60.;
pub const INNER_AREA: f64 = 36. * std::f64::consts::PI;
pub fn actor() -> Actor {
    Actor {
        id: "compiler-integration-test".into(),
        channel: Channel::Mcp,
    }
}
/// Two exact, independently selectable profiles from a single persistent
/// rectangle-and-circle sketch. No authored display meshes participate.
pub struct SharedProfiles {
    pub session: Session,
    pub sketch: EntityId,
    pub request: ExtrudeRequest,
}
impl SharedProfiles {
    pub fn new(parameters: ExtrudeParameters) -> Self {
        let (sketch, edits) = Source::create(Datum::Xy, "Shared rectangle and disk").unwrap();
        let mut document = Document::new("Exact compiler integration")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let mut source = Source::read(&document, sketch).unwrap().0;
        source.add_rectangle([0., 0.], [WIDTH, HEIGHT]).unwrap();
        let center = source.add_point([20., 20.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 6.,
            },
        });
        let regions = sketch_regions::extract(&source);
        assert!(regions.diagnostics.is_empty());
        assert_eq!(regions.regions.len(), 2);
        let request = ExtrudeRequest::new(
            sketch,
            regions.regions.iter().map(|r| r.intent()).collect(),
            parameters,
        );
        document = document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let source_only = prepare_extrude(&session, 0, actor(), &request).unwrap();
        session.commit(source_only.prepared).unwrap();
        Self {
            session,
            sketch,
            request,
        }
    }
    pub fn outer(&self) -> &ExtrudeOutput {
        self.request
            .outputs
            .iter()
            .find(|o| o.region.holes.len() == 1)
            .unwrap()
    }
    pub fn inner(&self) -> &ExtrudeOutput {
        self.request
            .outputs
            .iter()
            .find(|o| o.region.holes.is_empty())
            .unwrap()
    }
    pub fn source(&self) -> Source {
        Source::read_for_evaluation(
            self.session.document(),
            self.sketch,
            self.session.revision(),
            None,
        )
        .unwrap()
        .0
    }
    fn replace_source(&mut self, source: Source) {
        let edits = source
            .replace(self.session.document(), self.sketch)
            .unwrap();
        self.apply(edits);
    }
    pub fn apply(&mut self, edits: Vec<Mutation>) {
        self.session
            .apply(Request {
                expected_revision: self.session.revision(),
                actor: actor(),
                label: "Integration fixture edit".into(),
                edits,
            })
            .unwrap();
    }
    /// Move the two right corners while retaining every point/curve/constraint
    /// ID. Existing RegionIntents must rebuild instead of being recaptured.
    pub fn set_width(&mut self, width: f64) {
        let mut source = self.source();
        let mut moved = 0;
        for point in &mut source.sketch.points {
            if point.position_mm[0] == WIDTH {
                point.position_mm[0] = width;
                moved += 1;
            }
        }
        assert_eq!(moved, 2);
        self.replace_source(source);
    }
    /// The old outer profile intent must become stale; the selected disk stays
    /// independently valid. A compiler must never adopt the new hole silently.
    pub fn add_unexpected_hole(&mut self) {
        let mut source = self.source();
        let center = source.add_point([50., 30.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 3.,
            },
        });
        self.replace_source(source);
    }
}

pub fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-5,
        "{actual} differs from {expected}"
    );
}
pub fn point_near(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        near(actual, expected);
    }
}
