//! Source-bound, read-only assembly inspection receipts. These DTOs contain no
//! meshes/native handles and never assert safety between explicitly sampled poses.
use crate::{
    document::{Document, EntityId},
    model::{JointCoordinate, OccurrencePath},
    project_file::ContentHash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const MAX_INSPECTION_PAIRS: usize = 256;
pub const MAX_RANGE_SAMPLES: usize = 256;
pub const MAX_RANGE_PAIR_CHECKS: usize = 4096;
pub const MAX_CLEARANCE_WITNESSES: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
}
impl InspectionStamp {
    pub fn capture(document: &Document, revision: u64) -> Result<Self, String> {
        Ok(Self {
            document: document.id(),
            revision,
            source_hash: ContentHash::document(document)?,
        })
    }
    pub fn check(&self, document: &Document, revision: u64) -> Result<(), String> {
        if *self != Self::capture(document, revision)? {
            Err("Assembly inspection belongs to stale source".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrencePair {
    pub a: OccurrencePath,
    pub b: OccurrencePath,
}
impl OccurrencePair {
    pub fn validate(&self) -> Result<(), String> {
        for path in [&self.a, &self.b] {
            if path.root.is_nil()
                || path.members.len() > 32
                || path.members.iter().any(|id| id.is_nil())
            {
                return Err(
                    "Occurrence paths need non-nil identities and at most 32 explicit members"
                        .into(),
                );
            }
        }
        if self.a == self.b {
            return Err("Choose two distinct physical occurrence paths".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairQuery {
    pub expected: InspectionStamp,
    pub pairs: Vec<OccurrencePair>,
    pub contact_tolerance_mm: f64,
}
impl PairQuery {
    pub fn validate(&self) -> Result<(), String> {
        if self.expected.document.is_nil()
            || self.pairs.is_empty()
            || self.pairs.len() > MAX_INSPECTION_PAIRS
            || !self.contact_tolerance_mm.is_finite()
            || !(1e-7..=1e-3).contains(&self.contact_tolerance_mm)
        {
            return Err("Inspection needs current source, 1..256 explicit pairs and contact tolerance 0.0000001..0.001 mm".into());
        }
        let mut seen = BTreeSet::new();
        for pair in &self.pairs {
            pair.validate()?;
            let key = if pair.a < pair.b {
                (&pair.a, &pair.b)
            } else {
                (&pair.b, &pair.a)
            };
            if !seen.insert(key) {
                return Err("Inspection contains a duplicate occurrence pair".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairRelation {
    Separated,
    ContactWithinTolerance,
    Overlapping,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionWitness {
    pub a_mm: [f64; 3],
    pub b_mm: [f64; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairValues {
    pub relation: PairRelation,
    pub overlap_volume_mm3: f64,
    pub minimum_clearance_mm: f64,
    pub witnesses: Vec<InspectionWitness>,
    pub witnesses_complete: bool,
    pub total_solutions: u32,
    pub inner_solution: bool,
    pub tolerance_mm: f64,
    pub boolean_fuzzy_mm: f64,
}
impl PairValues {
    pub fn validate(&self, query: &PairQuery) -> Result<(), String> {
        if !self.overlap_volume_mm3.is_finite()
            || self.overlap_volume_mm3 < 0.
            || !self.minimum_clearance_mm.is_finite()
            || self.minimum_clearance_mm < 0.
            || self.tolerance_mm != query.contact_tolerance_mm
            || self.boolean_fuzzy_mm != 0.
            || self.witnesses.is_empty()
            || self.witnesses.len() > MAX_CLEARANCE_WITNESSES
            || (self.total_solutions as usize) < self.witnesses.len()
            || self.witnesses_complete != (self.total_solutions as usize == self.witnesses.len())
        {
            return Err("Native pair result contains invalid values or witness metadata".into());
        }
        let consistent = match self.relation {
            PairRelation::Overlapping => {
                self.overlap_volume_mm3 > 0. && self.minimum_clearance_mm <= self.tolerance_mm
            }
            PairRelation::ContactWithinTolerance => {
                self.overlap_volume_mm3 == 0. && self.minimum_clearance_mm <= self.tolerance_mm
            }
            PairRelation::Separated => {
                self.overlap_volume_mm3 == 0. && self.minimum_clearance_mm > self.tolerance_mm
            }
        };
        if !consistent
            || self.witnesses.iter().any(|w| {
                w.a_mm.iter().chain(&w.b_mm).any(|v| !v.is_finite())
                    || ((0..3)
                        .map(|i| (w.a_mm[i] - w.b_mm[i]).powi(2))
                        .sum::<f64>()
                        .sqrt()
                        - self.minimum_clearance_mm)
                        .abs()
                        > self.tolerance_mm * 4.
            })
        {
            return Err("Pair relation or witnesses disagree with exact native metrics".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionFailureKind {
    Missing,
    Stale,
    Invalid,
    Unsupported,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionFailure {
    pub kind: InspectionFailureKind,
    pub message: String,
}
impl std::fmt::Display for InspectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for InspectionFailure {}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairEvaluation {
    pub pair: OccurrencePair,
    pub result: Result<PairValues, InspectionFailure>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairReport {
    pub stamp: InspectionStamp,
    pub query: PairQuery,
    pub pairs: Vec<PairEvaluation>,
}
impl PairReport {
    pub fn validate_against(&self, query: &PairQuery) -> Result<(), String> {
        query.validate()?;
        if self.query != *query
            || self.stamp != query.expected
            || self.pairs.len() != query.pairs.len()
        {
            return Err(
                "Pair report belongs to another query/source or omits requested pairs".into(),
            );
        }
        for (value, pair) in self.pairs.iter().zip(&query.pairs) {
            if value.pair != *pair {
                return Err("Pair report identities/order differ from the request".into());
            }
            match &value.result {
                Ok(values) => values.validate(query)?,
                Err(error) if error.message.trim().is_empty() => {
                    return Err("Pair failure needs an explicit diagnostic".into());
                }
                Err(_) => {}
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionRangeQuery {
    pub expected: InspectionStamp,
    pub pairs: Vec<OccurrencePair>,
    pub contact_tolerance_mm: f64,
    pub joint: EntityId,
    pub coordinate: JointCoordinate,
    pub start: f64,
    pub end: f64,
    pub step: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleUnit {
    Millimeters,
    Radians,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeCoverage {
    SamplesOnly,
}
impl MotionRangeQuery {
    pub fn pair_query(&self) -> PairQuery {
        PairQuery {
            expected: self.expected,
            pairs: self.pairs.clone(),
            contact_tolerance_mm: self.contact_tolerance_mm,
        }
    }
    pub fn unit(&self) -> SampleUnit {
        if self.coordinate == JointCoordinate::Rotation {
            SampleUnit::Radians
        } else {
            SampleUnit::Millimeters
        }
    }
    pub fn sample_values(&self) -> Result<Vec<f64>, String> {
        if self.joint.is_nil()
            || [self.start, self.end, self.step]
                .iter()
                .any(|v| !v.is_finite())
            || self.step == 0.
        {
            return Err(
                "Range needs a joint and finite start/end/nonzero step in mm or radians".into(),
            );
        }
        if self.start == self.end {
            return Ok(vec![self.start]);
        }
        let ratio = (self.end - self.start) / self.step;
        if !ratio.is_finite() || ratio < 0. || ratio.floor() >= MAX_RANGE_SAMPLES as f64 {
            return Err("Range step direction/count exceeds the 256-sample budget".into());
        }
        let mut values = vec![];
        for i in 0..=ratio.floor() as usize {
            let value = self.start + i as f64 * self.step;
            if !value.is_finite() {
                return Err("Range arithmetic overflowed".into());
            }
            if (self.step > 0. && value > self.end) || (self.step < 0. && value < self.end) {
                break;
            }
            if values.last() == Some(&value) {
                return Err("Range step is too small for these coordinates".into());
            }
            values.push(value);
        }
        if values.last() != Some(&self.end) {
            values.push(self.end);
        }
        if values.len() > MAX_RANGE_SAMPLES {
            return Err("Range endpoint exceeds the 256-sample budget".into());
        }
        Ok(values)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.pair_query().validate()?;
        let count = self.sample_values()?.len();
        if count * self.pairs.len() > MAX_RANGE_PAIR_CHECKS {
            return Err("Range exceeds the 4096 native pair-check budget".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeSample {
    pub value: f64,
    pub candidate_stamp: Option<InspectionStamp>,
    pub result: Result<PairReport, InspectionFailure>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionRangeReport {
    pub stamp: InspectionStamp,
    pub query: MotionRangeQuery,
    pub unit: SampleUnit,
    pub coverage: RangeCoverage,
    pub sample_values: Vec<f64>,
    pub actual_intervals: Vec<f64>,
    pub samples: Vec<RangeSample>,
}
impl MotionRangeReport {
    pub fn validate_against(&self, query: &MotionRangeQuery) -> Result<(), String> {
        query.validate()?;
        let values = query.sample_values()?;
        let intervals: Vec<_> = values.windows(2).map(|p| p[1] - p[0]).collect();
        if self.stamp != query.expected
            || self.query != *query
            || self.unit != query.unit()
            || self.sample_values != values
            || self.actual_intervals != intervals
            || self.samples.len() != values.len()
        {
            return Err(
                "Range report differs from source, units, requested samples or actual steps".into(),
            );
        }
        for (sample, value) in self.samples.iter().zip(values) {
            if sample.value != value {
                return Err("Range sample value differs from its explicit plan".into());
            }
            if sample.candidate_stamp.is_some_and(|s| {
                s.document != self.stamp.document || s.revision != self.stamp.revision
            }) {
                return Err("Range candidate belongs to a foreign source".into());
            }
            match &sample.result {
                Ok(report) => {
                    let stamp = sample
                        .candidate_stamp
                        .ok_or("Successful range sample needs its actual candidate stamp")?;
                    let mut pair = query.pair_query();
                    pair.expected = stamp;
                    report.validate_against(&pair)?;
                }
                Err(error) if error.message.trim().is_empty() => {
                    return Err("Failed range sample needs an explicit diagnostic".into());
                }
                Err(_) => {}
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "query",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AssemblyInspectionQuery {
    Current(PairQuery),
    Range(MotionRangeQuery),
}
impl AssemblyInspectionQuery {
    pub fn stamp(&self) -> InspectionStamp {
        match self {
            Self::Current(query) => query.expected,
            Self::Range(query) => query.expected,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Current(query) => query.validate(),
            Self::Range(query) => query.validate(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "report",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AssemblyInspectionReport {
    Current(PairReport),
    Range(MotionRangeReport),
}
impl AssemblyInspectionReport {
    pub fn validate_against(&self, query: &AssemblyInspectionQuery) -> Result<(), String> {
        match (self, query) {
            (Self::Current(report), AssemblyInspectionQuery::Current(query)) => {
                report.validate_against(query)
            }
            (Self::Range(report), AssemblyInspectionQuery::Range(query)) => {
                report.validate_against(query)
            }
            _ => Err("Assembly inspection reply has a different query kind".into()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyInspectionReply {
    pub request: u64,
    pub stamp: InspectionStamp,
    pub query: AssemblyInspectionQuery,
    pub result: Result<AssemblyInspectionReport, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query() -> MotionRangeQuery {
        let document = Document::new("Sample plan").unwrap();
        MotionRangeQuery {
            expected: InspectionStamp::capture(&document, 0).unwrap(),
            pairs: vec![OccurrencePair {
                a: OccurrencePath {
                    root: EntityId::new(),
                    members: vec![],
                },
                b: OccurrencePath {
                    root: EntityId::new(),
                    members: vec![],
                },
            }],
            contact_tolerance_mm: 1e-7,
            joint: EntityId::new(),
            coordinate: JointCoordinate::Z,
            start: 0.,
            end: 1.,
            step: 0.3,
        }
    }
    #[test]
    fn range_retains_explicit_endpoint_and_actual_nonuniform_final_step() {
        let mut q = query();
        let values = q.sample_values().unwrap();
        assert_eq!(values, vec![0., 0.3, 0.6, 0.8999999999999999, 1.]);
        q.start = 1.;
        q.end = 0.;
        q.step = -0.3;
        assert_eq!(q.sample_values().unwrap().last(), Some(&0.));
        q.step = 0.3;
        assert!(q.validate().is_err());
        q.start = 0.;
        q.end = 256.;
        q.step = 1.;
        assert!(q.validate().is_err());
    }
    #[test]
    fn pairs_reject_self_symmetric_duplicates_nonfinite_tolerance_and_foreign_stamp() {
        let q = query();
        let mut p = q.pair_query();
        let pair = p.pairs[0].clone();
        p.pairs.push(OccurrencePair {
            a: pair.b.clone(),
            b: pair.a.clone(),
        });
        assert!(p.validate().is_err());
        p.pairs = vec![OccurrencePair {
            a: pair.a.clone(),
            b: pair.a,
        }];
        assert!(p.validate().is_err());
        p = q.pair_query();
        p.contact_tolerance_mm = f64::NAN;
        assert!(p.validate().is_err());
        assert!(
            q.expected
                .check(&Document::new("Other source").unwrap(), 0)
                .is_err()
        );
    }
    #[test]
    fn sample_budget_and_units_are_explicit() {
        let mut q = query();
        q.coordinate = JointCoordinate::Rotation;
        assert_eq!(q.unit(), SampleUnit::Radians);
        q.end = 100.;
        q.step = 1.;
        let pair = q.pairs[0].clone();
        for _ in 0..41 {
            let mut next = pair.clone();
            next.b.root = EntityId::new();
            q.pairs.push(next);
        }
        assert!(q.validate().is_err());
    }
    #[test]
    fn report_validation_rejects_missing_pairs_foreign_samples_and_fabricated_metrics() {
        let q = query();
        let p = q.pair_query();
        let values = PairValues {
            relation: PairRelation::Separated,
            overlap_volume_mm3: 0.,
            minimum_clearance_mm: 1.,
            witnesses: vec![InspectionWitness {
                a_mm: [0.; 3],
                b_mm: [1., 0., 0.],
            }],
            witnesses_complete: true,
            total_solutions: 1,
            inner_solution: false,
            tolerance_mm: p.contact_tolerance_mm,
            boolean_fuzzy_mm: 0.,
        };
        let report = PairReport {
            stamp: p.expected,
            query: p.clone(),
            pairs: vec![PairEvaluation {
                pair: p.pairs[0].clone(),
                result: Ok(values.clone()),
            }],
        };
        report.validate_against(&p).unwrap();
        let mut bad = report.clone();
        bad.pairs.clear();
        assert!(bad.validate_against(&p).is_err());
        let mut bad = report.clone();
        bad.pairs[0].pair.a.root = EntityId::new();
        assert!(bad.validate_against(&p).is_err());
        let mut bad = values.clone();
        bad.overlap_volume_mm3 = 1.;
        assert!(bad.validate(&p).is_err());
        let mut bad = values.clone();
        bad.witnesses[0].b_mm[0] = 2.;
        assert!(bad.validate(&p).is_err());
        let mut bad = values;
        bad.minimum_clearance_mm = f64::NAN;
        assert!(bad.validate(&p).is_err());
        let values = q.sample_values().unwrap();
        let mut range = MotionRangeReport {
            stamp: q.expected,
            query: q.clone(),
            unit: q.unit(),
            coverage: RangeCoverage::SamplesOnly,
            actual_intervals: values.windows(2).map(|p| p[1] - p[0]).collect(),
            sample_values: values.clone(),
            samples: values
                .iter()
                .map(|v| RangeSample {
                    value: *v,
                    candidate_stamp: Some(q.expected),
                    result: Ok(report.clone()),
                })
                .collect(),
        };
        range.validate_against(&q).unwrap();
        range.samples[0].candidate_stamp.as_mut().unwrap().document = EntityId::new();
        assert!(range.validate_against(&q).is_err());
        range.samples[0].candidate_stamp = Some(q.expected);
        range.actual_intervals[0] = 1.;
        assert!(range.validate_against(&q).is_err());
    }
}
