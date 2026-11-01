import assert from 'node:assert/strict';
import test from 'node:test';
import {summarizeInputTrace} from './measure-native-input.mjs';
const input = sequence=>({phase:'input',sequence,kind:1,occurred:sequence,delivered:sequence+.001});
const frame = (sequence,delay)=>({...input(sequence),phase:'presented',frame:sequence,acquired:sequence+.002,presented:sequence+delay});
const trace = (rows,version=2)=>[{phase:'start',version,clock_delta_seconds:0},...rows].map(row=>JSON.stringify(row)).join('\n');
test('native input quantiles include slow samples and enforce the feedback budget',()=>{
  const report=summarizeInputTrace(trace([input(1),frame(1,.05),input(2),frame(2,.3)]));
  assert.equal(report.inputCount,2);
  assert.ok(Math.abs(report.overallMs.max-300)<1e-6);
  assert.equal(report.overallMs.p95,report.overallMs.max);
  assert.equal(report.budgetPassed,false);
});
test('unpresented inputs cannot pass and a documented cutoff excludes only later terminal actions',()=>{
  const text=trace([input(1),frame(1,.05),input(2)]);
  assert.deepEqual(summarizeInputTrace(text).unpresentedSequences,[2]);
  assert.equal(summarizeInputTrace(text).budgetPassed,false);
  const report=summarizeInputTrace(text,1);
  assert.equal(report.budgetPassed,true);
  assert.equal(report.excludedAfterSequence,1);
});
test('corrupt asynchronous records and mismatched clocks are rejected',()=>{
  assert.throws(()=>summarizeInputTrace('{"phase":"input","delivered":nan}'),/Invalid JSON/);
  assert.throws(()=>summarizeInputTrace(trace([input(1),{...frame(1,.05),occurred:2}])) ,/does not match/);
  assert.throws(()=>summarizeInputTrace(trace([input(1),input(1)])),/duplicate input/);
  assert.throws(()=>summarizeInputTrace('{"phase":"start","clock_delta_seconds":1}'),/clock/);
});
test('a dropped candidate is measured through its next actual displayed frame',()=>{
  const text=trace([input(1),{...frame(1,.05),presented:0},{phase:'frame-presented',frame:1,acquired:1.002,presented:0},{phase:'frame-presented',frame:2,acquired:1.04,presented:1.08}],3);
  const report=summarizeInputTrace(text);
  assert.deepEqual(report.recoveredDroppedCandidateSequences,[1]);
  assert.ok(Math.abs(report.overallMs.max-80)<1e-6);
  assert.equal(report.budgetPassed,true);
});

test('v3 candidate records must match the drawable and cannot omit its presentation',()=>{
  const candidate=frame(1,.05);
  assert.deepEqual(summarizeInputTrace(trace([input(1),candidate],3)).unpresentedSequences,[1]);
  assert.throws(()=>summarizeInputTrace(trace([input(1),candidate,{phase:'frame-presented',frame:1,acquired:1.002,presented:1.06}],3)),/does not match its drawable/);
});
