use super::*;

const TOLERANCE: f64 = 1e-7;

fn spec(start_height_mm: f64) -> Spec {
    Spec {
        stock_radius_mm: 8.,
        stock_height_mm: 30.,
        tube_radius_mm: 0.6,
        pitch_mm: 2.,
        path_height_mm: 15.,
        start_height_mm,
        start_angle_rad: 0.,
    }
}

fn point(cut: Cut, theta: f64, x: f64, y: f64) -> [f64; 3] {
    let (s, c) = (theta + cut.spec.start_angle_rad).sin_cos();
    let radial = cut.spec.stock_radius_mm + x;
    [
        radial * c - cut.tangent * y * s,
        radial * s + cut.tangent * y * c,
        cut.spec.start_height_mm + cut.rise * theta - cut.vertical * y,
    ]
}

fn top_covers(mesh: &crate::display::DisplayMesh, p: [f64; 3]) -> bool {
    let vertices = mesh.vertices.as_chunks::<3>().0;
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.face_indices)
        .any(|(&triangle, &face)| {
            if face != 1 {
                return false;
            }
            let [a, b, c] = triangle.map(|index| vertices[index as usize]);
            let side = |a: [f64; 3], b: [f64; 3]| {
                (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
            };
            side(a, b) >= 0. && side(b, c) >= 0. && side(c, a) >= 0.
        })
}

#[test]
fn tessellation_keeps_the_groove_open_with_conformal_trims_and_outward_volume() {
    use crate::{display::Tessellation, manufacturing};

    let coarse = Tessellation {
        deflection: 0.07,
        angle: 0.5,
        triangle_limit: 1_000_000,
    };
    let fine = Tessellation {
        deflection: 0.02,
        angle: 0.35,
        ..coarse
    };
    for start in [16.0, 16.2, 16.6] {
        for phase in [-PI, -0.1, 0., 0.1, PI / 2., PI] {
            let cut = Cut::new(
                Spec {
                    start_angle_rad: phase,
                    ..spec(start)
                },
                TOLERANCE,
            )
            .unwrap();
            let mesh = cut.mesh(coarse).unwrap();
            let proof = manufacturing::certify(&mesh).unwrap();
            assert_eq!(proof.vertex_links as usize, mesh.vertices.len() / 3);
            assert_eq!(
                i64::from(proof.vertex_links) - i64::from(proof.edge_pairs)
                    + mesh.face_indices.len() as i64,
                2
            );
            assert!(
                (proof.signed_volume_mm3 - cut.volume_mm3()).abs() < 20.,
                "{proof:?}"
            );
            let mut seen = [false; 5];
            let vertices = mesh.vertices.as_chunks::<3>().0;
            for (&triangle, &face) in mesh
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .zip(&mesh.face_indices)
            {
                seen[face as usize] = true;
                let [a, b, c] = triangle.map(|v| vertices[v as usize]);
                if face == 2 {
                    let center = [(a[0] + b[0] + c[0]) / 3., (a[1] + b[1] + c[1]) / 3.];
                    assert!(8. - center[0].hypot(center[1]) < coarse.deflection);
                }
            }
            assert_eq!(seen, [true; 5]);
            for &vertex in vertices {
                assert_eq!(cut.classify(vertex), Ok(Location::Boundary), "{vertex:?}");
            }
            let opening = point(cut, (30. - start) / cut.rise, -0.3, 0.);
            assert!(!top_covers(&mesh, opening));
            assert!(top_covers(&mesh, [0., 0., 30.]));
            if phase == 0. && start == 16.2 {
                let refined = cut.mesh(fine).unwrap();
                let measured = manufacturing::certify(&refined).unwrap().signed_volume_mm3;
                assert!(refined.face_indices.len() > mesh.face_indices.len());
                assert!(
                    (measured - cut.volume_mm3()).abs()
                        < (proof.signed_volume_mm3 - cut.volume_mm3()).abs()
                );
                assert!(!top_covers(&refined, opening));
            }
        }
    }
}

#[test]
fn tessellation_validates_options_and_limits_work_before_allocating() {
    use crate::display::{MeshError, Tessellation};
    let cut = Cut::new(spec(16.2), TOLERANCE).unwrap();
    let coarse = Tessellation {
        deflection: 0.07,
        angle: 0.5,
        triangle_limit: 1_000_000,
    };
    assert_eq!(
        cut.mesh(Tessellation {
            deflection: 0.,
            ..coarse
        }),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        cut.mesh(Tessellation {
            deflection: f64::MIN_POSITIVE,
            ..coarse
        }),
        Err(MeshError::BudgetExceeded)
    );
    let triangles = cut.mesh(coarse).unwrap().face_indices.len();
    assert_eq!(
        cut.mesh(Tessellation {
            triangle_limit: triangles - 1,
            ..coarse
        }),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        cut.mesh(Tessellation {
            triangle_limit: triangles,
            ..coarse
        })
        .unwrap()
        .face_indices
        .len(),
        triangles
    );
}

#[test]
fn curved_patch_sampling_respects_the_requested_chord_bound() {
    let cut = Cut::new(spec(16.2), TOLERANCE).unwrap();
    let chart = Chart::new(cut);
    let options = Tessellation {
        deflection: 0.012,
        angle: 0.2,
        triangle_limit: 1_000_000,
    };
    let counts = Counts::new(&chart, options).unwrap();
    for col in [0, counts.cavity / 2, counts.cavity - 1] {
        for row in [0, counts.upper / 2, counts.upper - 1] {
            for indices in [
                [[col, row], [col + 1, row], [col + 1, row + 1]],
                [[col, row], [col + 1, row + 1], [col, row + 1]],
            ] {
                let parameters = indices
                    .map(|[col, row]| [fraction(col, counts.cavity), fraction(row, counts.upper)]);
                let points = parameters.map(|[u, v]| chart.cavity(u, v));
                let mean = |values: [f64; 3]| values.into_iter().sum::<f64>() / 3.;
                let exact = chart.cavity(
                    mean(parameters.map(|p| p[0])),
                    mean(parameters.map(|p| p[1])),
                );
                let error =
                    std::array::from_fn::<_, 3, _>(|i| mean(points.map(|p| p[i])) - exact[i]);
                assert!(error.into_iter().map(|x| x * x).sum::<f64>().sqrt() < options.deflection);
            }
        }
    }
}
