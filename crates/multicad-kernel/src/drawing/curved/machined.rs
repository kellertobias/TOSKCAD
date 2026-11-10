//! Analytic face trims and hidden edges of certified circular cuts.
use super::{pockets::Stock, *};
use multicad_rust_kernel::{
    headed_screw::hex_vertices,
    planar_projection::{
        Body, CircularOpeningEnd, Face, FaceOpening, HiddenCandidate, LineCandidate, RimCandidate,
    },
};
use std::f64::consts::TAU;

const ANGULAR: f64 = 32. * f64::EPSILON;
type Point = [f64; 3];

struct Screen {
    axes: [Point; 3],
    origin: Point,
}

impl Screen {
    fn new(frame: PlaneFrame) -> Self {
        Self {
            axes: planar::view_axes(frame),
            origin: frame.origin,
        }
    }

    fn vector(&self, v: Point) -> Point {
        self.axes.map(|axis| dot(axis, v))
    }

    fn point(&self, p: Point) -> Point {
        self.vector(std::array::from_fn(|i| p[i] - self.origin[i]))
    }
}

pub(super) struct Cut {
    pub faces: [Option<usize>; 2],
    pub centers: [Point; 2],
    pub outward: Point,
    pub radius: f64,
}

impl Cut {
    pub(super) fn append<T>(
        self,
        result: &mut Stock<T>,
        source: usize,
        body: usize,
    ) -> Result<(), GeometryError> {
        let [x, y, z] = self.outward;
        if z.abs() <= ANGULAR {
            // A view ray is parallel to the cut caps. The certified cavity
            // can only be reached after hitting intact enclosing stock.
            let length = x.hypot(y);
            let span = [-y * self.radius / length, x * self.radius / length];
            let [a, b] = self.centers.map(|center| {
                [-1., 1.].map(|sign| {
                    [
                        center[0] + span[0] * sign,
                        center[1] + span[1] * sign,
                        center[2],
                    ]
                })
            });
            for points in [a, b, [a[0], b[0]], [a[1], b[1]]] {
                result
                    .intrinsic_hidden
                    .push(HiddenCandidate::Line(LineCandidate { source, points }));
            }
            return Ok(());
        }
        if x.hypot(y) > ANGULAR {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust hidden circular stock projection requires face-on or edge-on cuts",
            ));
        }

        let near = usize::from(z < 0.);
        let rims = self.centers.map(|[u, v, depth]| RimCandidate {
            source,
            center: [u, v],
            depth,
            radius: self.radius,
            trim: [0., TAU],
        });
        if let Some(face) = self.faces[near] {
            let rim = rims[near];
            result.openings.push(FaceOpening {
                body,
                face,
                center: rim.center,
                radius: self.radius,
                end: self.faces[1 - near].map_or(
                    CircularOpeningEnd::Floor(rims[1 - near].depth),
                    CircularOpeningEnd::OppositeFace,
                ),
            });
            result.rims.push(rim);
        } else {
            result
                .intrinsic_hidden
                .push(HiddenCandidate::Rim(rims[near]));
        }
        // A rear mouth or blind floor projects onto the nearer rim.
        result
            .intrinsic_hidden
            .push(HiddenCandidate::Rim(rims[1 - near]));
        Ok(())
    }
}

pub(super) fn stock_with_cuts<T>(solid: T, count: usize) -> Stock<T> {
    Stock {
        solid,
        rims: Vec::with_capacity(count),
        spatial_rims: Vec::new(),
        openings: Vec::with_capacity(count),
        intrinsic_hidden: Vec::new(),
    }
}

pub(super) fn stock(
    solid: &Solid,
    source: usize,
    body: usize,
    options: DrawingProjection,
) -> Result<Option<Stock>, GeometryError> {
    let Some((solid, cuts)) = solid.drawing_circular_cuts() else {
        return Ok(None);
    };
    let screen = Screen::new(options.frame);
    let mut result = stock_with_cuts(solid, cuts.len());
    for cut in cuts {
        Cut {
            faces: cut.faces,
            centers: cut.centers.map(|p| screen.point(p)),
            outward: screen.vector(cut.outward),
            radius: cut.radius,
        }
        .append(&mut result, source, body)?;
    }
    Ok(Some(result))
}

pub(super) fn hex_stock(
    solid: &Solid,
    source: usize,
    body: usize,
    options: DrawingProjection,
) -> Result<Option<Stock<Body>>, GeometryError> {
    let Some((fastener, frame)) = solid.cosmetic_fastener_body() else {
        return Ok(None);
    };
    let Some([width, radius, low, high]) = fastener.straight_hex_bore() else {
        return Ok(None);
    };
    let screen = Screen::new(options.frame);
    let point = |p| screen.point(crate::rust_prism_topology::position(frame, p));
    let axis = screen.vector(unit(frame.normal()));
    let [bottom, top] = [low, high].map(|z| hex_vertices(width).map(|[x, y]| point([x, y, z])));
    let mut result = stock_with_cuts(hex_prism(bottom, top, axis), 1);
    Cut {
        faces: [Some(0), Some(1)],
        centers: [low, high].map(|z| point([0., 0., z])),
        outward: axis.map(|v| -v),
        radius,
    }
    .append(&mut result, source, body)?;
    Ok(Some(result))
}

fn hex_prism(bottom: [Point; 6], top: [Point; 6], axis: Point) -> Body {
    let edges = [bottom, top]
        .into_iter()
        .flat_map(|cap| (0..6).map(move |i| [cap[i], cap[(i + 1) % 6]]))
        .chain((0..6).map(|i| [bottom[i], top[i]]))
        .collect();
    let mut faces = vec![
        Face {
            point: bottom[0],
            normal: axis.map(|v| -v),
            edges: (0..6).collect(),
        },
        Face {
            point: top[0],
            normal: axis,
            edges: (6..12).collect(),
        },
    ];
    faces.extend((0..6).map(|i| {
        let next = (i + 1) % 6;
        let edge: Point = std::array::from_fn(|j| bottom[next][j] - bottom[i][j]);
        let normal = unit(std::array::from_fn(|j| {
            edge[(j + 1) % 3] * axis[(j + 2) % 3] - edge[(j + 2) % 3] * axis[(j + 1) % 3]
        }));
        Face {
            point: bottom[i],
            normal,
            edges: vec![i, i + 6, i + 12, next + 12],
        }
    }));
    Body { edges, faces }
}
