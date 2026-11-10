//! Shared trim samples for the five faces of a through-cap helical cut.
use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation, triangulate_region};
use spade::Point2;

const BOTTOM: u32 = 0;
const TOP: u32 = 1;
const CYLINDER: u32 = 2;
const CAVITY: u32 = 3;
const START_DISK: u32 = 4;

type Point = [f64; 3];

struct Chart {
    cut: Cut,
    width: f64,
    half_azimuth: f64,
    alpha: f64,
    low: f64,
    high: f64,
    bottom: f64,
}

impl Chart {
    fn new(cut: Cut) -> Self {
        let [x, y] = cut.rim;
        let shift = (cut.tangent * y).atan2(cut.spec.stock_radius_mm + x);
        let width = shift + cut.vertical * y / cut.rise;
        let low = cut.spec.start_height_mm - cut.vertical * y;
        let high = cut.spec.start_height_mm + cut.vertical * y;
        Self {
            cut,
            width,
            half_azimuth: shift,
            alpha: (x / cut.spec.tube_radius_mm).acos(),
            low,
            high,
            bottom: (low - (high - low)).max(low * 0.5),
        }
    }

    fn inside(&self, column: f64, row: f64) -> Point {
        let cut = self.cut;
        let y = (2. * column - 1.) * cut.rim[1];
        let shift = (cut.tangent * y / cut.spec.stock_radius_mm).asin();
        let end_z = cut.spec.start_height_mm - cut.vertical * y;
        let azimuth = shift + cut.vertical * y / cut.rise;
        self.cylinder(azimuth, mix(self.bottom, end_z, row))
    }

    fn cylinder(&self, azimuth: f64, z: f64) -> Point {
        let spec = self.cut.spec;
        let angle = spec.start_angle_rad + azimuth + (z - spec.start_height_mm) / self.cut.rise;
        let (sin, cos) = angle.sin_cos();
        let r = spec.stock_radius_mm;
        [r * cos, r * sin, z]
    }

    fn cavity(&self, column: f64, row: f64) -> Point {
        let cut = self.cut;
        let alpha = self.alpha + column * (TAU - 2. * self.alpha);
        let (s, c) = alpha.sin_cos();
        let [x, y] = [cut.spec.tube_radius_mm * c, cut.spec.tube_radius_mm * s];
        let top = cut.spec.stock_height_mm;
        let start = cut.spec.start_height_mm - cut.vertical * y;
        let z = mix(start, top, row);
        let theta = (z - start) / cut.rise;
        let (s, c) = (cut.spec.start_angle_rad + theta).sin_cos();
        let radial = cut.spec.stock_radius_mm + x;
        let side = cut.tangent * y;
        [radial * c - side * s, radial * s + side * c, z]
    }

    fn start_uv(&self, point: Point) -> [f64; 2] {
        let cut = self.cut;
        let (s, c) = cut.spec.start_angle_rad.sin_cos();
        [
            c * point[0] + s * point[1] - cut.spec.stock_radius_mm,
            cut.tangent * (c * point[1] - s * point[0])
                - cut.vertical * (point[2] - cut.spec.start_height_mm),
        ]
    }
}

struct Counts {
    inside: usize,
    outside: usize,
    cavity: usize,
    lower: usize,
    upper: usize,
    triangles: usize,
}

