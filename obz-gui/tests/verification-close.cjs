const test=require('node:test'),assert=require('node:assert/strict'),vm=require('node:vm'),fs=require('node:fs'),path=require('node:path');
const source=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8');
const removal=source.slice(source.indexOf('function hideMenu('),source.indexOf('function rememberMenuFocus('))+source.slice(source.indexOf('let removingArchiveTabs='),source.indexOf('\nfunction discardArchiveTab'));
function harness(){let release;const gate=new Promise(r=>release=r);const a={id:'a'},b={id:'b'},c={id:'c'},ctx={root:{querySelector:()=>null},archiveTabs:[a,b,c],currentTabId:'a',s:{archive:'a'},busy:false,wiped:[],invoke:async()=>gate,$:()=>({hidden:false}),tabBusy:()=>ctx.busy,toast(){},saveCurrentTab(){},wipeTab:t=>ctx.wiped.push(t.id),resetPreviewState(){},close(){},applyTab(){},emptyArchiveState(){},render(){},renderArchiveTabs(){},overlay:{hidden:true},document:{activeElement:null},archiveTabFocusOrigin:()=>({id:ctx.currentTabId,page:ctx.s.page,element:null})};vm.createContext(ctx);vm.runInContext(removal,ctx);return{ctx,a,b,c,release};}
test('two overlapping closes cannot splice an unrelated final tab',async()=>{const{ctx,a,b,c,release}=harness();const one=ctx.removeArchiveTabs([a,b]);assert.equal(await ctx.removeArchiveTabs([b]),false);ctx.archiveTabs.splice(ctx.archiveTabs.indexOf(b),1);release();assert.equal(await one,true);assert.deepEqual(ctx.archiveTabs.map(t=>t.id),['c']);assert.deepEqual(ctx.wiped,['a']);});
test('a task beginning while backend revocation is awaited prevents tab removal',async()=>{const{ctx,a,release}=harness();const pending=ctx.removeArchiveTabs([a]);ctx.busy=true;release();assert.equal(await pending,false);assert.equal(ctx.archiveTabs.length,3);assert.equal(ctx.wiped.length,0);});
test('backend revocation failure leaves tabs intact and releases close mutex',async()=>{const{ctx,a,release}=harness();ctx.invoke=async()=>{throw Error('offline')};await assert.rejects(ctx.removeArchiveTabs([a]),/offline/);assert.equal(ctx.archiveTabs.length,3);ctx.invoke=async()=>{};assert.equal(await ctx.removeArchiveTabs([a]),true);release();});
test('finishing an earlier close never clears a newer pending close object',async()=>{let release;const old={tabs:[],exit:false},newer={tabs:[],exit:false};const ctx={pendingNestedClose:old,finishingArchiveTabs:false,tabBusy:()=>false,removeArchiveTabs:()=>new Promise(r=>release=r),invoke:async()=>{},syncNestedSessions:async()=>{},showNestedClose(){}};vm.createContext(ctx);vm.runInContext(source.split('\n').find(l=>l.startsWith('async function finishNestedClose(')),ctx);const pending=ctx.finishNestedClose(false);ctx.pendingNestedClose=newer;release(true);await pending;assert.equal(ctx.pendingNestedClose,newer);assert.equal(ctx.finishingArchiveTabs,false);});

test('an automatic close never discards a nested edit completed after its clean snapshot', async () => {
    const tab = { id: 'tab', nestedId: 1, dirty: false };
    let editCommitted = false;
    let discard;
    let finishRead;
    let readStarted;
    const readingExternalEdits = new Promise(resolve => { readStarted = resolve; });
    let closeFinished;
    const finished = new Promise(resolve => { closeFinished = resolve; });
    const context = {
        currentTabId: tab.id, archiveTabFocusOrigin: () => ({}),
        closingArchiveTabs: false, finishingArchiveTabs: false, removingArchiveTabs: false,
        tabBusy: () => false, saveCurrentTab() {}, nestedCloseSelection: () => [tab],
        pendingNestedClose: null,
        syncNestedSessions: async () => { tab.dirty = editCommitted; },
        invoke: async (command, args) => {
            if (command === 'external_edits') {
                readStarted();
                return new Promise(resolve => { finishRead = () => resolve([]); });
            }
            assert.equal(command, 'close_nested');
            discard = args.discard;
            if (!discard && editCommitted) throw Error('uncommitted nested edit');
        },
        removeArchiveTabs: async () => { closeFinished('removed'); return true; },
        showNestedClose() {},
        error: error => closeFinished(error.message)
    };
    vm.createContext(context);
    vm.runInContext(source.slice(source.indexOf('function closeArchiveTabs('),
        source.indexOf("\nroot.querySelectorAll('[data-lucide]')")), context);
    context.closeArchiveTabs('current');
    await readingExternalEdits;
    editCommitted = true;
    finishRead();
    assert.equal(await finished, 'uncommitted nested edit');
    assert.equal(discard, false);
    assert.equal(tab.dirty, true);
});

test('a partial nested save refreshes dirty badges and preserves the outer write error', async () => {
    const inner = { nestedId: 1, archive: 'inner.zip', dirty: true };
    const middle = { nestedId: 2, archive: 'middle.zip', dirty: false };
    const writeError = new Error('outer archive write failed');
    const context = {
        nestedSaveDisplayPath: tab => tab.archive,
        run: async ({ operation }) => {
            assert.equal(operation, 'nested-save');
            throw writeError;
        },
        syncNestedSessions: async () => {
            inner.dirty = false;
            middle.dirty = true;
        }
    };
    vm.createContext(context);
    vm.runInContext(source.slice(source.indexOf('async function saveNested('),
        source.indexOf('function nestedCloseSelection(')), context);
    await assert.rejects(context.saveNested(inner), error => error === writeError);
    assert.equal(inner.dirty, false);
    assert.equal(middle.dirty, true);
    context.syncNestedSessions = async () => { throw Error('session refresh failed'); };
    await assert.rejects(context.saveNested(inner), error => error === writeError);
});
