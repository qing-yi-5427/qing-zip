(function (root) {
  'use strict';
  const missing = '未提供';
  const normalize = name => String(name).replaceAll('\\', '/');
  function bytes(value) {
    return Number.isSafeInteger(value) && value >= 0
      ? `${value.toLocaleString('zh-CN')} 字节` : '大小超出精确显示范围';
  }
  function describe(entries, selected, archive) {
    const names = [...selected].map(normalize);
    const matches = name => names.some(key => name === key || name.startsWith(key.endsWith('/') ? key : key + '/'));
    const included = names.length ? entries.filter(e => matches(normalize(e.name))) : entries;
    const one = names.length === 1 ? entries.find(e => normalize(e.name) === names[0]) : null;
    const rows = [['压缩包路径', archive]];
    if (one && !one.is_dir) {
      rows.push(['包内路径', one.name], ['大小', one.size_known === false ? '未知' : bytes(one.size)],
        ['压缩后大小', one.packed_size > 0 || (one.size_known !== false && one.size === 0) ? bytes(one.packed_size) : missing],
        ['压缩方法', one.method || missing], ['CRC', one.crc || missing],
        ['加密', one.encrypted ? '是' : '否'], ['修改时间', one.modified || missing],
        ['原始属性', one.attributes || missing], ['注释', one.comment || missing]);
      return { title: '文件属性', rows };
    }
    if (names.length === 1) rows.push(['包内路径', names[0]]);
    const files = included.filter(e => !e.is_dir);
    let total = 0n, unknown = 0;
    for (const file of files) {
      if (file.size_known === false || !Number.isSafeInteger(file.size) || file.size < 0) unknown++;
      else total += BigInt(file.size);
    }
    rows.push(['文件数', String(files.length)], ['目录条目', String(included.length - files.length)],
      [unknown ? '已知文件大小' : '文件总大小', `${total.toLocaleString('zh-CN')} 字节`]);
    if (unknown) rows.push(['大小未知的文件', String(unknown)]);
    rows.push(['加密文件', String(files.filter(e => e.encrypted).length)]);
    return { title: names.length ? '所选项目属性' : '压缩包内容属性', rows };
  }
  const api = { describe };
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.QingArchiveProperties = api;
})(globalThis);
