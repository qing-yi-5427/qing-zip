const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const src=fs.readFileSync(require.resolve('../frontend/app.js'),'utf8');
test('progress replacement restores logical action, falls back on completion, never steals outside focus',()=>{
 let focused=null;const cancel={dataset:{action:'canceljob'},focus(){focused='cancel'}},output={dataset:{action:'reveal-output'},focus(){focused='output'}},close={dataset:{action:'close'},focus(){focused='close'}};
 let controls=[cancel],inside=true;const content={contains:()=>inside,querySelectorAll:()=>controls,querySelector:()=>controls.find(b=>b===output)};
 const context={document:{activeElement:cancel},$:()=>content,overlay:{querySelector:()=>close},s:{active:1}};vm.createContext(context);vm.runInContext(src.slice(src.indexOf('function refreshJobContent('),src.indexOf('function renderProgress(')),context);
 context.refreshJobContent('running');assert.equal(focused,'cancel');controls=[output];context.refreshJobContent('done');assert.equal(focused,'output');controls=[];context.s.active=null;context.refreshJobContent('failed');assert.equal(focused,'close');focused=null;inside=false;context.refreshJobContent('running');assert.equal(focused,null);
});
