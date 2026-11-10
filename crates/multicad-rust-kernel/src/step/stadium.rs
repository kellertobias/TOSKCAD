use super::{
    Entity, Placement, Point, StepError, StepFile, Vertex, cross, direction, dot, line_edge,
    write_one,
};
use crate::{
    orthogonal_boundary::BoundaryFace, orthogonal_pockets::FacePlane,
    orthogonal_slots::OrthogonalSlot, stadium::Stadium,
};

/// Write a stadium prism with two exact semicylinders and four planar faces.
pub fn write_stadium_prism(
    profile: &Stadium,
    height: f64,
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_stadium_prism(profile, height, origin, u, v, reverse_sweep))
}

pub(super) fn encode_stadium_prism(
    file: &mut StepFile,
    profile: &Stadium,
    height: f64,
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<Entity, StepError> {
    if !height.is_finite() || height <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let placement = Placement::new(origin, u, v)?;
    let levels = if reverse_sweep {
        [-height, 0.]
    } else {
        [0., height]
    };
    let [low, high] = levels.map(|z| StadiumLevel::new(*profile, |xy| placement.point_f64(xy, z)));
    let trim = StadiumTrim::new(file, [low, high], placement.normal, profile.radius_mm())?;
    let mut faces = vec![trim.cap(file, 0, false, u), trim.cap(file, 1, true, u)];
    faces.extend(trim.walls(file, true));
    Ok(file.solid(&faces))
}

struct StadiumLevel {
    corners: [Point; 4],
    centers: [Point; 2],
}

impl StadiumLevel {
    fn new(profile: Stadium, placed: impl Fn([f64; 2]) -> Point) -> Self {
        Self {
            corners: profile.corners().map(&placed),
            centers: profile.centers().map(placed),
        }
    }
}

struct Rim {
    corners: [Vertex; 4],
    edges: [Entity; 4],
    circle_axes: [Entity; 2],
    spine: Point,
}

impl Rim {
    fn new(
        file: &mut StepFile,
        level: StadiumLevel,
        normal: Point,
        radius: f64,
    ) -> Result<Self, StepError> {
        let corners = level.corners.map(|point| file.vertex(point));
        let [a, b, c, d] = corners;
        let lower = line_edge(file, a, b)?;
        let upper = line_edge(file, c, d)?;
        let mut arc = |center, from: Vertex, to: Vertex| {
            let reference = direction(center, from.point)?;
            // World placement must preserve the vertices on the semicircle
            // to the distance accuracy declared by the STEP file.
            let error = |point: Point, sign: f64| {
                let [x, y, z]: Point =
                    std::array::from_fn(|i| point[i] - center[i] - sign * radius * reference[i]);
                x.hypot(y).hypot(z)
            };
            if radius * dot(reference, normal).abs() > 1e-7
                || error(from.point, 1.) > 1e-7
                || error(to.point, -1.) > 1e-7
            {
                return Err(StepError::InvalidGeometry);
            }
            let center = file.point(center);
            let axis = file.axis(center, normal, reference);
            let circle = file.entity(format_args!("CIRCLE('',{axis},{radius:.16E})"));
            let (start, end) = (from.vertex, to.vertex);
            let edge = file.entity(format_args!("EDGE_CURVE('',{start},{end},{circle},.T.)"));
            Ok((edge, axis))
        };
        let (arc_b, axis_b) = arc(level.centers[1], b, c)?;
        let (arc_a, axis_a) = arc(level.centers[0], d, a)?;
        Ok(Self {
            corners,
            edges: [lower, arc_b, upper, arc_a],
            circle_axes: [axis_b, axis_a],
            spine: direction(a.point, b.point)?,
        })
    }
}

pub(super) struct StadiumTrim {
    rims: [Rim; 2],
    rails: [Entity; 4],
    normal: Point,
    radius: f64,
}

impl StadiumTrim {
    fn new(
        file: &mut StepFile,
        [low, high]: [StadiumLevel; 2],
        normal: Point,
        radius: f64,
    ) -> Result<Self, StepError> {
        let low = Rim::new(file, low, normal, radius)?;
        let high = Rim::new(file, high, normal, radius)?;
        let mut rail = |i: usize| line_edge(file, low.corners[i], high.corners[i]);
        let rails = [rail(0)?, rail(1)?, rail(2)?, rail(3)?];
        Ok(Self {
            rims: [low, high],
            rails,
            normal,
            radius,
        })
    }

    pub(super) fn blind(
        file: &mut StepFile,
        slot: OrthogonalSlot,
        face: &BoundaryFace,
        normal: Point,
        transform: &dyn Fn(Point) -> Point,
    ) -> Result<Self, StepError> {
        let plane = FacePlane::from_face(face).ok_or(StepError::InvalidGeometry)?;
        let profile = slot.profile(face).map_err(|_| StepError::InvalidGeometry)?;
        let levels = [slot.depth_mm, 0.].map(|depth| {
            StadiumLevel::new(profile, |xy| {
                let mouth = plane.lift(xy);
                transform(std::array::from_fn(|i| mouth[i] - depth * face.normal[i]))
            })
        });
        Self::new(file, levels, normal, slot.radius_mm)
    }

    pub(super) fn bound(
        &self,
        file: &mut StepFile,
        level: usize,
        forward: bool,
        outer: bool,
    ) -> Entity {
        let mut edges = self.rims[level].edges;
        if !forward {
            edges.reverse();
        }
        let edges = edges.map(|edge| file.oriented_edge(edge, forward));
        file.bound(&edges, outer)
    }

    pub(super) fn cap(
        &self,
        file: &mut StepFile,
        level: usize,
        forward: bool,
        reference: Point,
    ) -> Entity {
        let bound = self.bound(file, level, forward, true);
        let normal = self
            .normal
            .map(|coordinate| if forward { coordinate } else { -coordinate });
        file.face(
            &[bound],
            self.rims[level].corners[0].cartesian,
            normal,
            reference,
        )
    }

    pub(super) fn walls(&self, file: &mut StepFile, outward: bool) -> [Entity; 4] {
        let [low, high] = &self.rims;
        std::array::from_fn(|i| {
            let mut trim = [
                (low.edges[i], true),
                (self.rails[(i + 1) % 4], true),
                (high.edges[i], false),
                (self.rails[i], false),
            ];
            if !outward {
                trim.reverse();
            }
            let edges = trim.map(|(edge, forward)| file.oriented_edge(edge, forward == outward));
            let bound = file.bound(&edges, true);
            if i % 2 == 0 {
                let tangent = low.spine.map(|value| if i == 0 { value } else { -value });
                let normal =
                    cross(tangent, self.normal).map(|value| if outward { value } else { -value });
                file.face(&[bound], low.corners[i].cartesian, normal, tangent)
            } else {
                let axis = low.circle_axes[i / 2];
                let radius = self.radius;
                let cylinder =
                    file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"));
                file.face_on(&[bound], cylinder, outward)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        orthogonal::{Box3, Operation, boolean},
        orthogonal_boundary::build,
        orthogonal_pockets::OrthogonalPocket,
        step::{
            supports_pocketed_orthogonal, supports_slotted_orthogonal, write_slotted_orthogonal,
        },
    };

    #[test]
    fn stadium_step_uses_two_partial_cylinders_and_supports_both_sweep_directions() {
        let profile = Stadium::new([[2., 3.], [8., 3.]], 1.5).unwrap();
        let u = [1., 0., 0.];
        let v = [0., 1., 0.];
        let run =
            |height, origin, reverse| write_stadium_prism(&profile, height, origin, u, v, reverse);
        for (reverse, far_z) in [
            (false, "4.0000000000000000E0"),
            (true, "-4.0000000000000000E0"),
        ] {
            let text = run(4., [0.; 3], reverse).unwrap();
            assert_eq!(text.matches("CIRCLE(").count(), 4);
            assert_eq!(text.matches("CYLINDRICAL_SURFACE(").count(), 2);
            assert_eq!(text.matches("ADVANCED_FACE(").count(), 6);
            assert_eq!(text.matches("EDGE_CURVE(").count(), 12);
            assert_eq!(text.matches("ORIENTED_EDGE(").count(), 24);
            assert!(text.contains(far_z));
        }
        assert_eq!(run(0., [0.; 3], false), Err(StepError::InvalidGeometry));
        assert_eq!(
            run(1., [0., f64::NAN, 0.], false),
            Err(StepError::InvalidPlacement)
        );
        assert_eq!(
            run(f64::MIN_POSITIVE, [0., 0., 1.], false),
            Err(StepError::InvalidGeometry)
        );
        assert_eq!(
            run(4., [0., 1e16, 0.], false),
            Err(StepError::InvalidGeometry)
        );
    }

    #[test]
    fn blind_slot_step_uses_exact_holes_and_keeps_round_pockets() {
        let stock = boolean(
            &[Box3 {
                min: [0.; 3],
                max: [20., 16., 10.],
            }],
            &[],
            Operation::Subtract,
            1e-7,
        )
        .unwrap()
        .unwrap();
        let boundary = build(stock.quads()).unwrap();
        let face = boundary
            .faces
            .iter()
            .position(|face| face.normal == [0., 0., 1.])
            .unwrap();
        let pocket = OrthogonalPocket {
            face,
            center_mm: [16., 7., 10.],
            radius_mm: 1.,
            depth_mm: 2.,
        };
        let slot = OrthogonalSlot {
            face,
            centers_mm: [[4., 7., 10.], [9., 7., 10.]],
            radius_mm: 1.5,
            depth_mm: 3.,
        };
        let place = |[x, y, z]: Point| [y + 7., -x + 2., z + 9.];
        let run = |slot| write_slotted_orthogonal(&stock, &boundary, &[pocket], &[slot], &place);
        assert!(supports_pocketed_orthogonal(&boundary, &[pocket]));
        let text = run(slot).unwrap();
        assert_eq!(text.matches("CIRCLE(").count(), 6);
        assert_eq!(text.matches("CYLINDRICAL_SURFACE(").count(), 3);
        assert_eq!(
            text.matches("ADVANCED_FACE(").count(),
            boundary.faces.len() + 7
        );
        assert_eq!(text.matches("FACE_BOUND(").count(), 2);
        assert_eq!(
            run(OrthogonalSlot {
                face: boundary.faces.len(),
                ..slot
            }),
            Err(StepError::Unsupported)
        );
        assert_eq!(
            run(OrthogonalSlot {
                depth_mm: 12.,
                ..slot
            }),
            Err(StepError::InvalidGeometry)
        );
        assert!(!supports_slotted_orthogonal(
            &boundary,
            &[pocket],
            &[slot; crate::polygon::MAX_VERTICES / 4],
        ));
    }
}
