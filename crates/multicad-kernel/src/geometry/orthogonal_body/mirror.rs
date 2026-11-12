//! Orientation-reversing placement for exact rectilinear material.
use super::*;

struct Reflection {
    axis: usize,
    offset: f64,
}

impl Reflection {
    fn point(&self, mut point: Point) -> Point {
        point[self.axis] = 2. * self.offset - point[self.axis];
        point
    }

    fn vector(&self, mut vector: Point) -> Point {
        vector[self.axis] = -vector[self.axis];
        vector
    }

    fn bounds(&self, bounds: Box3) -> Box3 {
        extent([self.point(bounds.min), self.point(bounds.max)])
    }

    fn frame(&self, frame: ReferenceFrame) -> ReferenceFrame {
        ReferenceFrame {
            origin_mm: self.point(frame.origin_mm),
            u: self.vector(frame.u),
            // A reflection reverses handedness. Reverse the second tangent
            // so the attachment normal still points out of the material.
            v: self.vector(frame.v).map(|value| -value),
        }
    }

    fn topology(&self, mut element: TopologyElement) -> TopologyElement {
        element.geometry = match element.geometry {
            Some(AnalyticGeometry::Plane { frame }) => Some(AnalyticGeometry::Plane {
                frame: self.frame(frame),
            }),
            Some(AnalyticGeometry::Line { a_mm, b_mm }) => Some(AnalyticGeometry::Line {
                a_mm: self.point(a_mm),
                b_mm: self.point(b_mm),
            }),
            Some(AnalyticGeometry::Point { position_mm }) => Some(AnalyticGeometry::Point {
                position_mm: self.point(position_mm),
            }),
            _ => unreachable!("orthogonal ancestry is planar, straight or a point"),
        };
        if let Some(AnalyticGeometry::Plane { frame }) = element.geometry {
            for anchor in &mut element.frames {
                anchor.frame = self.frame(anchor.frame);
                // Some source caps attach in their authored sketch direction.
                // Mirror publishes outward attachment frames like the native provider.
                if dot(anchor.frame.normal(), frame.normal()) < 0. {
                    anchor.frame.v = anchor.frame.v.map(|value| -value);
                }
            }
        }
        element
    }

    fn sources(&self, sources: Sources) -> Sources {
        let mut reflected = Sources::default();
        for support in sources.faces.into_values().flatten() {
            let face = self.topology(support.face);
            let Some(AnalyticGeometry::Plane { frame }) = face.geometry else {
                unreachable!("orthogonal face support");
            };
            let patches: Vec<_> = support
                .patches
                .into_iter()
                .map(|patch| self.bounds(patch))
                .collect();
            reflected
                .faces
                .entry(plane_key(frame.normal(), patches[0].min))
                .or_default()
                .push(Support { patches, face });
        }
        for edge in sources.edges.into_values().flatten() {
            let edge = self.topology(edge);
            let Some(AnalyticGeometry::Line { a_mm, b_mm }) = edge.geometry else {
                unreachable!("orthogonal edge support");
            };
            reflected
                .edges
                .entry(line_key(a_mm, b_mm))
                .or_default()
                .push(edge);
        }
        for vertex in sources.vertices.into_values().flatten() {
            let vertex = self.topology(vertex);
            let Some(AnalyticGeometry::Point { position_mm }) = vertex.geometry else {
                unreachable!("orthogonal vertex support");
            };
            reflected
                .vertices
                .entry(key(position_mm))
                .or_default()
                .push(vertex);
        }
        reflected
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SupportKey {
    Plane(PlaneKey),
    Line([PointKey; 2]),
    Point(PointKey),
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Name<'a> {
    geometry: SupportKey,
    origins: &'a BTreeSet<Origin>,
    boundary: &'a BTreeSet<Origin>,
    anchors: BTreeSet<(&'a Origin, [PointKey; 3])>,
}

fn name(element: &TopologyElement) -> Name<'_> {
    let geometry = match element.geometry {
        Some(AnalyticGeometry::Plane { frame }) => {
            SupportKey::Plane(plane_key(frame.normal(), frame.origin_mm))
        }
        Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
            let mut ends = [key(a_mm), key(b_mm)];
            ends.sort_unstable();
            SupportKey::Line(ends)
        }
        Some(AnalyticGeometry::Point { position_mm }) => SupportKey::Point(key(position_mm)),
        _ => unreachable!("orthogonal boundary geometry"),
    };
    Name {
        geometry,
        origins: &element.origins,
        boundary: &element.boundary,
        anchors: element
            .frames
            .iter()
            .map(|anchor| {
                (
                    &anchor.origin,
                    [anchor.frame.origin_mm, anchor.frame.u, anchor.frame.v].map(key),
                )
            })
            .collect(),
    }
}

impl Solid {
    pub(crate) fn mirror_cardinal(
        &self,
        plane: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if self.policy != policy {
            return Err(mirror_error(
                GeometryErrorKind::Unsupported,
                "Rust Mirror requires the source geometry policy",
            ));
        }
        let normal = plane.normal();
        if normal.iter().filter(|&&n| n != 0.).count() != 1 {
            return self.mirror_placed(plane, feature, policy);
        }
        let axis = axis(normal);
        let reflection = Reflection {
            axis,
            offset: plane.origin[axis],
        };
        let mut sources = Sources::default();
        let Some((_, cells)) = self.orthogonal_operand(false, &mut sources)? else {
            return self.mirror_placed(plane, feature, policy);
        };
        let cells: Vec<_> = cells
            .into_iter()
            .map(|cell| reflection.bounds(cell))
            .collect();
        let bounds = extent(cells.iter().flat_map(|cell| [cell.min, cell.max]));
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        let geometry =
            orthogonal::boolean(&cells, &[], Operation::Union, policy.linear_tolerance_mm)
                .map_err(operation_error)?
                .ok_or_else(|| {
                    mirror_error(
                        GeometryErrorKind::InvalidResult,
                        "Rust Mirror lost its stock",
                    )
                })?;
        let sources = reflection.sources(sources);
        let body = OrthogonalBody::build(feature, geometry, Some(&sources))?;
        let mut expected: Vec<_> = sources
            .faces
            .values()
            .flatten()
            .map(|support| &support.face)
            .chain(sources.edges.values().flatten())
            .chain(sources.vertices.values().flatten())
            .map(name)
            .collect();
        let mut actual: Vec<_> = body.topology.elements.iter().map(name).collect();
        expected.sort_unstable();
        actual.sort_unstable();
        // Reflecting whole stock preserves every analytic support and its
        // incidence. Refuse a result if normalization loses or splits names.
        if actual != expected {
            return Err(mirror_error(
                GeometryErrorKind::Unsupported,
                "Rust Mirror could not preserve every source face, edge and vertex",
            ));
        }
        Ok(Self {
            body: SolidBody::Orthogonal(Rc::new(body), PlaneFrame::default()),
            policy,
            provenance: vec![],
            rust_origins: vec![],
            retained_placement: None,
        })
    }

