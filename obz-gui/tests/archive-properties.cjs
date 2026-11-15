const test = require('node:test');
const assert = require('node:assert/strict');
const { describe } = require('../frontend/archive-properties.js');
const entry = (name, size, extra = {}) => ({name, size, size_known:true, packed_size:0, is_dir:false, encrypted:false, ...extra});
test('unknown member length and omitted packed sizes are never displayed as zero', () => {
  const result = Object.fromEntries(describe([entry('a',0,{size_known:false})],['a'],'archive').rows);
  assert.equal(result['大小'],'未知'); assert.equal(result['压缩后大小'],'未提供');
  const empty = Object.fromEntries(describe([entry('empty',0)],['empty'],'archive').rows);
  assert.equal(empty['大小'],'0 字节'); assert.equal(empty['压缩后大小'],'0 字节');
});
test('folder selections count descendants once, preserve case, and do not select adjacent prefixes', () => {
  const entries = [entry('中文/a',4),entry('中文/deep/b',8),entry('中文2/c',16),entry('OTHER',32),entry('other',64)];
  const rows = Object.fromEntries(describe(entries,['中文/','中文/a','OTHER'],'archive').rows);
  assert.equal(rows['文件数'],'3'); assert.equal(rows['文件总大小'],'44 字节');
});
test('aggregate sizes remain exact beyond Number precision and distinguish unknown members', () => {
  const entries = [entry('a',Number.MAX_SAFE_INTEGER),entry('b',Number.MAX_SAFE_INTEGER),entry('unknown',0,{size_known:false})];
  const rows = Object.fromEntries(describe(entries,[],'archive').rows);
  assert.equal(rows['已知文件大小'],'18,014,398,509,481,982 字节');
  assert.equal(rows['大小未知的文件'],'1');
});
