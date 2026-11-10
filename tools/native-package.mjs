// Native runtime layout, source/notice closure and packaged smoke checks.
import {execFileSync} from 'node:child_process';
import {cpSync,copyFileSync,existsSync,lstatSync,mkdirSync,readFileSync,readdirSync,realpathSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve,relative,sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const run=(command,args,options={})=>execFileSync(command,args,{cwd:root,encoding:'utf8',stdio:['ignore','pipe','pipe'],...options})?.trim()??'';
const escapeXML=value=>value.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
export const rustCandidateBuildArgs=Object.freeze(['-p','multicad-app','--release','--locked','--no-default-features']);
export function nativeSourceHash(source){
  const paths=[];walk(source,path=>paths.push(path));
  // Rust Path ordering compares components (a/child precedes a.cpp), not the
  // flattened path string. Keep source identity identical to the build helper.
  paths.sort((a,b)=>{const aa=a.split(sep),bb=b.split(sep);for(let i=0;i<Math.min(aa.length,bb.length);i++){const order=Buffer.compare(Buffer.from(aa[i]),Buffer.from(bb[i]));if(order)return order;}return aa.length-bb.length;});
  const hash=createHash('sha256');
  for(const path of paths){hash.update(relative(source,path).split(sep).join('/'));hash.update(Buffer.from([0]));hash.update(createHash('sha256').update(readFileSync(path)).digest());}
  return hash.digest('hex');
}

// Native project files: .tcd for new projects; .multicad stays openable. Both
// name the same container, so both document types open in ToskCAD as Editor.
export const PROJECT_TYPES=[
  {uti:'de.tokenet.toskcad.project',mime:'application/x-toskcad-project',extension:'tcd',name:'ToskCAD Project'},
  {uti:'de.tokenet.multicad.project',mime:'application/x-multicad-project',extension:'multicad',name:'ToskCAD Project (legacy .multicad)'},
];
function macOSDocumentTypes(){
  const strings=values=>`<array>${values.map(value=>`<string>${escapeXML(value)}</string>`).join('')}</array>`;
  const documents=PROJECT_TYPES.map(type=>`<dict><key>CFBundleTypeName</key><string>${escapeXML(type.name)}</string><key>CFBundleTypeRole</key><string>Editor</string><key>LSHandlerRank</key><string>Owner</string><key>CFBundleTypeIconFile</key><string>AppIcon.icns</string><key>LSItemContentTypes</key>${strings([type.uti])}</dict>`).join('');
  const exported=PROJECT_TYPES.map(type=>`<dict><key>UTTypeIdentifier</key><string>${type.uti}</string><key>UTTypeDescription</key><string>${escapeXML(type.name)}</string><key>UTTypeConformsTo</key>${strings(['public.data','public.content'])}<key>UTTypeIconFile</key><string>AppIcon.icns</string><key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key>${strings([type.extension])}<key>public.mime-type</key>${strings([type.mime])}</dict></dict>`).join('');
  return `<key>CFBundleDocumentTypes</key><array>${documents}</array><key>UTExportedTypeDeclarations</key><array>${exported}</array>`;
}
// Bundled example projects (CAD-186): examples/examples.json lists every
// top-level .tcd demo and the further files each requires. Packaging copies
// exactly that closure into Resources/examples (macOS) or resources/examples
// (Windows/Linux) after checking each native container's envelope.
export const EXAMPLES_FORMAT='toskcad.examples';
const DEVELOPER_PATH=/(?:\/(?:Users|home)\/[^"\/\\]+\/|[A-Za-z]:\\\\Users\\\\|\/private\/var\/folders\/)/;
const relativeExamplePath=value=>typeof value==='string'&&value.length>0&&!value.includes('\\')&&!value.startsWith('/')&&value.split('/').every(part=>part&&part!=='.'&&part!=='..');
const hashOf=bytes=>createHash('sha256').update(bytes).digest();
const sameHash=(digest,value)=>Array.isArray(value)&&value.length===32&&Buffer.from(value).equals(digest);
export function checkProjectContainer(path){
  const name=path.split(/[\\/]/).at(-1);
  const fail=reason=>{throw new Error(`Example ${name}: ${reason}. Regenerate it with its generator (docs/architecture/demo-projects.md).`);};
  const bytes=readFileSync(path);
  if(bytes.length<52||!bytes.subarray(0,8).equals(Buffer.from('MCADPRJ\0','latin1')))fail('not a native ToskCAD project');
  const version=bytes.readUInt32LE(8);if(version!==1&&version!==2)fail(`unsupported format version ${version}`);
  const length=Number(bytes.readBigUInt64LE(12));if(length>4*1024*1024||52+length>bytes.length)fail('manifest length exceeds the file');
  const manifestBytes=bytes.subarray(52,52+length);
  if(!hashOf(manifestBytes).equals(bytes.subarray(20,52)))fail('manifest checksum mismatch (damaged file)');
  let manifest;try{manifest=JSON.parse(manifestBytes.toString('utf8'));}catch{fail('manifest is not JSON');}
  if(typeof manifest.identity?.name!=='string'||!manifest.identity.name)fail('missing project identity');
  if(manifest.recovery)fail('is a recovery snapshot, not a project');
  let offset=52+length,source;
  if(version===2){
    const chunk=manifest.source;if(!chunk||!Number.isSafeInteger(chunk.length))fail('missing canonical source record');
    source=bytes.subarray(offset,offset+chunk.length);if(source.length!==chunk.length)fail('canonical source is truncated');
    if(!sameHash(hashOf(source),chunk.sha256))fail('canonical source checksum mismatch (damaged file)');
    offset+=chunk.length;
    for(const record of [...(manifest.payloads??[]),...(manifest.caches??[])]){
      const part=bytes.subarray(offset,offset+record.chunk.length);
      if(part.length!==record.chunk.length||!sameHash(hashOf(part),record.chunk.sha256))fail('embedded payload is damaged or truncated');
      offset+=record.chunk.length;
    }
    if(offset!==bytes.length)fail('unexpected trailing data');
  }else source=Buffer.from(JSON.stringify(manifest.document??null));
  const text=source.toString('utf8');
  try{JSON.parse(text);}catch{fail('canonical source is not JSON');}
  const developer=DEVELOPER_PATH.exec(text)||DEVELOPER_PATH.exec(manifestBytes.toString('utf8'));
  if(developer)fail(`contains the absolute developer path "${developer[0]}"`);
  return {name:manifest.identity.name,version,document:manifest.identity.id};
}
export function validateExamples(directory=join(root,'examples')){
  const manifestPath=join(directory,'examples.json');
  if(!existsSync(manifestPath))throw new Error(`Missing ${manifestPath}: list every bundled example there (docs/architecture/demo-projects.md)`);
  let manifest;try{manifest=JSON.parse(readFileSync(manifestPath,'utf8'));}catch(error){throw new Error(`examples/examples.json is not JSON: ${error.message}`);}
  if(manifest?.format!==EXAMPLES_FORMAT||manifest.version!==1||!Array.isArray(manifest.examples))throw new Error(`examples/examples.json must have format "${EXAMPLES_FORMAT}", version 1 and an examples array`);
  for(const key of Object.keys(manifest))if(!['format','version','examples'].includes(key))throw new Error(`examples/examples.json: unknown field "${key}"`);
  const listed=[],files=new Set(['examples.json']);
  for(const entry of manifest.examples){
    for(const key of Object.keys(entry??{}))if(!['file','requires'].includes(key))throw new Error(`examples/examples.json: unknown example field "${key}"`);
    const file=entry?.file;
    if(!relativeExamplePath(file)||file.includes('/')||!/\.tcd$/i.test(file))throw new Error(`examples/examples.json: "${file}" must be a .tcd file at the top of examples/`);
    if(listed.includes(file))throw new Error(`examples/examples.json lists ${file} twice`);
    if(!existsSync(join(directory,file)))throw new Error(`Bundled example ${file} is listed in examples/examples.json but missing`);
    listed.push(file);files.add(file);
    for(const required of entry.requires??[]){
      if(!relativeExamplePath(required))throw new Error(`examples/examples.json: ${file} requires "${required}", which is not a relative path inside examples/`);
      if(!existsSync(join(directory,required)))throw new Error(`Bundled example ${file} requires ${required}, which is missing from examples/`);
      files.add(required);
    }
  }
  const unlisted=readdirSync(directory).filter(name=>/\.tcd$/i.test(name)&&!listed.includes(name));
  if(unlisted.length)throw new Error(`Add ${unlisted.join(', ')} to examples/examples.json; every bundled .tcd must be listed`);
  if(!listed.length)throw new Error('examples/examples.json lists no example projects');
  const projects=[...files].filter(file=>/\.tcd$/i.test(file)).map(file=>({file,...checkProjectContainer(join(directory,file))}));
  return {examples:listed,files:[...files].sort(),projects};
}
export function bundleExamples(resources,directory=join(root,'examples')){
  const checked=validateExamples(directory);
  const target=join(resources,'examples');mkdirSync(target,{recursive:true});
  for(const file of checked.files){const destination=join(target,...file.split('/'));mkdirSync(dirname(destination),{recursive:true});copyFileSync(join(directory,...file.split('/')),destination);}
  return checked;
}
// Wood catalog facts are independently checked here; no imported PDF/art is needed.
function validateWoodCatalog(inside,files,projects){
  const wood=JSON.parse(readFileSync(inside('wood-screws-curation.json'),'utf8'));
  const ledger=JSON.parse(readFileSync(inside('sources/wood-screws/source-ledger.json'),'utf8'));
  const only=(v,keys)=>v!=null&&typeof v==='object'&&!Array.isArray(v)&&Object.keys(v).every(k=>keys.includes(k));
  const stable=v=>JSON.stringify(v,(_key,value)=>value&&typeof value==='object'&&!Array.isArray(value)?Object.fromEntries(Object.entries(value).sort(([a],[b])=>a.localeCompare(b))):value);
  if(!only(wood,['format','version','revision','verified_on','scope','representation','modeling_targets','sources','entries','coverage','excluded_families'])||wood.format!=='toskcad.wood-screw-curation'||wood.version!==1||wood.revision!=='1'||!wood.verified_on||!wood.scope||wood.representation!=='simplified_envelope'||!Array.isArray(wood.entries)||wood.entries.length!==44||!Array.isArray(wood.coverage)||wood.coverage.length!==12||!Array.isArray(wood.sources)||wood.sources.length!==3)throw new Error('Invalid bundled wood screw curation');
  if(!only(wood.modeling_targets,['head_height','across_flats','underhead_length','stock_diameter','distal_end','point','manufacturing','thread_extent'])||wood.modeling_targets.manufacturing!=='unavailable'||wood.modeling_targets.point!=='unspecified_manufacturer_end'||Object.values(wood.modeling_targets).some(v=>typeof v!=='string'||!v))throw new Error('Invalid bundled wood envelope representation');
  const sourceHashes=new Map([
    ['bitork-din571-undated','8012551843685e2fe5a4514650ed4c411137db05f176e627192c24c9796ee6bf'],
    ['placencia-din7998-undated','c4e1d550a264484f044c6c6675a2a50c636f61a198eda3ac463ba62a8b0f821f'],
    ['westfield-din571-undated','b984e632bd5016745bf60d78e5c07b3771f408a920d72397250a4662834be610'],
  ]),sourceIds=new Set();
  const sourceUrls=new Map([['bitork-din571-undated','https://www.bitorkfasteners.com/_files/ugd/179fe5_75017d246622499e8e22164ae1b4ed59.pdf'],['placencia-din7998-undated','https://www.tornilleriaplacencia.com/temas/bootstrap/uploads/archivos/Apartado_Tecnico.pdf'],['westfield-din571-undated','https://www.westfieldfasteners.co.uk/Standards/Woodscrew-CoachScr.pdf']]);
  for(const s of wood.sources){if(!only(s,['id','url','sha256','reference','role','rights'])||sourceIds.has(s.id)||sourceHashes.get(s.id)!==s.sha256||sourceUrls.get(s.id)!==s.url||![s.reference,s.role,s.rights].every(x=>typeof x==='string'&&x))throw new Error('Invalid bundled wood source reference');sourceIds.add(s.id);}
  if(!only(ledger,['format','version','verified_on','sources','excluded_source','edition_records'])||ledger.format!=='toskcad.wood-screw-source-ledger'||ledger.version!==1||stable(ledger.sources)!==stable(wood.sources))throw new Error('Bundled wood source ledger differs from curation');
  const specs=new Map([
    [4,{lengths:[16,20,25,30,35,40],head:[2.8,2.5,3.1,6.64,7,7.50],profile:[1.8,2.8]}],
    [5,{lengths:[16,20,25,30,35,40,45,50],head:[3.5,3.13,3.88,7.64,8,8.63],profile:[2.2,3.5]}],
    [6,{lengths:[20,25,30,35,40,45,50,55,60],head:[4,3.63,4.38,9.64,10,10.89],profile:[2.6,4.2]}],
    [8,{lengths:[25,30,35,40,45,50,55,60,70,80,100],head:[5.5,5.13,5.88,12.57,13,14.20],profile:[3.6,5.6]}],
    [10,{lengths:[30,35,40,45,50,55,60,70,80,100],head:[7,6.55,7.45,16.57,17,18.72],profile:[4.5,7]}],
  ]),ids=new Set(),woodFiles=new Set(),cells=new Set();
  const headKeys=['height_nominal_mm','height_min_mm','height_max_mm','across_flats_min_mm','across_flats_max_mm','diagonal_reference_mm'];
  const lengthLimits=new Map([[16,[15.1,16.9]],[20,[18.95,21.05]],[25,[23.95,26.05]],[30,[28.95,31.05]],[35,[33.75,36.25]],[40,[38.75,41.25]],[45,[43.75,46.25]],[50,[48.75,51.25]],[55,[53.5,56.5]],[60,[58.5,61.5]],[70,[68.5,71.5]],[80,[78.5,81.5]],[100,[98.25,101.75]]]);
  for(const e of wood.entries){
    const spec=specs.get(e.nominal_mm),slug=`d${e.nominal_mm}-l${e.underhead_length_mm}`,cell=JSON.stringify([e.nominal_mm,e.underhead_length_mm]);
    if(!only(e,['id','file','designation','family','nominal_mm','underhead_length_mm','length_min_mm','length_max_mm','head','v_reference','minimum_thread_extent_ratio','source_weight_g','dimensional_source','profile_reference_source','minimum_extent_source','correction'])||!spec?.lengths.includes(e.underhead_length_mm)||cells.has(cell)||e.id!==`wood-screw.bitork-din571.${slug}`||e.file!==`wood-screw-bitork-din571-${slug}.tcd`||e.designation!==`${e.nominal_mm} × ${e.underhead_length_mm}`||e.family!=='bitork-din571'||ids.has(e.id)||woodFiles.has(e.file)||!files.has(e.file)||e.correction!==null||e.minimum_thread_extent_ratio!==0.6||e.dimensional_source!=='bitork-din571-undated'||e.profile_reference_source!=='placencia-din7998-undated'||e.minimum_extent_source!=='westfield-din571-undated'||!Number.isFinite(e.source_weight_g)||e.source_weight_g<=0||e.length_min_mm!==lengthLimits.get(e.underhead_length_mm)?.[0]||e.length_max_mm!==lengthLimits.get(e.underhead_length_mm)?.[1]||!only(e.head,headKeys)||headKeys.some((key,i)=>e.head[key]!==spec.head[i])||!only(e.v_reference,['pitch_mm','core_diameter_mm','included_angle_deg','included_angle_tolerance_deg'])||e.v_reference.pitch_mm!==spec.profile[0]||e.v_reference.core_diameter_mm!==spec.profile[1]||e.v_reference.included_angle_deg!==60||e.v_reference.included_angle_tolerance_deg!==6)throw new Error('Invalid bundled wood project/source closure');
    const key=(BigInt(e.nominal_mm*1000)*1000000n+BigInt(e.underhead_length_mm*1000))*100n;
    const identity=`cad23700-0000-4000-8000-${key.toString(16).padStart(12,'0')}`;
    if(projects.find(p=>p.file===e.file)?.document!==identity)throw new Error('Bundled wood project identity differs from curation');
    const raw=readFileSync(inside(e.file)),n=Number(raw.readBigUInt64LE(12)),manifest=JSON.parse(raw.subarray(52,52+n)),doc=JSON.parse(raw.subarray(52+n,52+n+manifest.source.length));
    const mechanical=Object.values(doc.entities).filter(entity=>entity.data?.definition?.type_id==='model.mechanical_part');
    if(!manifest.required_capabilities?.includes('model.mechanical_part.wood_screw.1')||mechanical.length!==1||mechanical[0].data.definition.data.parameters.kind!=='wood_screw'||mechanical[0].data.definition.data.parameters.representation!=='simplified_envelope'||mechanical[0].data.definition.data.provenance?.source_url!==sourceUrls.get(e.dimensional_source)||mechanical[0].data.definition.data.provenance?.catalog_id!==`toskcad.sourced.${e.id}`||mechanical[0].data.definition.data.provenance?.reference_geometry.kind!=='wood_screw'||mechanical[0].data.definition.data.provenance.reference_geometry.representation!=='simplified_envelope')throw new Error('Bundled wood project representation/capability differs from curation');
    ids.add(e.id);woodFiles.add(e.file);cells.add(cell);
  }
  const nominals=new Set([2,2.5,3,3.5,4,4.5,5,6,7,8,9,10]),covered=new Set();
  for(const c of wood.coverage){
    const expected=wood.entries.filter(e=>e.nominal_mm===c.nominal_mm).map(e=>e.id).sort();
    if(!only(c,['family','nominal_mm','status','entry_ids','reason'])||c.family!=='bitork-din571'||!nominals.delete(c.nominal_mm)||!Array.isArray(c.entry_ids)||stable([...c.entry_ids].sort())!==stable(expected)||(expected.length?(c.status!=='supported'||c.reason!==null):(c.status!=='unavailable'||typeof c.reason!=='string'||!c.reason)))throw new Error('Invalid bundled wood coverage');
    for(const id of c.entry_ids){if(covered.has(id))throw new Error('Duplicate bundled wood coverage');covered.add(id);}
  }
  if(nominals.size||covered.size!==44)throw new Error('Bundled wood coverage omits a project');
  const excluded=new Set(['fischer-powerfast-ii-ctf-bc','phillips-h-wood-screw']);
  if(!Array.isArray(wood.excluded_families)||wood.excluded_families.length!==2)throw new Error('Invalid bundled wood exclusions');
  for(const f of wood.excluded_families)if(!only(f,['family','status','reason'])||!excluded.delete(f.family)||f.status!=='unavailable'||typeof f.reason!=='string'||!f.reason)throw new Error('Invalid bundled wood exclusions');
}
export function validateBundledLibrary(directory=join(root,'library')){
  const base=realpathSync(directory),manifest=JSON.parse(readFileSync(join(directory,'library.json'),'utf8'));
  const only=(value,keys)=>Object.keys(value??{}).every(key=>keys.includes(key));
  if(!only(manifest,['format','version','revision','entries','requires'])||manifest.format!=='toskcad.bundled-library'||manifest.version!==1||typeof manifest.revision!=='string'||!manifest.revision||manifest.revision.length>100||!Array.isArray(manifest.entries)||!manifest.entries.length||manifest.entries.length>256||!Array.isArray(manifest.requires)||manifest.requires.length>256)throw new Error('Invalid bundled Library manifest');
  const inside=file=>{
    if(!relativeExamplePath(file)||file.includes(':'))throw new Error(`Invalid bundled Library path: ${file}`);
    const path=join(directory,...file.split('/'));
    if(!lstatSync(path).isFile()||!realpathSync(path).startsWith(base+sep))throw new Error(`Bundled Library path is not a regular in-folder file: ${file}`);
    return path;
  };
  const files=new Set(['library.json']),identities=new Set(),projects=[];
  for(const entry of manifest.entries){
    if(!only(entry,['file','document','sha256'])||typeof entry.file!=='string'||entry.file.includes('/')||!entry.file.endsWith('.tcd')||files.has(entry.file)||typeof entry.document!=='string'||identities.has(entry.document)||! /^[0-9a-f]{64}$/.test(entry.sha256??''))throw new Error('Invalid or duplicate bundled Library entry');
    const path=inside(entry.file),bytes=readFileSync(path);
    if(bytes.length>32*1024*1024||createHash('sha256').update(bytes).digest('hex')!==entry.sha256)throw new Error(`Bundled Library checksum mismatch: ${entry.file}`);
    const project=checkProjectContainer(path);
    if(project.document!==entry.document)throw new Error(`Bundled Library document identity mismatch: ${entry.file}`);
    projects.push({file:entry.file,...project});files.add(entry.file);identities.add(entry.document);
  }
  for(const required of manifest.requires){inside(required);files.add(required);}
  for(const required of ['curation.json','washers-curation.json','README.md','sources/bolts/bearings.blt','sources/bolts/washer.blt','sources/bolts/WASHER-NOTICES.md','machine-screws-curation.json','sources/bolts/hex_socket.blt','sources/bolts/hex.blt','sources/bolts/MACHINE-SCREW-NOTICES.md','sources/bolts/SCREW-PROFILE-NOTICES.md','nuts-curation.json','sources/bolts/nut.blt','sources/bolts/NUT-NOTICES.md','sources/bolts/COPYING.LESSER','wood-screws-curation.json','sources/wood-screws/source-ledger.json','sources/wood-screws/WOOD-SCREW-NOTICES.md'])if(!files.has(required))throw new Error(`Bundled Library notice/source closure is missing ${required}`);
  const curation=JSON.parse(readFileSync(inside('curation.json'),'utf8'));
  if(curation.data_license!=='LGPL-2.1-or-later'||createHash('sha256').update(readFileSync(inside('sources/bolts/bearings.blt'))).digest('hex')!==curation.data_sha256)throw new Error('Bundled Library reference license/hash differs from curation');
  const washers=JSON.parse(readFileSync(inside('washers-curation.json'),'utf8'));
  if(washers.format!=='toskcad.washer-curation'||washers.version!==1||washers.data_license!=='LGPL-2.1-or-later'||createHash('sha256').update(readFileSync(inside('sources/bolts/washer.blt'))).digest('hex')!==washers.data_sha256)throw new Error('Bundled Library washer reference license/hash differs from curation');
  const screws=JSON.parse(readFileSync(inside('machine-screws-curation.json'),'utf8'));
  if(screws.format!=='toskcad.machine-screw-curation'||screws.version!==1||!Array.isArray(screws.data_sources)||screws.data_sources.length!==2)throw new Error('Invalid bundled machine screw curation');
  for(const source of screws.data_sources)if(!files.has(source.file)||source.license!=='LGPL-2.1-or-later'||createHash('sha256').update(readFileSync(inside(source.file))).digest('hex')!==source.sha256)throw new Error('Bundled Library machine screw source license/hash differs from curation');
  if(screws.profile_revision!=='1'||!Array.isArray(screws.profile_entries)||screws.profile_entries.length!==52||!Array.isArray(screws.profile_coverage)||screws.profile_coverage.length!==96||!Array.isArray(screws.profile_sources)||screws.profile_sources.length!==11)throw new Error('Invalid bundled screw profile curation');
  const profileIds=new Set(),profileFiles=new Set(),profileSources=new Set();
  for(const source of screws.profile_sources){
    if(typeof source.id!=='string'||profileSources.has(source.id)||!/^https:\/\//.test(source.url??'')||!/^[0-9a-f]{64}$/.test(source.sha256??'')||!source.edition||!source.reference||!source.rights)throw new Error('Invalid bundled screw profile source reference');
    profileSources.add(source.id);
  }
  for(const entry of screws.profile_entries){
    const source=entry.source_facts?.verification;
    if(typeof entry.id!=='string'||profileIds.has(entry.id)||profileFiles.has(entry.file)||!files.has(entry.file)||!screws.profile_sources.some(ref=>ref.url===entry.verification_url&&ref.sha256===source?.source_sha256)||!source?.reference||!entry.source_reference||!Array.isArray(entry.modifications)||!entry.modifications.length)throw new Error('Invalid bundled screw profile project/source closure');
    profileIds.add(entry.id);profileFiles.add(entry.file);
  }
  const coverageCells=new Set(),coveredProfiles=new Set();
  for(const cell of screws.profile_coverage){
    const key=JSON.stringify([cell.family,cell.nominal_mm]);
    if(coverageCells.has(key)||!cell.reason||!['supported','unavailable'].includes(cell.status)||(cell.status==='supported'?(!screws.profile_entries.some(entry=>entry.id===cell.entry_id&&entry.family===cell.family&&entry.nominal_mm===cell.nominal_mm)||coveredProfiles.has(cell.entry_id)):cell.entry_id!=null))throw new Error('Invalid bundled screw profile coverage');
    coverageCells.add(key);if(cell.status==='supported')coveredProfiles.add(cell.entry_id);
  }
  if(coveredProfiles.size!==profileIds.size)throw new Error('Bundled screw profile coverage omits a project');
  const nuts=JSON.parse(readFileSync(inside('nuts-curation.json'),'utf8'));
  if(nuts.format!=='toskcad.nut-curation'||nuts.version!==1||!Array.isArray(nuts.entries)||nuts.entries.length!==19||!Array.isArray(nuts.coverage)||nuts.coverage.length!==24||nuts.entries.some(entry=>!files.has(entry.file)))throw new Error('Invalid bundled nut curation or project closure');
  if(nuts.data_license!=='LGPL-2.1-or-later'||createHash('sha256').update(readFileSync(inside('sources/bolts/nut.blt'))).digest('hex')!==nuts.data_sha256)throw new Error('Bundled Library nut reference license/hash differs from curation');
  validateWoodCatalog(inside,files,projects);
  const unlisted=readdirSync(directory).filter(name=>name.endsWith('.tcd')&&!files.has(name));
  if(unlisted.length)throw new Error(`Unlisted bundled Library projects: ${unlisted.join(', ')}`);
  return {files:[...files].sort(),projects};
}
export function bundleLibrary(resources,directory=join(root,'library')){
  const checked=validateBundledLibrary(directory),target=join(resources,'library');mkdirSync(target,{recursive:true});
  for(const file of checked.files){const destination=join(target,...file.split('/'));mkdirSync(dirname(destination),{recursive:true});copyFileSync(join(directory,...file.split('/')),destination);}
  return checked;
}
export function runtimeExecutable(stage,platform=process.platform){return platform==='darwin'?join(stage,'MultiCAD.app/Contents/MacOS/multicad'):join(stage,platform==='win32'?'multicad.exe':'multicad');}
// The current development/archive policy is ad-hoc signing. Seal the outer
// bundle after its resources; a copied executable signature does not seal a
// newly assembled app. Publisher signing must follow the final resource write.
function sealMacOSBundle(resources){
  if(process.platform!=='darwin')return;
  const bundle=resolve(resources,'../..');
  run('codesign',['--force','--sign','-',bundle]);
  run('codesign',['--verify','--deep','--strict',bundle]);
}
export function assembleRuntime(stage,binary,version,{rustOnly=false}={}){
  mkdirSync(stage,{recursive:true});
  const executable=runtimeExecutable(stage);mkdirSync(dirname(executable),{recursive:true});copyFileSync(binary,executable);
  if(process.platform==='darwin'){
    const contents=join(stage,'MultiCAD.app/Contents');
    writeFileSync(join(contents,'Info.plist'),`<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>multicad</string><key>CFBundleIdentifier</key><string>de.tokenet.multicad</string><key>CFBundleName</key><string>ToskCAD</string><key>CFBundleDisplayName</key><string>ToskCAD</string><key>CFBundleIconFile</key><string>AppIcon.icns</string><key>CFBundlePackageType</key><string>APPL</string><key>NSPrincipalClass</key><string>NSApplication</string><key>CFBundleShortVersionString</key><string>${escapeXML(version.split('-')[0])}</string><key>CFBundleVersion</key><string>${escapeXML(version.split('-')[0])}</string><key>LSMinimumSystemVersion</key><string>13.0</string><key>NSHighResolutionCapable</key><true/>${macOSDocumentTypes()}</dict></plist>\n`);
  }else if(process.platform==='linux'){
    writeFileSync(join(stage,'multicad.desktop'),`[Desktop Entry]\nType=Application\nName=ToskCAD\nExec=multicad %f\nIcon=multicad\nTerminal=false\nCategories=Graphics;Engineering;\nMimeType=${PROJECT_TYPES.map(type=>type.mime).join(';')};\n`);
    const mime=join(stage,'share/mime/packages');mkdirSync(mime,{recursive:true});
    writeFileSync(join(mime,'toskcad.xml'),`<?xml version="1.0" encoding="UTF-8"?>\n<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">\n${PROJECT_TYPES.map(type=>`  <mime-type type="${type.mime}">\n    <comment>${escapeXML(type.name)}</comment>\n    <glob pattern="*.${type.extension}" case-sensitive="false"/>\n  </mime-type>\n`).join('')}</mime-info>\n`);
    const icons=join(stage,'share/icons/hicolor/512x512/apps');mkdirSync(icons,{recursive:true});
    copyFileSync(join(root,'assets/branding/app/multicad.png'),join(icons,'multicad.png'));
  }
  const resources=process.platform==='darwin'?join(stage,'MultiCAD.app/Contents/Resources'):join(stage,'resources');mkdirSync(resources,{recursive:true});
  const iconName=process.platform==='darwin'?'AppIcon.icns':process.platform==='win32'?'AppIcon.ico':'AppIcon.png';
  copyFileSync(join(root,'assets/branding/app',iconName),join(resources,iconName));
  copyFileSync(join(root,'LICENSE'),join(resources,'LICENSE'));
  if(!rustOnly){
    copyFileSync(join(root,'native-dependencies.json'),join(resources,'native-dependencies.json'));
    for(const name of ['OCCT-LICENSE-LGPL-2.1.txt','OCCT-LGPL-EXCEPTION.txt'])copyFileSync(join(root,'docs/architecture/foundation',name),join(resources,name));
  }
  // Fonts vendored in the source tree for sketch text ship their licence texts.
  const fonts=join(root,'crates/multicad-core/fonts'),fontNotices=join(resources,'third-party/bundled-fonts');mkdirSync(fontNotices,{recursive:true});
  for(const name of readdirSync(fonts))if(isNoticeFile(join(fonts,name)))copyFileSync(join(fonts,name),join(fontNotices,name));
  bundleLibrary(resources);
  const examples=bundleExamples(resources);
  const build=rustOnly?'Rust-only development candidate. Some modeling and export features still require the OpenCascade build.':'native development build.';
  writeFileSync(join(stage,'README.txt'),`ToskCAD ${version} ${build}\nIncludes parameter editing, constrained sketches and initial native solid extrusion.\nNative .tcd projects support save/open, embedded resources and explicit recovery; existing .multicad projects still open and save in place.\nSpecialized workspaces and drawing and 3D export workflows remain under development.\nBundled examples (Examples, or the welcome screen with no recent projects): ${examples.projects.filter(p=>examples.examples.includes(p.file)).map(p=>p.name).join(', ')}. Opening one saves an editable copy in Documents/ToskCAD Examples; the bundled originals are never changed.\nRun with no arguments to open the native window, or pass one PROJECT.tcd to open it; --diagnostics is headless.\nmulticad --mcp serves a local MCP client over stdin/stdout; it opens no network port.\nmacOS: open MultiCAD.app. Windows: run multicad.exe. Linux: run ./multicad on an Ubuntu 24.04+ desktop with graphics drivers.\nNo Python, Node.js, FreeCAD or source checkout is required at runtime.\n${process.platform==='darwin'?'This macOS bundle is ad-hoc signed, without publisher signing or notarization.':'This development build is not publisher-signed.'}\nSource and third-party notices accompany release archives.\n`);
  sealMacOSBundle(resources);
  return {executable,resources};
}

export function stageSourceSupport(destination,commit,version,{rustOnly=false}={}){
  const instructions=join(destination,'SOURCE-REPRODUCTION.md');
  const native=join(destination,'artifacts/native-deps');
  if(rustOnly){
    if(existsSync(native))throw new Error('Rust candidate source contains native dependency assets');
    writeFileSync(instructions,`# ToskCAD ${version} Rust-only development candidate\n\nSource commit: ${commit}\n\nThis tree contains the exact project source and vendored Cargo dependencies for the Rust-only executable. Install the pinned Rust toolchain and your platform SDK, then run:\n\n    MULTICAD_RELEASE_VERSION=${version} MULTICAD_BUILD_COMMIT=${commit} cargo build ${rustCandidateBuildArgs.join(' ')} --offline\n\nOn PowerShell set both environment variables before running Cargo.\n`);
    return;
  }
  // Everything needed to rebuild/relink the static native library is supplied.
  mkdirSync(native,{recursive:true});
  copyFileSync(join(root,'artifacts/native-deps/occt-7.9.3.tar.gz'),join(native,'occt-7.9.3.tar.gz'));
  // Include actual extracted source too, so any local native patches remain
  // part of the corresponding source rather than only shipping the base tar.
  cpSync(join(root,'artifacts/native-deps/OCCT-7_9_3'),join(native,'OCCT-7_9_3'),{recursive:true});
  writeFileSync(instructions,`# MultiCAD ${version}\n\nSource commit: ${commit}\n\nThis tree includes product source, Cargo.lock, vendored Cargo dependencies and the checksum-pinned OCCT source archive. It permits rebuilding the complete application against a changed OCCT library.\n\nInstall the pinned Rust toolchain, CMake, a C++17 toolchain and Node.js (build tooling only). From this folder run:\n\n    node tools/foundation-deps.mjs\n    MULTICAD_RELEASE_VERSION=${version} MULTICAD_BUILD_COMMIT=${commit} cargo build --workspace --release --locked --offline\n\nOn PowerShell set the two environment variables separately before cargo. Native caches and target output can be deleted and regenerated; no preinstalled OCCT/FreeCAD/Python is used. The upstream OCCT archive and exception/license are included. To change OCCT, replace the source/archive pin and rebuild; do not use the old native cache as evidence of a changed library. Build tools and OS SDKs are prerequisites.\n`);
}
export function createSourceTree(destination,commit,version,options={}){
  mkdirSync(destination,{recursive:true});
  const archive=execFileSync('git',['archive','--format=tar',commit],{cwd:root,maxBuffer:128*1024*1024});
  execFileSync('tar',['-xf','-','-C',destination],{input:archive});
  stageSourceSupport(destination,commit,version,options);
  const vendorConfig=run('cargo',['vendor','--offline','--locked','--versioned-dirs',join(destination,'vendor')],{cwd:destination,maxBuffer:16*1024*1024});
  const cargoDirectory=join(destination,'.cargo');mkdirSync(cargoDirectory,{recursive:true});
  const config=join(cargoDirectory,'config.toml');
  const previous=existsSync(config)?readFileSync(config,'utf8'):'';
  const local=vendorConfig.replace(/^directory = .*$/m,'directory = "vendor"');
  writeFileSync(config,previous+'\n'+local+'\n');
  const selection=options.rustOnly?['--manifest-path','crates/multicad-app/Cargo.toml','--no-default-features']:[];
  const metadata=JSON.parse(run('cargo',['metadata','--offline','--locked','--format-version','1',...selection],{cwd:destination,maxBuffer:32*1024*1024}));
  return metadata;
}
function walk(path,visit){for(const e of readdirSync(path,{withFileTypes:true})){const p=join(path,e.name);if(e.isDirectory())walk(p,visit);else if(e.isFile())visit(p);}}
// Licence texts, including a font's same-stem .txt licence sidecar (Hack-Regular.txt
// beside Hack-Regular.ttf in epaint_default_fonts, used for sketch text outlines).
export function isNoticeFile(path){
  const name=path.split(/[\\/]/).at(-1);
  if(path.endsWith('.rs'))return false;
  if(/licen[cs]e|copying|copyright|notice|\bofl\b|\bufl\b/i.test(name))return true;
  const font=/^(.+)\.txt$/i.exec(name);
  return Boolean(font)&&['.ttf','.otf'].some(extension=>existsSync(join(dirname(path),font[1]+extension)));
}
export function collectNotices(resources,source,metadata){
  const notices=join(resources,'third-party');mkdirSync(notices,{recursive:true});
  const records=[];
  for(const p of metadata.packages){
    if(p.source===null)continue;
    if(!p.license)throw new Error(`Missing license metadata for ${p.name}`);
    const directory=dirname(p.manifest_path);const target=join(notices,`${p.name}-${p.version}`);
    walk(directory,path=>{if(isNoticeFile(path)){const dest=join(target,relative(directory,path));mkdirSync(dirname(dest),{recursive:true});copyFileSync(path,dest);}});
    records.push({name:p.name,version:p.version,license:p.license,source:p.source});
  }
  writeFileSync(join(resources,'third-party.json'),JSON.stringify(records.sort((a,b)=>a.name.localeCompare(b.name)),null,2)+'\n');
  sealMacOSBundle(resources);
}
export function auditRuntime(binary){
  if(process.platform==='darwin'){
    const libraries=run('otool',['-L',binary]).split('\n').slice(1).map(line=>line.trim().split(' (')[0]).filter(Boolean);
    for(const library of libraries)if(!library.startsWith('/System/Library/')&&!library.startsWith('/usr/lib/'))throw new Error(`Unbundled non-system dependency: ${library}`);
    return libraries;
  }
  if(process.platform==='linux'){
    const output=run('ldd',[binary]);if(output.includes('not found'))throw new Error('Unresolved native runtime dependency');
    if(/\/home\/|\/opt\/|\/target\//.test(output))throw new Error('Developer-path runtime dependency');return output.split('\n');
  }
  const imports=peImports(binary);
  for(const name of imports)if(!/^(?:api-ms-|ext-ms-)/i.test(name)&&!existsSync(join(process.env.SystemRoot??'C:\\Windows','System32',name)))throw new Error(`Unbundled non-system Windows dependency: ${name}`);
  return imports;
}
export function peImports(binary){
  const b=readFileSync(binary);const pe=b.readUInt32LE(0x3c);if(b.toString('ascii',pe,pe+4)!=='PE\0\0')throw new Error('Not a PE executable');
  const optional=pe+24,sections=b.readUInt16LE(pe+6),sectionStart=optional+b.readUInt16LE(pe+20);
  if(b.readUInt16LE(optional)!==0x20b)throw new Error('Expected a 64-bit Windows executable');
  const offset=rva=>{for(let i=0;i<sections;i++){const s=sectionStart+i*40;const start=b.readUInt32LE(s+12),size=Math.max(b.readUInt32LE(s+8),b.readUInt32LE(s+16));if(rva>=start&&rva<start+size)return b.readUInt32LE(s+20)+rva-start;}throw new Error('Unmapped PE RVA');};
  const rva=b.readUInt32LE(optional+120);if(!rva)return [];
  const imports=[];for(let d=offset(rva);b.readUInt32LE(d)||b.readUInt32LE(d+12);d+=20){const start=offset(b.readUInt32LE(d+12)),end=b.indexOf(0,start);if(end<start)throw new Error('Invalid PE import');imports.push(b.toString('ascii',start,end));}
  const forbidden=imports.filter(name=>/^(?:msvcp|vcruntime|concrt|python|tk|qt)/i.test(name));
  if(forbidden.length)throw new Error(`Unbundled Windows runtime: ${forbidden.join(', ')}`);
  return imports;
}
