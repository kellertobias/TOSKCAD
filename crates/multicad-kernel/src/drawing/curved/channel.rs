//! Certified blocker volumes and physical edges of a bent normal sweep.
use super::{machined::Cut, *};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame};
use multicad_rust_kernel::{
    normal_sweep::Edge as ProfileEdge,
    orthogonal::Box3,
    planar_projection::{Body, Face, HiddenCandidate, LineCandidate},
};
use std::collections::BTreeMap;

mod profile;

type Point = [f64; 3];
const ANGULAR: f64 = 32. * f64::EPSILON;

pub(super) fn add(
    solid: &Solid,
    source: usize,
    options: DrawingProjection,
    scene: &mut stadium::Shapes,
) -> Result<bool, GeometryError> {
    let Some((body, frame)) = solid.normal_sweep_body() else {
        return Ok(false);
    };
    let (rings, height) = body.drawing_section();
    let profile = profile::Profile::new(rings).ok_or_else(|| {
        unsupported(
            "Rust mixed drawing requires rectangular walls and paired concentric quarter bends",
        )
    })?;
    let view = View::new(options.frame);
    let sweep_axis = unit(frame.normal());
    let axis = view.vector(sweep_axis.map(|value| value * height.signum()));
    let axial = axis[0].hypot(axis[1]) <= ANGULAR;
    if !axial && axis[2].abs() > ANGULAR {
        return Err(unsupported(
            "Rust mixed bent-sweep drawing requires an axial or transverse view",
        ));
    }
    let levels = [height.min(0.), height.max(0.)];
    let point = |p| view.point(crate::rust_prism_topology::position(frame, p));
    for bend in profile.bends {
        let [inner, outer] = bend.radii;
        let spokes = bend
            .spokes
            .map(|[u, v]| view.vector(std::array::from_fn(|i| frame.u[i] * u + frame.v[i] * v)));
        scene.rounds.push(
            RoundOccluder::quarter_annulus(
                point([bend.center[0], bend.center[1], 0.]),
                axis,
                spokes,
                outer,
                inner,
                height.abs(),
            )
            .map_err(|_| {
                unsupported("Rust mixed bent-sweep drawing requires cardinal quarter bend spokes")
            })?,
        );
    }

    let blocker_offset = scene.bodies.len();
    let rectangles: Vec<_> = profile
        .rectangles
        .into_iter()
        .map(|[[x0, y0], [x1, y1]]| Box3 {
            min: [x0, y0, levels[0]],
            max: [x1, y1, levels[1]],
        })
        .collect();
    let mut blockers: Vec<_> = rectangles
        .iter()
        .map(|&region| box_body(region, point))
        .collect();
    let mut cut_candidates = machined::stock_with_cuts((), 0);
    for (faces, outline) in body.drawing_piercings(frame) {
        let (index, cap_faces) = host(&rectangles, frame, faces, outline, solid.geometry_policy().linear_tolerance_mm)
            .ok_or_else(|| unsupported("Rust bent-sweep drawing could not pair a piercing with one complete rectangular wall"))?;
        if let [
            ProfileEdge::Circle {
                center, radius_mm, ..
            },
        ] = outline
        {
            let centers = faces.map(|face| view.point(face_point(face, *center)));
            Cut {
                faces: cap_faces.map(Some),
                centers,
                outward: unit(sub(centers[0], centers[1])),
                radius: *radius_mm,
            }
            .append(&mut cut_candidates, source, blocker_offset + index)?;
        } else {
            let points: [Vec<_>; 2] = faces.map(|face| {
                outline
                    .iter()
                    .map(|edge| view.point(face_point(face, edge.start())))
                    .collect()
            });
            let [Ok(a), Ok(b)] = points.map(<[_; 4]>::try_from) else {
                return Err(unsupported(
                    "Rust mixed bent-sweep drawing requires a rectangular piercing",
                ));
            };
            let through = unit(sub(b[0], a[0]));
            let close = through[2].abs() <= ANGULAR;
            let near = usize::from(through[2] > 0.);
            add_rectangular_cut(&mut blockers[index], [a, b], cap_faces);
            for i in 0..4 {
                let j = (i + 1) % 4;
                for (end, points) in [[a[i], a[j]], [b[i], b[j]]].into_iter().enumerate() {
                    publish_line(scene, source, points, close || end != near);
                }
                publish_line(scene, source, [a[i], b[i]], close);
            }
        }
    }
    scene.bodies.extend(blockers);
    scene.openings.extend(cut_candidates.openings);
    scene.hidden.extend(cut_candidates.intrinsic_hidden);
    scene
        .rims
        .extend(cut_candidates.rims.into_iter().map(|rim| {
            planar_projection::SpatialRimCandidate {
                source: rim.source,
                ellipse: curved_projection::Ellipse {
                    center: rim.center,
                    u: [1., 0.],
                    v: [0., 1.],
                    major: rim.radius,
                    minor: rim.radius,
                    trim: rim.trim,
                },
                depth: rim.depth,
                depth_sine: 0.,
            }
        }));

    let near_end = axis[2] > 0.;
    for end in [false, true] {
        let hidden = axial && end != near_end;
        for rim in body.drawing_rims(frame, end) {
            match rim {
                AnalyticGeometry::Line { a_mm, b_mm } => {
                    publish_line(scene, source, [a_mm, b_mm].map(|p| view.point(p)), hidden);
                }
                AnalyticGeometry::CircularArc {
                    frame,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                } => {
                    for mut rim in stadium::spatial_rims(
                        source,
                        frame,
                        radius_mm,
                        start_rad,
                        sweep_rad,
                        |p| view.point(p),
                        |p| view.vector(p),
                    ) {
                        if rim.ellipse.minor <= rim.ellipse.major * ANGULAR {
                            rim.ellipse.minor = 0.;
                        }
                        if hidden {
                            scene.hidden.push(HiddenCandidate::SpatialRim(rim));
                        } else {
                            scene.rims.push(rim);
                        }
                    }
                }
                _ => {
                    return Err(unsupported(
                        "Rust mixed bent sweep requires straight and circular cap edges",
                    ));
                }
            }
        }
    }
    if !axial {
        for points in panel::transverse_rails(solid, frame, options)? {
            publish_line(scene, source, points.map(|p| view.point(p)), false);
        }
    }
    Ok(true)
}

