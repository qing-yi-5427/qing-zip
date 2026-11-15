const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const context={};vm.createContext(context);
vm.runInContext(source.split('\n').find(line=>line.startsWith('function archiveOpenMessage(')),context);
test('encrypted directory asks for a password instead of exposing console cancellation',()=>{
 const result=context.archiveOpenMessage('archive engine failed (255): Enter password (will not be echoed): Break signaled');
 assert.match(result,/目录已加密/);assert.doesNotMatch(result,/255|Break/);
});
test('wrong supplied password is distinguishable from a missing password',()=>{
 assert.match(context.archiveOpenMessage('ERROR: Wrong password?',true),/密码不正确/);
 assert.match(context.archiveOpenMessage('Enter password:',true),/检查密码/);
});
test('ordinary corruption and cancellation errors remain visible',()=>{
 for(const message of ['CRC failed','operation cancelled','Cannot open C:\\password.zip as archive'])
  assert.equal(context.archiveOpenMessage('Error: '+message),message);
});