    fn mirror_placed(
        &self,
        plane: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        // Keep the exact cardinal reflection in the source chart. The two
        // reflections M and J make M*R*J a proper rigid placement, retaining
        // the existing boundary identities and outward attachment frames.
        let mut local = self.share()?;
        let placement = match &mut local.body {
            SolidBody::Prism(_, frame)
            | SolidBody::Linear(_, frame)
            | SolidBody::Orthogonal(_, frame) => std::mem::take(frame),
            SolidBody::PolygonPrism(body) => std::mem::take(&mut Rc::make_mut(body).frame),
            _ => {
                return Err(mirror_error(
                    GeometryErrorKind::Unsupported,
                    "Rust Mirror requires world-aligned rectilinear stock in its source chart without curved cuts",
                ));
            }
        };
        // Validate admission before recursing, including curved machining and
        // nonrectilinear polygon profiles, which cannot use this exact chart.
        if local.orthogonal_cells()?.is_none() {
            return Err(mirror_error(
                GeometryErrorKind::Unsupported,
                "Rust Mirror requires world-aligned rectilinear stock in its source chart without curved cuts",
            ));
        }
        let reflected = local.mirror_cardinal(
            PlaneFrame {
                u: [0., 1., 0.],
                v: [0., 0., 1.],
                ..Default::default()
            },
            feature,
            policy,
        )?;
        let normal = plane.normal();
        // Plane validation admits axes within an orthonormality tolerance.
        // Reflection depends only on the direction, not that residual length.
        let normal_squared = dot(normal, normal);
        let reflect_vector = |vector: Point| -> Point {
            let projection = 2. * dot(normal, vector) / normal_squared;
            std::array::from_fn(|i| vector[i] - projection * normal[i])
        };
        let columns = [
            reflect_vector(placement.u).map(|v| -v),
            reflect_vector(placement.v),
            reflect_vector(placement.normal()),
        ];
        let origin = reflect_vector(placement.origin);
        let offset = 2. * dot(normal, plane.origin) / normal_squared;
        reflected.transformed(RigidTransform {
            rotation: std::array::from_fn(|i| std::array::from_fn(|j| columns[j][i])),
            translation_mm: std::array::from_fn(|i| origin[i] + offset * normal[i]),
        })
    }
}

fn mirror_error(kind: GeometryErrorKind, message: &str) -> GeometryError {
    error("mirror", kind, message)
}
