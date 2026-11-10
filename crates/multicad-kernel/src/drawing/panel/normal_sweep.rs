//! Exact end-on drawing of line-and-arc stock with transverse piercings.
use super::*;
use multicad_core::references::ReferenceFrame;
use multicad_rust_kernel::normal_sweep::Edge as ProfileEdge;

mod transverse;
pub(in crate::drawing) use transverse::transverse_rails;

type Point3 = [f64; 3];

pub(in crate::drawing) fn project_normal_sweep(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let (body, frame) = input.solid.normal_sweep_body()?;
    let mut alternate = None;
    if curved::warped::needs_sweep_shell(frame, options) {
        let result = curved::project_scene(std::slice::from_ref(input), options);
        if !matches!(&result, Err(error) if error.kind == GeometryErrorKind::Unsupported) {
            return Some(result);
        }
        alternate = Some(result);
    }
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = frame.normal();
    let axial = dot(axis, u).hypot(dot(axis, v)) <= 32. * f64::EPSILON;
    let transverse = dot(axis, toward_viewer).abs() <= 32. * f64::EPSILON;
    if !axial && !transverse {
        return alternate;
    }
    if options.detail.is_some() {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust normal-sweep projection does not yet support a detail crop",
        )));
    }
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    if transverse {
        return Some(transverse::project(input, options, frame, &screen));
    }
    let tolerance = input.solid.geometry_policy().linear_tolerance_mm;
    Some((|| {
        let mut edges = Vec::new();
        for rim in body.drawing_rims(frame, false) {
            let curve = screen.curve(Some(rim))?;
            edges.push(Edge {
                hidden: false,
                curve,
            });
            if options.hidden_lines {
                // The two intact caps have the same projection. The cap
                // toward the viewer occludes every edge of the rear cap.
                edges.push(Edge {
                    hidden: true,
                    curve,
                });
            }
        }
        if options.hidden_lines {
            for (faces, outline) in body.drawing_piercings(frame) {
                // The certified opening lies strictly between the caps, so
                // all its edges and generators are behind the front cap.
                for [a, b] in piercing_edges(faces, outline, toward_viewer) {
                    let [a, b] = [screen.point(a), screen.point(b)];
                    if (a[0] - b[0]).hypot(a[1] - b[1]) > tolerance {
                        edges.push(Edge {
                            hidden: true,
                            curve: Curve::Line([a, b]),
                        });
                    }
                }
            }
        }
        curved::publish(edges, options, 0)
    })())
}

fn piercing_edges(
    faces: [ReferenceFrame; 2],
    outline: &[ProfileEdge],
    view: Point3,
) -> Vec<[Point3; 2]> {
    let point = |frame: ReferenceFrame, [u, v]: Point| {
        std::array::from_fn(|i| frame.origin_mm[i] + u * frame.u[i] + v * frame.v[i])
    };
    if let [
        ProfileEdge::Circle {
            center, radius_mm, ..
        },
    ] = outline
    {
        // For an edge-on circle the two tangent points are on the exact
        // projected diameter. Their rails span the cylindrical cut wall.
        let normal = faces[0].normal();
        let tangent: Point3 = std::array::from_fn(|i| {
            view[(i + 1) % 3] * normal[(i + 2) % 3] - view[(i + 2) % 3] * normal[(i + 1) % 3]
        });
        let length = dot(tangent, tangent).sqrt();
        let offset = tangent.map(|value| value * radius_mm / length);
        let [first, last] = faces.map(|face| {
            let center = point(face, *center);
            [-1., 1.].map(|sign| std::array::from_fn(|i| center[i] + sign * offset[i]))
        });
        return vec![first, last, [first[0], last[0]], [first[1], last[1]]];
    }

    let mut edges = Vec::with_capacity(3 * outline.len());
    for edge in outline {
        let a = faces.map(|face| point(face, edge.start()));
        let b = faces.map(|face| point(face, edge.end()));
        edges.extend([[a[0], b[0]], [a[1], b[1]], a]);
    }
    edges
}
