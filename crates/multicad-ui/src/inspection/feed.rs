use super::*;
use std::sync::Arc;
pub trait MeasurementBackend {
    fn request(
        &mut self,
        document: &Document,
        stamp: MeasurementStamp,
        query: Option<&MeasurementQuery>,
    ) -> Result<u64, String>;
    fn poll(&mut self) -> Option<MeasurementReply>;
    fn cancel(&mut self) {}
}
pub type MeasurementBackendFactory = Arc<dyn Fn() -> Box<dyn MeasurementBackend> + Send + Sync>;
#[derive(Default)]
pub(super) struct Feed {
    factory: Option<MeasurementBackendFactory>,
    backend: Option<Box<dyn MeasurementBackend>>,
    pub stamp: Option<MeasurementStamp>,
    pub pins: Vec<PinnedMeasurement>,
    expected: Option<(u64, MeasurementStamp, Option<MeasurementQuery>)>,
    requested: Option<(MeasurementStamp, Option<MeasurementQuery>)>,
    pub batch: Option<Arc<MeasurementBatch>>,
    pub error: Option<String>,
}
impl Feed {
    pub fn set_factory(&mut self, factory: MeasurementBackendFactory) {
        if let Some(b) = self.backend.as_mut() {
            b.cancel();
        }
        self.factory = Some(factory);
        self.backend = None;
        self.expected = None;
        self.requested = None;
        self.batch = None;
        self.error = None;
    }
    pub fn has_factory(&self) -> bool {
        self.factory.is_some()
    }
    pub fn pending(&self) -> bool {
        self.expected.is_some()
    }
    pub fn tick(&mut self, session: &Session, query: Option<&MeasurementQuery>, active: bool) {
        let key = (session.document().id(), session.revision());
        if self.stamp.is_none_or(|s| (s.document, s.revision) != key) {
            self.batch = None;
            self.requested = None;
            match MeasurementStamp::capture(session.document(), session.revision()) {
                Ok(stamp) => self.stamp = Some(stamp),
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
            match multicad_core::inspection::pins(session.document())
                .map(|id| {
                    multicad_core::inspection::read(session.document(), id)
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(pins) => self.pins = pins,
                Err(error) => {
                    self.error = Some(error);
                    self.pins.clear();
                    return;
                }
            }
        }
        let stamp = self.stamp.expect("captured source");
        let needed = query.is_some()
            || self.pins.iter().any(|pin| pin.visible)
            || (active && !self.pins.is_empty());
        if !needed {
            if let Some(b) = self.backend.as_mut() {
                b.cancel();
            }
            self.backend = None;
            self.expected = None;
            self.requested = None;
            self.batch = None;
            self.error = None;
            return;
        }
        if self.backend.is_none() {
            self.backend = self.factory.as_ref().map(|f| f());
        }
        let Some(backend) = self.backend.as_mut() else {
            self.error = Some("Native measurement provider is unavailable".into());
            return;
        };
        while let Some(reply) = backend.poll() {
            let Some((id, expected, query)) = self.expected.clone() else {
                continue;
            };
            if reply.request != id {
                continue;
            }
            if reply.stamp != expected {
                self.expected = None;
                discard_transient(&mut self.batch);
                self.error = Some("Measurement reply has a foreign source stamp".into());
                continue;
            }
            if reply.stamp != stamp {
                continue;
            }
            let checked = reply.result.and_then(|batch| {
                stamp.check(session.document(), session.revision())?;
                if batch.stamp != stamp {
                    return Err("Measurement batch differs from its accepted source".into());
                }
                if batch.transient.as_ref().map(|m| &m.query) != query.as_ref() {
                    return Err("Measurement reply belongs to different A/B references".into());
                }
                let validate = |m: &MeasurementEvaluation| -> Result<(), String> {
                    if m.stamp != stamp {
                        return Err(
                            "Measurement value belongs to a different source snapshot".into()
                        );
                    }
                    if let Ok(v) = &m.result {
                        validate_values(v, &m.query)?;
                    }
                    Ok(())
                };
                if let Some(m) = &batch.transient {
                    validate(m)?;
                }
                if batch.pinned.len() != self.pins.len() {
                    return Err(
                        "Pinned measurement publication omits or adds source identities".into(),
                    );
                }
                for pin in &self.pins {
                    let m = batch
                        .pinned
                        .get(&pin.id)
                        .ok_or("Pinned measurement is missing from the native result")?;
                    if m.query != pin.query {
                        return Err(
                            "Pinned measurement result differs from canonical references".into(),
                        );
                    }
                    validate(m)?;
                }
                Ok(Arc::new(batch))
            });
            self.expected = None;
            match checked {
                Ok(batch) => {
                    self.batch = Some(batch);
                    self.error = None;
                }
                Err(error) => {
                    discard_transient(&mut self.batch);
                    self.error = Some(error);
                }
            }
        }
        let request = (stamp, query.cloned());
        if self.requested.as_ref() != Some(&request) {
            discard_transient(&mut self.batch);
            match backend.request(session.document(), stamp, query) {
                Ok(id) => {
                    self.expected = Some((id, stamp, query.cloned()));
                    self.requested = Some(request);
                    self.error = None;
                }
                Err(error) => {
                    self.expected = None;
                    self.error = Some(error);
                    self.requested = Some(request);
                }
            }
        }
    }
    pub fn suspend(&mut self) {
        if let Some(b) = self.backend.as_mut() {
            b.cancel();
        }
        self.backend = None;
        self.expected = None;
        self.requested = None;
        self.batch = None;
    }
    pub fn retry(&mut self) {
        self.requested = None;
    }
    pub fn current<'a>(
        &'a self,
        session: &Session,
        query: &MeasurementQuery,
    ) -> Option<&'a MeasurementEvaluation> {
        let stamp = self.stamp?;
        if stamp.document != session.document().id()
            || stamp.revision != session.revision()
            || self.pending()
        {
            return None;
        }
        self.batch
            .as_ref()?
            .transient
            .as_ref()
            .filter(|m| m.stamp == stamp && m.query == *query)
    }
}
// A new temporary pair can invalidate its own result while the pinned
// references still belong to the same checked source stamp.
fn discard_transient(batch: &mut Option<Arc<MeasurementBatch>>) {
    if let Some(accepted) = batch.as_mut() {
        if accepted.pinned.is_empty() {
            *batch = None;
        } else {
            Arc::make_mut(accepted).transient = None;
        }
    }
}
fn validate_values(v: &MeasurementValues, query: &MeasurementQuery) -> Result<(), String> {
    if !v.minimum_mm.is_finite()
        || v.minimum_mm < 0.
        || !v.tolerance_mm.is_finite()
        || !(1e-7..=1e-3).contains(&v.tolerance_mm)
        || !v.boolean_fuzzy_mm.is_finite()
        || !(0. ..=v.tolerance_mm).contains(&v.boolean_fuzzy_mm)
        || v.witnesses.is_empty()
        || v.witnesses.len() > 64
        || v.total_solutions < (v.witnesses.len() as u32)
        || v.witnesses_complete != (v.total_solutions as usize == v.witnesses.len())
        || v.key_a.kind != query.a.intent.kind
        || v.key_b.kind != query.b.intent.kind
    {
        return Err("Native measurement returned invalid units, limits or reference kinds".into());
    }
    for w in &v.witnesses {
        if w.a_mm.iter().chain(&w.b_mm).any(|x| !x.is_finite())
            || ((0..3)
                .map(|i| (w.b_mm[i] - w.a_mm[i]).powi(2))
                .sum::<f64>()
                .sqrt()
                - v.minimum_mm)
                .abs()
                > v.tolerance_mm * 4.
        {
            return Err("Measurement witnesses disagree with its scalar distance".into());
        }
    }
    if let Some(delta) = v.axis_components_mm
        && (delta.iter().any(|x| !x.is_finite())
            || (delta.iter().map(|x| x * x).sum::<f64>().sqrt() - v.minimum_mm).abs()
                > v.tolerance_mm * 4.
            || v.witnesses.iter().any(|w| {
                (0..3).any(|i| ((w.b_mm[i] - w.a_mm[i]) - delta[i]).abs() > v.tolerance_mm * 4.)
            }))
    {
        return Err("Measurement components disagree with its exact distance".into());
    }
    match &v.angle {
        ScalarMetric::Unsupported { .. } => {}
        ScalarMetric::Radians { value }
            if value.is_finite()
                && *value >= 0.
                && *value <= std::f64::consts::FRAC_PI_2 + 1e-10 => {}
        _ => return Err("Invalid measurement angle units or value".into()),
    }
    match &v.parallel_plane_separation {
        ScalarMetric::Unsupported { .. } => {}
        ScalarMetric::Millimeters { value } if value.is_finite() && *value >= 0. => {}
        _ => return Err("Invalid supporting-plane separation units or value".into()),
    }
    Ok(())
}
