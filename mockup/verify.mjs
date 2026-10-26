import vm from 'node:vm';
import fs from 'node:fs';
import assert from 'node:assert/strict';
const nodes=new Map();
const node=()=>({innerHTML:'',textContent:'',value:'',dataset:{},style:{},classList:{add(){},remove(){}},addEventListener(){},querySelector(){return node();},querySelectorAll(){return[];},focus(){},setSelectionRange(){},hasAttribute(){return false;}});
const document={getElementById(id){if(!nodes.has(id))nodes.set(id,node());return nodes.get(id);},addEventListener(){},querySelector(){return node();}};
const ctx=vm.createContext({document,window:{},localStorage:{setItem(){}},requestAnimationFrame:fn=>fn(),setTimeout:()=>0,clearTimeout(){},console});
for(const file of ['app.js','workflows.js'])vm.runInContext(fs.readFileSync(new URL(file,import.meta.url),'utf8'),ctx,{filename:file});
const run=s=>vm.runInContext(s,ctx);let checks=0;
assert(nodes.get('app').innerHTML.includes('empty-inspector'));assert(!nodes.get('app').innerHTML.includes('AI & connections'));checks+=2;
for(const project of ['makerspace','table','case','console','tower'])for(const mode of run('modes.map(m=>m[0])')){
 run(`go('${project}');switchMode('${mode}')`);
 for(const ribbon of run(`ribbonTabs['${mode}'].map(r=>r[0])`)){
  run(`switchRibbon('${ribbon}')`);const html=nodes.get('app').innerHTML;assert(html.includes('Design canvas'));assert(html.includes('aria-label="Inspector"'));checks++;
 }
}
for(const name of ['projects','library','create-symbol','symbol-update','export','tour','parameter-help','choose-plane','building-types','page-setup','add-page','drawing-material','assign-material','sheet-check','rail-example','new-export-set','edit-export-set','set-preview','mcp','dxf-settings','pdf-settings','step-settings','mesh-settings','print-orientation','print-aids','constraints','repair','pcb','settings','commands','sweep','fixture','sequence','appearance','health','new-project','annotation','add-view','dimension']){
 run(`state.modal='${name}';renderModal()`);assert(nodes.get('overlay-root').innerHTML.includes('role="dialog"'),name);checks++;
}
for(const kind of ['wall','door','window','level','stairs','extrude','joint','carcass','panel','laminate','panel-drill','groove','holes','member-connection','cope','profile','frame-path','base-sheet','bend','sheet-corner','hem','sketch-references','edge-treatment','floor-guide','section-guide','repeat-guides','support','bridge','render-style']){
 run(`beginTool('${kind}','${kind}')`);assert(nodes.get('app').innerHTML.includes('data-action="apply-op"'),kind);checks++;
}
run("go('console','sketch');act('extrude');act('apply-op')");assert.equal(run('state.placed.length'),2);run("act('undo')");assert.equal(run('state.placed.length'),0);run("act('redo')");assert.equal(run('state.placed.length'),2);checks+=3;
run("go('makerspace');act('wall');act('example-wall');act('apply-op')");assert.equal(run('state.customWalls.length'),1);assert(run('scene()').includes('Interior stud'));run("act('undo')");assert.equal(run('state.customWalls.length'),0);checks+=3;
run("go('console');act('flat')");assert.equal(run('state.flat'),true);assert(run('scene()').includes('Unfolded sheet-metal'));checks+=2;
run("act('add-page');state.draftPageName='Bend instructions';act('save-page')");assert.equal(run('activeSheet().name'),'Bend instructions');assert.equal(run('activeSheet().views.length'),0);checks+=2;
run("go('tower');act('floor-guide');act('apply-op');act('generate-sheets')");assert(run('activeSheet().guideId'));assert.equal(run('state.mode'),'drawings');checks+=2;
run("go('case');act('start-rail-example');state.railPitch=40;act('apply-op')");assert.equal(run('state.railLinked'),true);checks++;
run("go('table');switchMode('export')");assert.equal(run('matchedParts(state.exportSets[0]).length'),2);checks++;
run("act('new-export-set');state.draftSetName='Steel only';state.draftSetTags='metal';act('save-export-set')");assert.equal(run('state.exportSets.at(-1).name'),'Steel only');assert.equal(run('matchedParts(state.exportSets.at(-1)).length'),3);checks+=2;
run("act('mcp-sample');act('mcp-inspect');act('mcp-apply')");assert.equal(run('state.railLinked'),true);assert.equal(run('state.mcpPending'),null);run("act('undo')");assert.equal(run('mcpAudit[0].status'),'Undone');checks+=3;
run("switchMode('parameters');act('add-parameter')");assert.equal(run('state.parameters.length'),5);checks++;
run("go('console','model');switchRibbon('wood')");assert(run('scene()').includes('Main PCB'));assert(!run('scene()').includes('data-part=\"Laminated top\"'));checks+=2;
run("go('case');switchMode('parameters');workflowInput({target:{dataset:{param:'0:value'},tagName:'INPUT',type:'text',value:'700',hasAttribute:()=>false}})");assert.equal(run('state.parameters[0].value'),'700');assert(run('parameterResult(state.parameters[3])').includes('638.6'));run("act('undo')");assert.equal(run('state.parameters[0].value'),'600');checks+=3;
console.log(`${checks} prototype render and interaction checks passed.`);
