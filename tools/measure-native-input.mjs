import {readFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';

const kinds = {1:'mouse-down',2:'mouse-up',3:'right-down',4:'right-up',5:'pointer-move',6:'pointer-drag',10:'key-down',11:'key-up',22:'scroll'};
const distribution = values => {
  if (!values.length) return null;
  const sorted = [...values].sort((a,b)=>a-b);
  return {count:sorted.length,p50:sorted[Math.ceil(sorted.length*.5)-1],p95:sorted[Math.ceil(sorted.length*.95)-1],max:sorted.at(-1)};
};
export function summarizeInputTrace(text, throughSequence = Infinity) {
  if (throughSequence !== Infinity && (!Number.isSafeInteger(throughSequence) || throughSequence < 1)) throw new Error('Invalid measurement cutoff');
  const rows = text.trim().split('\n').filter(Boolean).map((line,index)=>{
    try { return JSON.parse(line); } catch { throw new Error(`Invalid JSON at line ${index+1}`); }
  });
  const start = rows.find(row=>row.phase==='start');
  if (!start || !Number.isFinite(start.clock_delta_seconds) || Math.abs(start.clock_delta_seconds)>.001) throw new Error('Missing or inconsistent host clock validation');
  const inputs = new Map(), presented = new Map(), frames = new Map();
  let overflow = 0;
  for (const row of rows) {
    if (!['start','input','acquired','presented','frame-presented','overflow','exit'].includes(row.phase)) throw new Error('Unknown trace phase');
    if (row.phase==='input') {
      if (!Number.isSafeInteger(row.sequence) || row.sequence<1 || !kinds[row.kind] || !Number.isFinite(row.occurred) || !Number.isFinite(row.delivered) || row.occurred<0 || row.delivered<row.occurred || inputs.has(row.sequence)) throw new Error('Invalid or duplicate input');
      inputs.set(row.sequence,row);
    }
    if (row.phase==='presented') {
      if (presented.has(row.sequence) || !Number.isSafeInteger(row.frame) || row.frame<1 || !Number.isFinite(row.presented) || !Number.isFinite(row.acquired)) throw new Error('Invalid or duplicate presentation');
      presented.set(row.sequence,row);
    }
    if (row.phase==='frame-presented') {
      if (frames.has(row.frame) || !Number.isSafeInteger(row.frame) || row.frame<1 || !Number.isFinite(row.presented) || !Number.isFinite(row.acquired) || (row.presented!==0 && row.presented<row.acquired)) throw new Error('Invalid frame presentation');
      frames.set(row.frame,row);
    }
    if (row.phase==='overflow' && row.sequence<=throughSequence) overflow++;
  }
  for (const [sequence,row] of presented) {
    const input = inputs.get(sequence);
    if (!input || input.kind!==row.kind || input.occurred!==row.occurred || input.delivered!==row.delivered || row.acquired<row.delivered || (row.presented!==0 && row.presented<row.acquired)) throw new Error('Presentation does not match its input and host clock');
    const actual=frames.get(row.frame);
    if (start.version===3 && actual && (actual.acquired!==row.acquired || actual.presented!==row.presented)) throw new Error('Candidate does not match its drawable presentation');
  }
  const selected = [...inputs.values()].filter(row=>row.sequence<=throughSequence);
  if (!selected.length) throw new Error('No native input samples in measurement interval');
  const missing = [], dropped = [], recovered = [], delays = [], grouped = {};
  for (const input of selected) {
    let frame = presented.get(input.sequence);
    if (!frame) { missing.push(input.sequence); continue; }
    if (start.version===3 && !frames.has(frame.frame)) { missing.push(input.sequence); continue; }
    if (frame.presented===0) {
      const later=[...frames.values()].filter(value=>value.frame>=frame.frame && value.acquired>=frame.acquired && value.presented>0).sort((a,b)=>a.presented-b.presented)[0];
      if (!later) { dropped.push(input.sequence); continue; }
      recovered.push(input.sequence);
      frame=later;
    }
    const ms = (frame.presented-input.occurred)*1000;
    delays.push(ms);
    (grouped[kinds[input.kind]]??=[]).push(ms);
  }
  const overall = distribution(delays);
  return {scope:'native event occurrence to a subsequent Metal drawable presentation after input delivery; coalesced inputs share a frame',throughSequence:throughSequence===Infinity?null:throughSequence,inputCount:selected.length,excludedAfterSequence:inputs.size-selected.length,unpresentedSequences:missing,droppedSequences:dropped,recoveredDroppedCandidateSequences:recovered,overflow,overallMs:overall,byKindMs:Object.fromEntries(Object.entries(grouped).map(([kind,values])=>[kind,distribution(values)])),budgetPassed:!!overall && !missing.length && !dropped.length && !overflow && overall.p95<=100 && overall.max<=250};
}
if (process.argv[1] && import.meta.url===pathToFileURL(process.argv[1]).href) {
  const [file,cutoff] = process.argv.slice(2);
  if (!file) throw new Error('Usage: node tools/measure-native-input.mjs TRACE.jsonl [LAST_MEASURED_SEQUENCE]');
  console.log(JSON.stringify(summarizeInputTrace(readFileSync(file,'utf8'),cutoff===undefined?Infinity:Number(cutoff)),null,2));
}
