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
  function technical(properties) {
    const definitions=[['Type','实际格式'],['Solid','固实压缩'],['Multivolume','多卷归档'],['Volumes','卷数'],['Physical Size','归档物理大小'],['Total Physical Size','总物理大小'],['Offset','起始偏移'],['Tail Size','尾部数据'],['Headers Size','头部大小'],['Method','压缩方法'],['Blocks','数据块'],['Encrypted','加密'],['Characteristics','格式特征']];
    const rows=[['当前打开文件大小',`${BigInt(properties.source_bytes).toLocaleString('zh-CN')} 字节`]];
    properties.layers.forEach((layer,index)=>{
      const prefix=properties.layers.length>1?`第 ${index+1} 层 · `:'';
      for(const [key,label] of definitions){let value=layer.fields[key];
        if(value===undefined)value=missing;
        else if(['Solid','Multivolume','Encrypted'].includes(key))value=value==='+'?'是':'否';
        else if(['Physical Size','Total Physical Size','Offset','Tail Size','Headers Size'].includes(key))value=BigInt(value).toLocaleString('zh-CN')+' 字节';
        rows.push([prefix+label,value]);
      }
    });
    return rows;
  }
  const api = { describe, technical };
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.QingArchiveProperties = api;
})(globalThis);
