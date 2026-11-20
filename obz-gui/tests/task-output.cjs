const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const makeContext=value=>vm.createContext(Object.assign(value,{deepExtract:{jobActions:()=>''},performance,updateToolbar:()=>{},schedulePendingQueue:()=>{}}));
const taskSource=source=>source.slice(source.indexOf('const ordinaryRetries='),source.indexOf('const tabs='));
const runSource=source=>source.slice(source.indexOf('async function run('),source.indexOf('function forgetRecycledArchives('));


test('cancelled tasks use a neutral message without hiding real failure details', () => {
  const source=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8');
  const field={textContent:''};const notices=[];
  const context=makeContext({s:{jobs:[]},esc:String,shellQueueStatus:()=>'',btn:()=>'',overlay:{hidden:false},$:()=>field,toast:value=>notices.push(value)});
  for(const prefix of ['function taskError(', 'function error(', 'function historyPage()'])
    vm.runInContext(source.split('\n').find(line=>line.startsWith(prefix)),context);
  vm.runInContext(taskSource(source),context);
  context.error(Error('operation cancelled'));
  assert.deepEqual(notices,['任务已取消']);assert.equal(field.textContent,'');
  const nested='写回失败，内层工作副本已保留，可重试或另存为：operation cancelled';
  context.error(Error(nested));
  assert.equal(notices.at(-1),'任务已取消。内层工作副本已保留，可重试或另存为。');
  assert.equal(field.textContent,'');
  assert.equal(context.taskError(nested+'; cleanup failed'),nested+'; cleanup failed');
  context.s.jobs=[{label:'解压',status:'cancelled',error:'operation cancelled'}];
  for(const name of ['jobs','historyPage']) {
    const html=context[name]();assert.match(html,/任务已取消/);assert.doesNotMatch(html,/operation cancelled|Error:/);
  }
  const detail='CRC failed: operation cancelled.txt';
  context.s.jobs[0]={label:'解压',status:'failed',error:detail};
  assert.ok(context.jobs().includes(detail));
  context.error(Error(detail));assert.equal(field.textContent,'Error: '+detail);
  assert.equal(context.s.jobs[0].error,detail);
});

test('clicking a task result reveals that task output, including numeric native job IDs', async () => {
  const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
  const handler = source.split('\n').find(line => line.startsWith("root.addEventListener('click',async") && line.includes('dataset.jobOutput'));
  assert.ok(handler);
  let click;
  const calls = [];
  const jobs = [
    {id: 7, outputPath: 'D:\\extracted'},
    {id: 8, outputPath: 'D:\\new archive.zip'}
  ];
  vm.runInNewContext(handler, {
    root: {addEventListener: (_, fn) => { click = fn; }},
    s: {jobs}, invoke: (command, args) => calls.push([command, args.path]),
    error: e => { throw e; }
  });
  for (const id of ['8', '7', '42']) {
    await click({target: {closest: () => ({dataset: {jobOutput: id}})}});
  }
  assert.deepEqual(calls, [['reveal_path', jobs[1].outputPath], ['reveal_path', jobs[0].outputPath]]);
});

test('listing results belong to the caller and are not retained in task history', async () => {
  const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
  const run = taskSource(source)+runSource(source);
  const result = {entries: [{name: 'member.txt'}]};
  const state = {jobs: []};
  const context = makeContext({
    s: state,
    invoke: async command => command === 'wait_operation' ? {status:'complete', progress:{}, result} : 1
  });
  vm.runInContext(run, context);
  assert.equal(await context.run({operation:'list'}, 'list', {silent:true}), result);
  assert.equal(state.jobs.length, 1);
  assert.equal(state.jobs[0].status, 'complete');
  assert.ok(!JSON.stringify(state.jobs).includes('member.txt'));
});

test('clipboard export discards incomplete output and publishes only a completed selection', async () => {
  const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
  const copy = source.slice(source.indexOf('async function copySelectedFiles('), source.indexOf('async function run('));
  for (const failure of [null, 'extract', 'publish_clipboard_export']) {
    const calls = [], notices = [];
    const context = makeContext({
      s: {archive: 'sample.zip', selected: new Set(['nested/item.txt'])},
      invoke: async (name, args) => {
        calls.push(name);
        if (name === failure) throw Error(name);
        if (name === 'clipboard_directory') return 'C:\\export';
        if (name === 'publish_clipboard_export') assert.equal(args.paths[0], 'C:\\export\\nested\\item.txt');
      },
      run: async (request, label, options) => {
        calls.push('extract');
        assert.equal(request.dest, 'C:\\export');
        assert.equal(options.temporaryOutput, true);
        if (failure === 'extract') throw Error('cancelled');
      },
      toast: message => notices.push(message)
    });
    const result = vm.runInContext(copy + '\ncopySelectedFiles()', context);
    if (failure) await assert.rejects(result);
    else await result;
    assert.equal(calls.at(-1), 'discard_clipboard_export');
    assert.equal(calls.includes('publish_clipboard_export'), failure !== 'extract');
    assert.equal(notices.length, failure ? 0 : 1);
  }
});


