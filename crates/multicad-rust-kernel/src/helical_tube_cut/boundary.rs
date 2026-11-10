//! Parametric boundary and surface integrals of the certified helical cut.
use super::*;

type Point = [f64; 3];

#[derive(Clone, Copy, Debug)]
pub enum BoundaryEdge {
    Bottom,
    TopRim,
    FloorRim,
    FloorCylinder,
    TopCavity,
    Rail { positive: bool },
}

#[derive(Clone, Copy, Debug)]
pub struct FaceProperties {
    pub area_mm2: f64,
    pub center_mm: Point,
}

impl Cut {
    pub fn spec(self) -> Spec {
        self.spec
    }

    pub fn floor_frame(self) -> (Point, Point, Point) {
        let (sin, cos) = self.spec.start_angle_rad.sin_cos();
        let radial = [cos, sin, 0.];
        let transverse = [-sin * self.tangent, cos * self.tangent, -self.vertical];
        (
            [
                self.spec.stock_radius_mm * cos,
                self.spec.stock_radius_mm * sin,
                self.spec.start_height_mm,
            ],
            radial,
            transverse,
        )
    }

    pub fn edge_domain(self, edge: BoundaryEdge) -> [f64; 2] {
        match edge {
            BoundaryEdge::Bottom => [0., TAU],
            BoundaryEdge::TopRim => {
                let [a, b] = self.cylinder_trims().map(|rail| rail.azimuth_rad[1]);
                [b, a + TAU]
            }
            BoundaryEdge::FloorRim | BoundaryEdge::TopCavity => {
                let first = (self.rim[0] / self.spec.tube_radius_mm).acos();
                [first, TAU - first]
            }
            BoundaryEdge::FloorCylinder => {
                let limit = (self.tangent * self.rim[1] / self.spec.stock_radius_mm).asin();
                [-limit, limit]
            }
            BoundaryEdge::Rail { positive } => {
                self.cylinder_trims()[usize::from(positive)].azimuth_rad
            }
        }
    }

    /// Point on a boundary edge in the stock frame. Parameters use the domain
    /// of the requested edge, preserving both helical turns and cap joins.
    pub fn edge_point(self, edge: BoundaryEdge, parameter: f64) -> Point {
        let radius = self.spec.stock_radius_mm;
        let tube = self.spec.tube_radius_mm;
        let (sin, cos) = parameter.sin_cos();
        match edge {
            BoundaryEdge::Bottom => [radius * cos, radius * sin, 0.],
            BoundaryEdge::TopRim => [radius * cos, radius * sin, self.spec.stock_height_mm],
            BoundaryEdge::Rail { positive } => {
                let trim = self.cylinder_trims()[usize::from(positive)];
                let z = self
                    .rise
                    .mul_add(parameter - trim.azimuth_rad[0], trim.height_mm[0]);
                [radius * cos, radius * sin, z]
            }
            BoundaryEdge::FloorCylinder => {
                let (sin_phase, cos_phase) = self.spec.start_angle_rad.sin_cos();
                [
                    radius * (cos_phase * cos - sin_phase * sin),
                    radius * (sin_phase * cos + cos_phase * sin),
                    self.spec.start_height_mm - self.vertical * radius * sin / self.tangent,
                ]
            }
            BoundaryEdge::FloorRim | BoundaryEdge::TopCavity => {
                let x = tube * cos;
                let y = tube * sin;
                let (turn, z) = match edge {
                    BoundaryEdge::FloorRim => (0., self.spec.start_height_mm - self.vertical * y),
                    _ => (self.cap_angle(y), self.spec.stock_height_mm),
                };
                let [x, y] = self.rotated_xy(turn, x, y);
                [x, y, z]
            }
        }
    }

    /// Upper bound on the magnitude of the curve's second derivative in
    /// its parameter. It bounds linear chord error by M*step²/8.
    pub fn edge_second_derivative_bound(self, edge: BoundaryEdge) -> f64 {
        let r = self.spec.tube_radius_mm;
        let radius = self.spec.stock_radius_mm;
        match edge {
            BoundaryEdge::Bottom | BoundaryEdge::TopRim | BoundaryEdge::Rail { .. } => radius,
            BoundaryEdge::FloorRim => r,
            BoundaryEdge::FloorCylinder => radius / self.tangent,
            BoundaryEdge::TopCavity => {
                let k = self.vertical * r / self.rise;
                (radius + r) * (k + k * k) + 2. * r * k + r
            }
        }
    }

