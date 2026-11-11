//! Exercise the real external-file admission budget independently of unit-test builds.
use multicad_rust_kernel::{
    normal_relief::{Kind, Relief},
    normal_sweep::{Edge, Sweep},
    step::Writer,
    step_read::{ReadError, read_analytic_solid, read_import_solid},
};
const TOL: f64 = 1e-7;
fn fixture(centers: impl Iterator<Item = [f64; 2]>) -> Relief {
    let p = [[0., 0.], [100., 0.], [100., 80.], [0., 80.]];
    let base = Sweep::new(
        vec![
            (0..4)
                .map(|i| Edge::Line {
                    a: p[i],
                    b: p[(i + 1) % 4],
                })
                .collect(),
        ],
        4.,
        TOL,
        1e-5,
    )
    .unwrap();
    let motifs = centers
        .map(|center| {
            Sweep::new(
                vec![vec![Edge::Circle {
                    center,
                    radius_mm: 0.5,
                    clockwise: false,
                }]],
                0.5,
                TOL,
                1e-5,
            )
            .unwrap()
        })
        .collect();
    Relief::new(base, motifs, Kind::Raised, TOL, 1e-5).unwrap()
}
#[path = "../src/step_read/normal_relief/tests/external_fixture.rs"]
mod external_fixture;
use external_fixture::external_193_motif_input;
#[test]
fn externally_valid_193_motif_shell_hits_typed_limit_and_public_reader_recovers() {
    let base = fixture((0..192).map(|i| [3. + (i % 16) as f64 * 6., 3. + (i / 16) as f64 * 6.]));
    let extra = fixture(std::iter::once([3., 75.]));
    let mut writer = Writer::new();
    for body in [&base, &extra] {
        writer
            .append_normal_relief(body, [0.; 3], [1., 0., 0.], [0., 1., 0.])
            .unwrap();
    }
    let source = writer.finish().unwrap();
    let external = external_193_motif_input(&source);
    let proof = read_analytic_solid(&external, TOL).unwrap();
    assert_eq!(proof.solid_count, 1);
    assert_eq!(proof.face_count, 392);
    assert!(matches!(
        read_import_solid(&external, TOL),
        Err(ReadError::Limit)
    ));
    let valid = multicad_rust_kernel::step::write_normal_relief(
        &extra,
        [0.; 3],
        [1., 0., 0.],
        [0., 1., 0.],
    )
    .unwrap();
    assert!(read_import_solid(&valid, TOL).is_ok());
}