test('partial moves retain counts and the backend reason in task history', async () => {
  const source=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8');
  const run=taskSource(source)+runSource(source);
  for(const reason of [null,'目标的数据流已改变，保留来源']) {
    const result={moved:2,copied:1,retained_sources:['C:/source/a'],retained_reasons:reason?[reason]:[]};
    const state={jobs:[]};let forgotten=false;
    const context=makeContext({s:state,invoke:async command=>{
      if(command==='wait_operation')return {status:'complete',progress:{},result};
      if(command==='forget_operation')forgotten=true;
      return 1;
    }});
    vm.runInContext(run,context);
    assert.equal(await context.run({operation:'fs-move'},'移动文件',{silent:true}),result);
    assert.ok(forgotten);assert.match(state.jobs[0].summary,/已移动 2 项/);
    assert.match(state.jobs[0].summary,/1 个来源仍保留/);
    assert.match(state.jobs[0].summary,/目标副本已保留/);
    if(reason)assert.ok(state.jobs[0].summary.includes(reason));
    assert.equal(state.jobs[0].partial,true);
    Object.assign(context,{esc:String,shellQueueStatus:()=>'',btn:()=>''});
    vm.runInContext(source.split('\n').find(line=>line.startsWith('function jobs()')),context);
    const card=context.jobs();assert.match(card,/部分完成/);assert.ok(!card.includes('已完成'));
    if(reason)assert.ok(card.includes(reason));
    vm.runInContext(source.split('\n').find(line=>line.startsWith('function historyPage()')),context);
    const history=context.historyPage();assert.match(history,/部分完成/);assert.ok(!history.includes('已完成'));
    if(reason)assert.ok(history.includes(reason));
  }
});


test('directory content merge shows retained metadata in both task presentations',async()=>{
 const source=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8');
 const result={copied:3,retained_sources:[],metadata_not_merged:['C:/source/folder']};const state={jobs:[]};
 const context=makeContext({s:state,esc:String,shellQueueStatus:()=>'',btn:()=>'',invoke:async command=>command==='wait_operation'?{status:'complete',progress:{},result}:1});
 vm.runInContext(taskSource(source)+runSource(source),context);
 assert.equal(await context.run({operation:'fs-copy'},'复制文件',{silent:true}),result);
 for(const name of ['jobs','historyPage']){
  vm.runInContext(source.split('\n').find(line=>line.startsWith('function '+name+'()')),context);
  const html=context[name]();assert.match(html,/部分完成/);assert.match(html,/已复制 3 个文件/);assert.match(html,/附加数据未合并/);assert.ok(html.includes('C:/source/folder'));
 }
});


test('access-time warnings survive task cleanup without marking archive data failed',async()=>{
 const source=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8');
 for(const uncertain of [false,true]){
  const result={outputs:['C:/out.zip'],input_atime:{applied_handles:1,unavailable_handles:0,unconfirmed_sources:uncertain?1:0,warnings:uncertain?[{source:'C:/input.txt',phase:'alternate_stream_proof'}]:[]}};
  const state={jobs:[]};const context=makeContext({s:state,QingFileWorkspace:require('../frontend/file-workspace.js'),esc:String,shellQueueStatus:()=>'',btn:()=>'',invoke:async command=>command==='wait_operation'?{status:'complete',progress:{},result}:1});
  vm.runInContext(taskSource(source)+runSource(source),context);
  await context.run({operation:'create'},'新建压缩文件',{silent:true});
  assert.equal(state.jobs[0].partial,false);
  for(const name of ['jobs','historyPage']){
   vm.runInContext(source.split('\n').find(line=>line.startsWith('function '+name+'()')),context);
   const html=context[name]();assert.match(html,/已完成/);assert.ok(!html.includes('部分完成'));
   assert.equal(html.includes('部分来源文件的访问时间保护无法逐项确认。'),uncertain);
   assert.equal(html.includes('C:/input.txt'),uncertain);
   assert.ok(!html.includes('读取句柄'));
  }
 }
});
