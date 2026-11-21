const QingArchiveListing=(()=>{
  const decoder=new TextDecoder('utf-8',{fatal:true});
  function read(body){
    const result=JSON.parse(decoder.decode(body));
    if(result.rows){
      result.entries=result.rows.map(([
        name,flags,size,packed_size,crc='',modified='',attributes='',method='',created='',accessed='',comment=''
      ])=>({
        name,is_dir:!!(flags&1),encrypted:!!(flags&2),size_known:!!(flags&4),
        size,packed_size,crc,modified,attributes,method,created,accessed,comment,
        leaf:name.slice(name.lastIndexOf('/')+1),
      }));
      delete result.rows;
    }
    return result;
  }
  return {read};
})();
