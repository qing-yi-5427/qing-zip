const QingArchiveListing=(()=>{
  const decoder=new TextDecoder('utf-8',{fatal:true});
  const reuseParent=1<<3;
  function read(body){
    const result=JSON.parse(decoder.decode(body));
    if(result.rows){
      let parent='',leafPrefix='';
      // Consecutive rows may share the previous entry's path through its last
      // separator. Keep the original spelling for properties and operations.
      result.entries=result.rows.map(([
        name,flags,size,packed_size,crc='',modified='',attributes='',method='',created='',accessed='',comment=''
      ])=>{
        let leaf;
        if(flags&reuseParent){
          leaf=leafPrefix+name;
          name=parent+name;
        }else{
          const slash=name.lastIndexOf('/');
          const parentEnd=Math.max(slash,name.lastIndexOf('\\'))+1;
          parent=name.slice(0,parentEnd);
          leafPrefix=parent.slice(slash+1);
          leaf=name.slice(slash+1);
        }
        return {
          name,is_dir:!!(flags&1),encrypted:!!(flags&2),size_known:!!(flags&4),
          size,packed_size,crc,modified,attributes,method,created,accessed,comment,leaf,
        };
      });
      delete result.rows;
    }
    return result;
  }
  return {read};
})();