fn host(
    regions: &[Box3],
    placement: PlaneFrame,
    faces: [ReferenceFrame; 2],
    outline: &[ProfileEdge],
    tolerance: f64,
) -> Option<(usize, [usize; 2])> {
    let axes = planar::view_axes(placement);
    let local = |p| axes.map(|axis| dot(axis, sub(p, placement.origin)));
    let levels = faces.map(|face| local(face.origin_mm));
    let movement = sub(levels[1], levels[0]);
    let axis = (0..2).find(|&i| movement[i].abs() > tolerance)?;
    if (0..3).any(|i| i != axis && movement[i].abs() > tolerance) {
        return None;
    }
    let support = axes.map(|direction| {
        let projection = |p| dot(direction, sub(p, placement.origin));
        if let [
            ProfileEdge::Circle {
                center, radius_mm, ..
            },
        ] = outline
        {
            let origin = projection(face_point(faces[0], *center));
            let r = radius_mm * dot(direction, faces[0].u).hypot(dot(direction, faces[0].v));
            return [origin - r, origin + r];
        }
        outline
            .iter()
            .map(|edge| projection(face_point(faces[0], edge.start())))
            .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], v| {
                [lo.min(v), hi.max(v)]
            })
    });
    let sign = usize::from(movement[axis] < 0.);
    let mut matches = regions.iter().enumerate().filter(|(_, region)| {
        (levels[sign][axis] - region.min[axis]).abs() <= tolerance
            && (levels[1 - sign][axis] - region.max[axis]).abs() <= tolerance
            && (0..3).all(|i| {
                i == axis
                    || (support[i][0] > region.min[i] + tolerance
                        && support[i][1] < region.max[i] - tolerance)
            })
    });
    let (index, _) = matches.next()?;
    matches
        .next()
        .is_none()
        .then_some((index, [2 * axis + sign, 2 * axis + 1 - sign]))
}

fn box_body(bounds: Box3, point: impl Fn(Point) -> Point) -> Body {
    let corners: [Point; 8] = std::array::from_fn(|mask| {
        point(std::array::from_fn(|i| {
            if mask & (1 << i) == 0 {
                bounds.min[i]
            } else {
                bounds.max[i]
            }
        }))
    });
    let mut edges = Vec::new();
    let mut index = BTreeMap::new();
    for (i, &p) in corners.iter().enumerate() {
        for axis in 0..3 {
            let j = i | (1 << axis);
            if i != j {
                index.insert([i, j], edges.len());
                edges.push([p, corners[j]]);
            }
        }
    }
    let faces = (0..3)
        .flat_map(|axis| [0, 1].map(move |high| (axis, high)))
        .map(|(axis, high)| {
            let anchor = high << axis;
            let normal = unit(sub(corners[1 << axis], corners[0]))
                .map(|value| if high == 0 { -value } else { value });
            let face_edges = index
                .iter()
                .filter(|([i, j], _)| ((i >> axis) & 1) == high && ((j >> axis) & 1) == high)
                .map(|(_, &i)| i)
                .collect();
            Face {
                point: corners[anchor],
                normal,
                edges: face_edges,
            }
        })
        .collect();
    Body { edges, faces }
}

fn add_rectangular_cut(body: &mut Body, [a, b]: [[Point; 4]; 2], faces: [usize; 2]) {
    let first = body.edges.len();
    let center: Point = std::array::from_fn(|i| a[0][i].midpoint(a[2][i]));
    for (end, vertices) in [a, b].into_iter().enumerate() {
        for i in 0..4 {
            body.faces[faces[end]].edges.push(body.edges.len());
            body.edges.push([vertices[i], vertices[(i + 1) % 4]]);
        }
    }
    body.edges.extend((0..4).map(|i| [a[i], b[i]]));
    for i in 0..4 {
        let j = (i + 1) % 4;
        let midpoint = std::array::from_fn(|k| a[i][k].midpoint(a[j][k]));
        body.faces.push(Face {
            point: a[i],
            normal: unit(sub(center, midpoint)),
            edges: vec![first + i, first + 4 + i, first + 8 + i, first + 8 + j],
        });
    }
}

fn publish_line(scene: &mut stadium::Shapes, source: usize, points: [Point; 2], hidden: bool) {
    let line = LineCandidate { source, points };
    if hidden {
        scene.hidden.push(HiddenCandidate::Line(line));
    } else {
        scene.lines.push(line);
    }
}

fn face_point(frame: ReferenceFrame, [u, v]: [f64; 2]) -> Point {
    std::array::from_fn(|i| frame.origin_mm[i] + u * frame.u[i] + v * frame.v[i])
}

struct View {
    axes: [Point; 3],
    origin: Point,
}
impl View {
    fn new(frame: PlaneFrame) -> Self {
        Self {
            axes: planar::view_axes(frame),
            origin: frame.origin,
        }
    }
    fn vector(&self, p: Point) -> Point {
        self.axes.map(|axis| dot(axis, p))
    }
    fn point(&self, p: Point) -> Point {
        self.vector(sub(p, self.origin))
    }
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn unsupported(message: &str) -> GeometryError {
    err(GeometryErrorKind::Unsupported, message)
}