impl Counts {
    fn new(chart: &Chart, options: Tessellation) -> Result<Self, MeshError> {
        let cut = chart.cut;
        let radius = cut.spec.stock_radius_mm;
        let tube = cut.spec.tube_radius_mm;
        let rise = cut.rise;
        let y = cut.rim[1];
        let radial = radius + cut.rim[0];
        // Limit deflection below the physical gap between lifted turns.
        // z-rise*azimuth has local gradient norm < 4 near the stock radius.
        let turn_gap = cut.spec.pitch_mm
            - 2. * (cut.vertical * tube + rise * (cut.tangent * tube / (radius - tube)).asin());
        let deflection = options.deflection.min(tube / 16.).min(turn_gap / 16.);
        // On a cylinder only the angular span affects chord error. Divide
        // the cell's allowed span between its row and column directions.
        let phi_step = (2. * deflection / radius).sqrt().min(options.angle * 0.5);
        let count = |span: f64, step: f64, minimum: usize| {
            let n = ((span / step).ceil() as usize).max(minimum);
            if n > options.triangle_limit {
                Err(MeshError::BudgetExceeded)
            } else {
                Ok(n)
            }
        };
        // w'(y) = tangent/sqrt(radius²-(tangent*y)²) + vertical/rise.
        // The start-disk ellipse also needs a bound in its own plane.
        let inner_slope = cut.tangent / radial + cut.vertical / rise;
        let ellipse_second = (cut.tangent * radius).powi(2) / radial.powi(3);
        let y_step = (phi_step / inner_slope).min((8. * deflection / ellipse_second).sqrt());
        let inside = count(2. * y, y_step, 2)?;
        let outside = count(TAU - 2. * chart.half_azimuth, phi_step, 3)?;
        let lower = count((chart.high - chart.bottom) / rise, phi_step, 1)?;
        // The cavity chart uses t=s*T(alpha). Its second-order remainder
        // is bounded by a*d_alpha²+b*d_s² after bounding the mixed term.
        // For interpolation, the weighted parameter variances are at most
        // one quarter of each cell's squared width.
        let sweep = (cut.spec.stock_height_mm - chart.low) / rise;
        let q = cut.vertical * tube / rise;
        let outer = radius + 2. * tube;
        let speed = outer + rise;
        let a = outer * q * q + tube + speed * q;
        let b = outer * sweep * sweep + 0.5 * speed * q;
        let alpha_step = (2. * deflection / a)
            .sqrt()
            .min(options.angle / (2. * (1. + q)));
        let sweep_step = (2. * deflection / b).sqrt().min(phi_step / sweep);
        let cavity = count(TAU - 2. * chart.alpha, alpha_step, 2)?;
        let upper = count(1., sweep_step, 1)?;
        let sides = (inside + outside) as u64
            + (inside as u64 * lower as u64)
            + (outside as u64 * (lower + upper) as u64)
            + (cavity as u64 * upper as u64);
        let caps = 2 * (inside + outside + cavity) as u64 - 4;
        let triangles = 2 * sides + caps;
        if triangles > options.triangle_limit as u64 {
            return Err(MeshError::BudgetExceeded);
        }
        Ok(Self {
            inside,
            outside,
            cavity,
            lower,
            upper,
            triangles: triangles as usize,
        })
    }
}

fn mix(a: f64, b: f64, fraction: f64) -> f64 {
    if fraction == 1. {
        b
    } else {
        a + fraction * (b - a)
    }
}

fn fraction(i: usize, count: usize) -> f64 {
    i as f64 / count as f64
}

fn vertex(mesh: &mut DisplayMesh, point: Point) -> usize {
    let index = mesh.vertices.len() / 3;
    mesh.vertices.extend(point);
    index
}

struct Grid {
    stride: usize,
    indices: Vec<usize>,
}

impl Grid {
    fn new(columns: usize, rows: usize, mut sample: impl FnMut(usize, usize) -> usize) -> Self {
        let mut indices = Vec::with_capacity((columns + 1) * (rows + 1));
        for col in 0..=columns {
            for row in 0..=rows {
                indices.push(sample(col, row));
            }
        }
        Self {
            stride: rows + 1,
            indices,
        }
    }

    fn at(&self, col: usize, row: usize) -> usize {
        self.indices[col * self.stride + row]
    }

    fn tessellate(&self, mesh: &mut DisplayMesh, face: u32) {
        let columns = self.indices.len() / self.stride - 1;
        let rows = self.stride - 1;
        for col in 0..columns {
            for row in 0..rows {
                let [a, b, c, d] = [
                    self.at(col, row),
                    self.at(col + 1, row),
                    self.at(col + 1, row + 1),
                    self.at(col, row + 1),
                ];
                if face == CAVITY {
                    mesh.triangle([a, c, b], face);
                    mesh.triangle([a, d, c], face);
                } else {
                    mesh.triangle([a, b, c], face);
                    mesh.triangle([a, c, d], face);
                }
            }
        }
    }
}

