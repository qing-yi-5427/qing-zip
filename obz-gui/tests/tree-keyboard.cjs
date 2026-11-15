const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require.resolve('../frontend/app.js'),'utf8');
test('tree navigation uses full logical rows for parent, child, boundaries and expansion',()=>{
 const rows=[{path:'',hasChildren:true},{path:'a/',hasChildren:true},{path:'a/b/'},{path:'z/'}],focused=[];let renders=0,clicked=0;
 const context={renderedTree:{rows},s:{collapsed:new Set()},focusTreePath:p=>focused.push(p),renderChrome:()=>renders++,$:()=>({querySelector:()=>({click:()=>clicked++})}),CSS:{escape:x=>x}};vm.createContext(context);vm.runInContext(source.slice(source.indexOf('function handleTreeKeydown('),source.indexOf('function renderChrome(')),context);
 const press=(path,key)=>context.handleTreeKeydown({key,target:{closest:()=>({dataset:{tree:path}})},preventDefault(){},stopPropagation(){}});
 press('a/','ArrowLeft');assert.ok(context.s.collapsed.has('a/'));assert.equal(renders,1);assert.equal(focused.at(-1),'a/');
 press('a/','ArrowLeft');assert.equal(focused.at(-1),'');press('a/','ArrowRight');assert.ok(!context.s.collapsed.has('a/'));
 press('a/','ArrowRight');assert.equal(focused.at(-1),'a/b/');press('a/b/','ArrowLeft');assert.equal(focused.at(-1),'a/');
 press('a/','End');assert.equal(focused.at(-1),'z/');press('z/','Home');assert.equal(focused.at(-1),'');press('a/b/','Enter');assert.equal(clicked,1);
});
