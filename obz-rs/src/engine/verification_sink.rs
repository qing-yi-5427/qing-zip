//! Host-owned SHA state. No worker digest is accepted as evidence.
use super::*;
use sha2::{Digest,Sha256};
const CHUNK_LIMIT:usize=64*1024;
#[cfg(test)]
thread_local! {pub(super) static CHUNKS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
#[cfg(test)]
thread_local! {pub(super) static AFTER_CHUNK:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}
struct Item { size:u64, offset:u64, expected:[u8;32], hash:Sha256, opened:bool, closed:bool }
pub(super) struct HashSink { items:BTreeMap<usize,Item> }
impl HashSink {
    pub(super) fn new(entries:&[Entry],expected:&BTreeMap<String,[u8;32]>)->Result<Self,String> {
        let mut items=BTreeMap::new();
        for (index,entry) in entries.iter().enumerate(){if let Some(hash)=expected.get(&entry.name){
            if entry.is_dir{return Err("directory cannot have payload proof".into());}
            items.insert(index,Item{size:entry.size,offset:0,expected:*hash,hash:Sha256::new(),opened:false,closed:false});
        }}
        if items.len()!=expected.len(){return Err("missing SHA proof member".into());}
        Ok(Self{items})
    }
    pub(super) fn wanted(&self,index:usize)->bool { self.items.contains_key(&index) }
    pub(super) fn frame(&mut self,kind:u8,data:&mut Fields,nonce:[u8;16])->Result<(u8,Vec<u8>),String> {
        let index=data.u32()? as usize;let offset=data.u64()?;
        let item=self.items.get_mut(&index).ok_or("unrequested proof stream")?;
        let mut reply=nonce.to_vec();reply.extend_from_slice(&(index as u32).to_le_bytes());reply.extend_from_slice(&offset.to_le_bytes());
        match kind {
            12=>{data.end()?;if item.opened||item.closed||offset!=item.size{return Err("invalid proof stream open".into());}item.opened=true;},
            14=>{
                let length=data.u32()? as usize;
                if !item.opened||item.closed||offset!=item.offset||length==0||length>CHUNK_LIMIT{return Err("invalid proof chunk state or length".into());}
                let end=offset.checked_add(length as u64).filter(|n|*n<=item.size).ok_or("proof chunk exceeds member size")?;
                let bytes=data.take(length)?;data.end()?;item.hash.update(bytes);item.offset=end;
                #[cfg(test)]
                {CHUNKS.with(|v|v.set(v.get()+1));AFTER_CHUNK.with(|h|{if let Some(run)=h.borrow_mut().take(){run();}});}
            },
            16=>{data.end()?;if !item.opened||item.closed||offset!=item.offset||offset!=item.size{return Err("incomplete proof stream close".into());}item.closed=true;},
            _=>return Err("unexpected proof stream frame".into()),
        }
        Ok((kind+1,reply))
    }
    pub(super) fn finish(&self)->Result<(),String> {
        for item in self.items.values(){
            if !item.opened||!item.closed||item.offset!=item.size{return Err("incomplete host SHA ledger".into());}
            let actual:[u8;32]=item.hash.clone().finalize().into();
            if actual!=item.expected{return Err("Candidate payload mismatch".into());}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sink()->HashSink {HashSink{items:BTreeMap::from([(0,Item{size:3,offset:0,expected:Sha256::digest(b"abc").into(),hash:Sha256::new(),opened:false,closed:false})])}}
    fn frame(index:u32,offset:u64,payload:Option<&[u8]>)->Vec<u8>{let mut v=index.to_le_bytes().to_vec();v.extend_from_slice(&offset.to_le_bytes());if let Some(p)=payload{v.extend_from_slice(&(p.len() as u32).to_le_bytes());v.extend_from_slice(p);}v}
    fn send(s:&mut HashSink,kind:u8,index:u32,offset:u64,payload:Option<&[u8]>)->Result<(),String>{s.frame(kind,&mut Fields(&frame(index,offset,payload)),[42;16]).map(|_|())}
    #[test]
    fn sha_stream_is_host_computed_and_requires_exact_open_write_close() {
        let mut s=sink();assert!(s.finish().is_err());send(&mut s,12,0,3,None).unwrap();send(&mut s,14,0,0,Some(b"a")).unwrap();send(&mut s,14,0,1,Some(b"bc")).unwrap();assert!(s.finish().is_err());send(&mut s,16,0,3,None).unwrap();s.finish().unwrap();
        assert!(send(&mut s,14,0,3,Some(b"x")).is_err());assert!(send(&mut s,16,0,3,None).is_err());assert!(send(&mut s,12,0,3,None).is_err());
        let mut s=sink();send(&mut s,12,0,3,None).unwrap();send(&mut s,14,0,0,Some(b"xyz")).unwrap();send(&mut s,16,0,3,None).unwrap();assert!(s.finish().is_err());
    }
    #[test]
    fn sha_stream_rejects_unrequested_out_of_order_overflow_truncated_and_trailing_frames() {
        for (kind,index,offset,payload) in [(12,1,3,None),(14,0,0,Some(&b"a"[..])),(16,0,0,None),(12,0,2,None)]{assert!(send(&mut sink(),kind,index,offset,payload).is_err());}
        for (offset,payload) in [(1,&b"a"[..]),(u64::MAX,&b"a"[..]),(0,&b"abcd"[..]),(0,&b""[..])] {let mut s=sink();send(&mut s,12,0,3,None).unwrap();assert!(send(&mut s,14,0,offset,Some(payload)).is_err());}
        let mut s=sink();send(&mut s,12,0,3,None).unwrap();send(&mut s,14,0,0,Some(b"a")).unwrap();assert!(send(&mut s,14,0,0,Some(b"a")).is_err());assert!(send(&mut s,16,0,1,None).is_err());
        for length in 0..12{assert!(sink().frame(12,&mut Fields(&frame(0,3,None)[..length]),[42;16]).is_err());}
        let mut tail=frame(0,3,None);tail.push(0);assert!(sink().frame(12,&mut Fields(&tail),[42;16]).is_err());
        let mut s=sink();s.items.get_mut(&0).unwrap().size=100000;send(&mut s,12,0,100000,None).unwrap();assert!(send(&mut s,14,0,0,Some(&vec![0;65537])).is_err());
    }
}
