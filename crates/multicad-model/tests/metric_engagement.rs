//! Fit context must not truncate manufacturing geometry or suppress validation.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::*,
};
use multicad_kernel::Solid;
use multicad_model::Compiler;
#[test]
fn declared_engagement_keeps_the_entire_long_native_thread_and_step_extent() {
    let mut prior = None;
    let dir = tempfile::tempdir().unwrap();
    for (index, intent) in [
        MetricFitEngagement::IntendedLength { length: length(3.) },
        MetricFitEngagement::IntendedLength { length: length(4.) },
        MetricFitEngagement::NormalGroupAssumed {},
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = Session::new(Document::new("Long fitted screw").unwrap()).unwrap();
        let request = ThreadRequest::new(
            "M3 full 8 mm",
            ThreadParameters {
                major_diameter: length(3.),
                pitch: length(0.5),
                length: length(8.),
                fit: ThreadFit::Metric6gExternal,
                metric_fit_engagement: Some(intent),
                ..Default::default()
            },
        );
        let q = prepare_thread(&s, 0, Actor::local(Channel::Mcp), &request).unwrap();
        s.commit(q.prepared).unwrap();
        let model = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(s.document(), s.revision())
        .unwrap();
        let solid = model.solid(request.feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let bounds = solid.bounds().unwrap();
        assert!((bounds.min[2]).abs() < 1e-5);
        assert!((bounds.max[2] - 8.).abs() < 1e-5);
        let volume = solid.volume_mm3().unwrap();
        if let Some(v) = prior {
            assert_eq!(volume, v);
        }
        prior = Some(volume);
        if index == 0 {
            let path = dir.path().join("long-metric.step");
            solid.write_step(&path).unwrap();
            let back = Solid::read_step(&path, Default::default()).unwrap();
            assert!((back.bounds().unwrap().max[2] - 8.).abs() < 1e-5);
            let area: f64 = solid.faces().unwrap().iter().map(|f| f.area_mm2).sum();
            let budget = multicad_kernel::GeometryPolicy::default().linear_tolerance_mm * area
                + 1e-7 * volume;
            assert!((back.volume_mm3().unwrap() - volume).abs() < budget);
        }
    }
}