    /// Areas use closed-form integration. The centers of area use smooth
    /// one-dimensional moment integrals, after integrating X and sweep angle.
    /// Order is stock bottom, stock top, cylinder, cavity and start disk.
    pub fn face_properties(self) -> [FaceProperties; 5] {
        let radius = self.spec.stock_radius_mm;
        let tube = self.spec.tube_radius_mm;
        let height = self.spec.stock_height_mm;
        let start = self.spec.start_height_mm;
        let [_, half] = self.rim;
        let center_sweep = self.cap_angle(0.);
        let start_plane_area = 2. * self.profile_half_area();
        let top_removed = self.removed_volume_mm3() / (height - start);
        let cylinder_removed = 2.
            * radius
            * center_sweep
            * (self.vertical * half + self.rise * (self.tangent * half / radius).asin());
        let cavity_angles = self.edge_domain(BoundaryEdge::FloorRim);
        let [a, b] = cavity_angles;
        let cavity_area = tube
            * center_sweep
            * (radius / self.vertical * (b - a) + self.vertical * tube * (b.sin() - a.sin()));

        let beta = (half / tube).asin();
        let caps = integrate([-beta, beta], |t| {
            let (sin, cos) = t.sin_cos();
            let y = tube * sin;
            let dy = tube * cos;
            let lo = -dy;
            let squared_tangent = (self.tangent * y).powi(2);
            let hi = -squared_tangent / (radius + (radius * radius - squared_tangent).sqrt());
            let width = hi - lo;
            let x_mean = (lo + hi) * 0.5;
            let floor = self.rotated_xy(0., x_mean, y);
            // The top Jacobian is affine in radial x; its weighted first
            // moment adds the interval's variance times the slope.
            let top_weight = self.tangent + self.vertical * (radius + x_mean) / self.rise;
            let top_mean = x_mean + self.vertical * width.powi(2) / (12. * self.rise * top_weight);
            let top = self.rotated_xy(self.cap_angle(y), top_mean, y);
            [
                floor[0],
                floor[1],
                start - self.vertical * y,
                top[0] * top_weight,
                top[1] * top_weight,
            ]
            .map(|value| value * width * dy)
        });
        let wall = integrate([-beta, beta], |t| {
            let (sin, cos) = t.sin_cos();
            let y = tube * sin;
            let transverse = self.tangent * y;
            let radial = (radius * radius - transverse * transverse).sqrt();
            let jacobian =
                radius * (self.vertical + self.rise * self.tangent / radial) * tube * cos;
            self.swept_moment(radial, transverse, y)
                .map(|value| value * jacobian)
        });
        let cavity = integrate(cavity_angles, |t| {
            let (sin, cos) = t.sin_cos();
            let x = tube * cos;
            let y = tube * sin;
            let jacobian = tube * (radius / self.vertical + self.vertical * x);
            self.swept_moment(radius + x, self.tangent * y, y)
                .map(|value| value * jacobian)
        });
        let cap = PI * radius * radius;
        let cylinder = TAU * radius * height;
        let top = cap - top_removed;
        let retained_wall = cylinder - cylinder_removed;
        [
            FaceProperties {
                area_mm2: cap,
                center_mm: [0.; 3],
            },
            FaceProperties {
                area_mm2: top,
                center_mm: [-caps[3] / top, -caps[4] / top, height],
            },
            FaceProperties {
                area_mm2: retained_wall,
                center_mm: [
                    -wall[0] / retained_wall,
                    -wall[1] / retained_wall,
                    (cylinder * height * 0.5 - wall[2]) / retained_wall,
                ],
            },
            FaceProperties {
                area_mm2: cavity_area,
                center_mm: cavity.map(|moment| moment / cavity_area),
            },
            FaceProperties {
                area_mm2: start_plane_area,
                center_mm: [caps[0], caps[1], caps[2]].map(|moment| moment / start_plane_area),
            },
        ]
    }

    fn cap_angle(self, y: f64) -> f64 {
        (self.spec.stock_height_mm - self.spec.start_height_mm + self.vertical * y) / self.rise
    }

    fn rotated_xy(self, turn: f64, x: f64, y: f64) -> [f64; 2] {
        let (sin, cos) = (turn + self.spec.start_angle_rad).sin_cos();
        let a = self.spec.stock_radius_mm + x;
        let b = self.tangent * y;
        [a * cos - b * sin, a * sin + b * cos]
    }

    fn profile_half_area(self) -> f64 {
        let radius = self.spec.stock_radius_mm;
        let tube = self.spec.tube_radius_mm;
        let y = self.rim[1];
        let alpha = (self.tangent * y / radius).asin();
        let beta = (y / tube).asin();
        let disk = 0.5 * (y * (-self.rim[0]) + tube * tube * beta);
        // Compute the very small difference from the rectangle without
        // subtracting two large R*y terms.
        let sine = alpha.sin();
        let difference = if alpha < 0.01 {
            let a2 = alpha * alpha;
            alpha * a2 * (-1. / 3. + a2 * (7. / 60. - a2 * (31. / 2520.)))
        } else {
            alpha + sine * (alpha.cos() - 2.)
        };
        disk + radius * radius * difference / (2. * self.tangent)
    }

    fn swept_moment(self, x: f64, y: f64, profile_y: f64) -> Point {
        let sweep = self.cap_angle(profile_y);
        let phase = self.spec.start_angle_rad;
        let (sin, cos) = (phase + sweep * 0.5).sin_cos();
        let factor = 2. * (sweep * 0.5).sin();
        [
            factor * (x * cos - y * sin),
            factor * (x * sin + y * cos),
            sweep
                * (self.spec.start_height_mm - self.vertical * profile_y + self.rise * sweep * 0.5),
        ]
    }
}

fn integrate<const N: usize>([a, b]: [f64; 2], value: impl Fn(f64) -> [f64; N]) -> [f64; N] {
    // Composite symmetric Gauss–Legendre on the smooth angular domain.
    const NODES: [(f64, f64); 4] = [
        (0.183_434_642_495_649_8, 0.362_683_783_378_362),
        (0.525_532_409_916_329, 0.313_706_645_877_887_3),
        (0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
        (0.960_289_856_497_536_3, 0.101_228_536_290_376_3),
    ];
    let panels = 16;
    let half = (b - a) / (2. * panels as f64);
    let mut result = [0.; N];
    for panel in 0..panels {
        let center = (2. * panel as f64 + 1.).mul_add(half, a);
        for (node, weight) in NODES {
            let [left, right] = [-node, node].map(|t| value(t.mul_add(half, center)));
            for i in 0..N {
                result[i] += (left[i] + right[i]) * weight * half;
            }
        }
    }
    result
}