fn planar_cap(
    mesh: &mut DisplayMesh,
    ring: Vec<usize>,
    project: impl Fn(Point) -> [f64; 2],
    face: u32,
) -> Result<(), MeshError> {
    let vertices = mesh.vertices.as_chunks::<3>().0;
    let points = ring
        .iter()
        .map(|&id| {
            let [x, y] = project(vertices[id]);
            Point2::new(x, y)
        })
        .collect();
    let edges = (0..ring.len())
        .map(|i| [i, (i + 1) % ring.len()])
        .collect::<Vec<_>>();
    for triangle in triangulate_region(points, &edges, 0)?.triangles {
        mesh.triangle(triangle.map(|i| ring[i]), face);
    }
    Ok(())
}

impl Cut {
    /// Tessellate the five exact supports with shared indexed trim edges.
    /// Face order is bottom, top, outer cylinder, cavity wall, start disk.
    pub fn mesh(self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let chart = Chart::new(self);
        let Counts {
            inside,
            outside,
            cavity,
            lower,
            upper,
            triangles,
        } = Counts::new(&chart, options)?;
        let mut mesh = DisplayMesh {
            vertices: Vec::with_capacity(3 * (triangles / 2 + 2)),
            indices: Vec::with_capacity(3 * triangles),
            face_indices: Vec::with_capacity(triangles),
        };

        // The interior chart strip ends at the elliptical edge of the
        // start disk. Its vertical ends are reused by the outer strip.
        let inside_grid = Grid::new(inside, lower, |col, row| {
            vertex(
                &mut mesh,
                chart.inside(fraction(col, inside), fraction(row, lower)),
            )
        });
        let outside_grid = Grid::new(outside, lower + upper, |col, row| {
            if row <= lower && col == 0 {
                return inside_grid.at(inside, row);
            }
            if row <= lower && col == outside {
                return inside_grid.at(0, row);
            }
            let t = fraction(col, outside);
            let azimuth = mix(chart.width, TAU - chart.width, t);
            let split = mix(chart.low, chart.high, t);
            let z = if row <= lower {
                mix(chart.bottom, split, fraction(row, lower))
            } else {
                mix(
                    split,
                    self.spec.stock_height_mm,
                    fraction(row - lower, upper),
                )
            };
            vertex(&mut mesh, chart.cylinder(azimuth, z))
        });
        let cavity_grid = Grid::new(cavity, upper, |col, row| {
            if col == 0 {
                return outside_grid.at(0, lower + row);
            }
            if col == cavity {
                return outside_grid.at(outside, lower + row);
            }
            vertex(
                &mut mesh,
                chart.cavity(fraction(col, cavity), fraction(row, upper)),
            )
        });

        inside_grid.tessellate(&mut mesh, CYLINDER);
        outside_grid.tessellate(&mut mesh, CYLINDER);
        cavity_grid.tessellate(&mut mesh, CAVITY);

        let base = (0..outside)
            .map(|col| outside_grid.at(col, 0))
            .chain((0..inside).map(|col| inside_grid.at(col, 0)))
            .collect::<Vec<_>>();
        // Below the groove the cylinder has straight generators. Carry
        // this ring directly down to Z=0 with no extra helical subdivision.
        let bottom = base
            .iter()
            .map(|&id| {
                let [x, y, _] = mesh.vertices.as_chunks::<3>().0[id];
                vertex(&mut mesh, [x, y, 0.])
            })
            .collect::<Vec<_>>();
        let center = vertex(&mut mesh, [0.; 3]);
        for i in 0..base.len() {
            let j = (i + 1) % base.len();
            let [a, b] = [bottom[i], bottom[j]];
            mesh.triangle([a, b, base[j]], CYLINDER);
            mesh.triangle([a, base[j], base[i]], CYLINDER);
            mesh.triangle([center, b, a], BOTTOM);
        }
        let top = (0..=outside)
            .map(|col| outside_grid.at(col, lower + upper))
            .chain((1..cavity).rev().map(|col| cavity_grid.at(col, upper)))
            .collect();
        planar_cap(&mut mesh, top, |[x, y, _]| [x, y], TOP)?;
        let floor = (0..=cavity)
            .map(|col| cavity_grid.at(col, 0))
            .chain((1..inside).map(|col| inside_grid.at(col, lower)))
            .collect();
        planar_cap(&mut mesh, floor, |p| chart.start_uv(p), START_DISK)?;
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests;
