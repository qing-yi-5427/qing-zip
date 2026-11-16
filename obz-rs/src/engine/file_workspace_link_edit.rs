//! Explicit existing-reparse operations. No existing object is disposed of.
//! TxF excludes metadata-only writers that ordinary sharing pins cannot stop.
use super::*;
use super::super::super::file_workspace_transaction::{Transaction,CommitOutcome};
use std::sync::atomic::{AtomicBool,Ordering};
#[derive(Clone,Debug,Deserialize,Serialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct LinkReceipt{pub volume:String,pub file_id:String,pub tag:String,pub reparse_sha256:String}
#[derive(Debug,Serialize)]
pub struct InspectedLink{pub path:String,pub kind:LinkKind,pub target:String,pub print_target:String,pub relative:bool,pub can_edit:bool,pub edit_reason:Option<String>,pub receipt:LinkReceipt}
#[derive(Debug,Serialize)]
pub struct EditedLink{pub path:String,pub removed:bool,pub receipt:Option<LinkReceipt>}
struct Parsed{kind:LinkKind,target:String,print:String,relative:bool}
fn invalid()->String{"不支持或损坏的链接数据；未修改".into()}
fn u16_at(data:&[u8],pos:usize)->Result<u16,String>{Ok(u16::from_le_bytes(data.get(pos..pos+2).ok_or_else(invalid)?.try_into().map_err(|_|invalid())?))}
fn u32_at(data:&[u8],pos:usize)->Result<u32,String>{Ok(u32::from_le_bytes(data.get(pos..pos+4).ok_or_else(invalid)?.try_into().map_err(|_|invalid())?))}
fn wide_name(data:&[u8],offset:usize,length:usize)->Result<String,String>{
    if offset%2!=0 || length%2!=0{return Err(invalid());}
    let bytes=data.get(offset..offset.checked_add(length).ok_or_else(invalid)?).ok_or_else(invalid)?;
    let wide=bytes.chunks_exact(2).map(|v|u16::from_le_bytes([v[0],v[1]])).collect::<Vec<_>>();
    if wide.contains(&0){return Err(invalid());}String::from_utf16(&wide).map_err(|_|invalid())
}
fn absolute_target(substitute:&str)->Result<String,String>{
    let rest=substitute.strip_prefix("\\??\\").ok_or_else(invalid)?;
    if let Some(unc)=rest.strip_prefix("UNC\\"){
        if unc.split('\\').filter(|p|!p.is_empty()).count()<2{return Err(invalid());}return Ok(format!("\\\\{unc}"));
    }
    let path=Path::new(rest);
    if !path.is_absolute() || !matches!(path.components().next(),Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::Disk(_))){return Err(invalid());}Ok(rest.into())
}
fn parse(data:&[u8],directory:bool)->Result<Parsed,String>{
    if data.len()<8 || data.len()>16384 || u16_at(data,4)? as usize+8!=data.len() || u16_at(data,6)?!=0{return Err(invalid());}
    let tag=u32_at(data,0)?;
    if tag==0xa000001d{
        if directory || u32_at(data,8)?!=2{return Err(invalid());}
        let target=std::str::from_utf8(data.get(12..).ok_or_else(invalid)?).map_err(|_|invalid())?;
        if target.is_empty() || target.contains('\0'){return Err(invalid());}
        return Ok(Parsed{kind:LinkKind::Wsl,target:target.into(),print:String::new(),relative:false});
    }
    if tag!=0xa0000003 && tag!=0xa000000c{return Err(invalid());}
    let symbolic=tag==0xa000000c;if !symbolic&&!directory{return Err(invalid());}
    let flags=if symbolic{u32_at(data,16)?}else{0};if flags&!1!=0{return Err(invalid());}
    let names=data.get(if symbolic{20}else{16}..).ok_or_else(invalid)?;if names.len()%2!=0{return Err(invalid());}
    let sub_offset=u16_at(data,8)? as usize;let sub_len=u16_at(data,10)? as usize;
    let print_offset=u16_at(data,12)? as usize;let print_len=u16_at(data,14)? as usize;
    let sub=wide_name(names,sub_offset,sub_len)?;let print=wide_name(names,print_offset,print_len)?;
    if sub.is_empty(){return Err(invalid());}
    // Unknown bytes outside the declared names cannot be silently discarded.
    for (i,byte) in names.iter().enumerate(){if !(sub_offset..sub_offset+sub_len).contains(&i)&&!(print_offset..print_offset+print_len).contains(&i)&&*byte!=0{return Err(invalid());}}
    let target=if flags==1{
        if Path::new(&sub).is_absolute() || sub.starts_with("\\??\\"){return Err(invalid());}sub
    }else{absolute_target(&sub)?};
    Ok(Parsed{kind:if symbolic{if directory{LinkKind::SymbolicDirectory}else{LinkKind::SymbolicFile}}else{LinkKind::Junction},target,print,relative:flags==1})
}
fn info(file:&File)->Result<[u32;13],String>{
    #[link(name="kernel32")] unsafe extern "system"{fn GetFileInformationByHandle(handle:Handle,info:*mut u32)->i32;}
    let mut data=[0;13];if unsafe{GetFileInformationByHandle(file.as_raw_handle(),data.as_mut_ptr())}==0{return Err(std::io::Error::last_os_error().to_string());}
    Ok(data)
}
fn get_optional(file:&File)->Result<Option<Vec<u8>>,String>{
    let mut data=vec![0u8;16384];let mut size=0;
    if unsafe{DeviceIoControl(file.as_raw_handle(),0x900a8,std::ptr::null(),0,data.as_mut_ptr(),data.len() as u32,&mut size,std::ptr::null_mut())}==0{
        let error=std::io::Error::last_os_error();return if error.raw_os_error()==Some(4390){Ok(None)}else{Err(format!("读取链接数据失败：{error}"))};
    }
    if size as usize>data.len(){return Err(invalid());}data.truncate(size as usize);Ok(Some(data))
}
fn get(file:&File)->Result<Vec<u8>,String>{get_optional(file)?.ok_or("项目没有链接数据".into())}
fn receipt(file:&File,data:&[u8])->Result<LinkReceipt,String>{let id=info(file)?;Ok(LinkReceipt{volume:format!("{:08x}",id[7]),file_id:format!("{:08x}{:08x}",id[11],id[12]),tag:format!("{:08x}",u32_at(data,0)?),reparse_sha256:Sha256::digest(data).iter().map(|v|format!("{v:02x}")).collect()})}
fn observed(file:&File,path:&Path)->Result<InspectedLink,String>{
    let meta=file.metadata().map_err(|e|e.to_string())?;if meta.file_attributes()&0x400==0{return Err("请选择已有链接；普通文件或目录未修改".into());}
    let data=get(file)?;let parsed=parse(&data,meta.file_attributes()&0x10!=0)?;
    let edit_reason=if info(file)?[10]!=1{Some("多硬链接对象仅支持查看，不能编辑或清除链接数据".into())}else{None};
    Ok(InspectedLink{path:path.display().to_string(),kind:parsed.kind,target:parsed.target,print_target:parsed.print,relative:parsed.relative,can_edit:edit_reason.is_none(),edit_reason,receipt:receipt(file,&data)?})
}
pub fn inspect_link(path:&str,opts:&EngineOptions)->Result<InspectedLink,String>{
    let path=checked_path(path)?;let parent_path=path.parent().ok_or("请选择链接项目")?;let name=path.file_name().ok_or("请选择链接项目")?;
    let mut guards=CheckedDirectories::default();let parent=parent_pin(parent_path,&mut guards)?;
    let file=child(&parent,name,None,0x100081,1)?;opts.control.clone().unwrap_or_default().check()?;
    observed(&file,&path)
}
fn replacement(parsed:&Parsed,target:&str)->Result<Vec<u8>,String>{
    match parsed.kind{
        LinkKind::Wsl=>wsl_data(target),LinkKind::Junction=>{let path=checked_path(target)?;check_junction_target(&path)?;junction_data(&path)},
        LinkKind::SymbolicFile|LinkKind::SymbolicDirectory=>{
            if !parsed.relative{return symbolic_data(&checked_path(target)?);}
            if target.is_empty() || target.contains('\0') || Path::new(target).components().any(|c|matches!(c,Component::Prefix(_)|Component::RootDir)){return Err("请输入相对 Windows 目标路径；保持原链接的相对模式".into());}
            for name in target.split(['\\','/']).filter(|v|!v.is_empty()&&!matches!(*v,"."|"..")){checked_name(name)?;}
            let name=target.encode_utf16().collect::<Vec<_>>();let length=12+name.len()*2;
            if length+8>16384{return Err("符号链接目标过长".into());}
            let mut data=Vec::with_capacity(length+8);data.extend(0xa000000cu32.to_le_bytes());data.extend((length as u16).to_le_bytes());data.extend(0u16.to_le_bytes());
            for value in [0,(name.len()*2) as u16,0,0]{data.extend(value.to_le_bytes());}data.extend(1u32.to_le_bytes());for word in name{data.extend(word.to_le_bytes());}Ok(data)
        },_=>Err(invalid()),
    }
}
fn verify(file:&File,expected:Option<&[u8]>,attributes:u32)->Result<(),String>{
    let attrs=file.metadata().map_err(|e|e.to_string())?.file_attributes();
    if attrs&!0x400!=attributes&!0x400{return Err("链接以外的文件属性核验不符".into());}
    match expected{Some(data)=>{if get(file)?!=data{return Err("链接修改后数据核验不符".into());}},None=>{
        if attrs&0x400!=0{return Err("链接数据尚未清除".into());}
        if get_optional(file)?.is_some(){return Err("清除后仍存在链接数据".into());}
    }}Ok(())
}
fn isolated<T:Send>(kind:LinkKind,work:impl FnOnce()->Result<T,String>+Send)->Result<T,String>{if matches!(kind,LinkKind::SymbolicFile|LinkKind::SymbolicDirectory){privilege::run(work)}else{work()}}
pub fn edit_link(path:&str,expected:&LinkReceipt,target:Option<&str>,opts:&EngineOptions)->Result<EditedLink,String>{
    if opts.overwrite!="error"{return Err("链接编辑不接受覆盖选项".into());}
    let path=checked_path(path)?;let parent_path=path.parent().ok_or("请选择链接项目")?;let name=path.file_name().ok_or("请选择链接项目")?;
    let mut guards=CheckedDirectories::default();guards.check(parent_path)?;guards.allow_publication(parent_path)?;let parent=parent_pin(parent_path,&mut guards)?;
    let tx=Transaction::begin().map_err(|e|format!("链接编辑需要可用文件事务；尚未修改：{e}"))?;
    let lease=tx.lock_target(parent_path,directory_identity(&parent)?).map_err(|e|format!("无法锁定链接父目录；尚未修改：{e}"))?;drop(lease);
    // READ_DATA participates in delete sharing. WRITE is shared solely for our
    // transacted handle; it is the TxF view, not this pin, that excludes metadata.
    let pin=child(&parent,name,None,0x100081,3)?;let original=observed(&pin,&path)?;
    if !original.can_edit{return Err(original.edit_reason.unwrap_or("链接仅支持查看".into()));}
    if &original.receipt!=expected{return Err("链接在查看后已改变；请刷新后重试，未修改".into());}
    let file=tx.open_reparse(&path,directory_identity(&pin)?)?;let old=get(&file)?;let attrs=file.metadata().map_err(|e|e.to_string())?.file_attributes();let parsed=parse(&old,attrs&0x10!=0)?;
    if info(&file)?[10]!=1{return Err("多硬链接对象不能编辑；未修改".into());}
    if &receipt(&file,&old)?!=expected{return Err("链接在查看后已改变；请刷新后重试，未修改".into());}
    let new=target.map(|value|replacement(&parsed,value)).transpose()?;let ctl=opts.control.clone().unwrap_or_default();ctl.check()?;
    #[cfg(test)] BEFORE_EDIT.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run();}});
    ctl.check()?;if &receipt(&file,&get(&file)?)?!=expected{return Err("链接编辑前身份或数据改变；未修改".into());}
    let changed=AtomicBool::new(false);let mut header=[0u8;8];header[..4].copy_from_slice(&old[..4]);
    #[cfg(test)] let fail=FAIL_VERIFY.with(|flag|flag.replace(false));
    #[cfg(test)] let fail_restore=FAIL_RESTORE.with(|flag|flag.replace(false));
    #[cfg(test)] let panic_verify=PANIC_VERIFY.with(|flag|flag.replace(false));
    let outcome=isolated(parsed.kind,||{
        ioctl(&file,if new.is_some(){0x900a4}else{0x900ac},new.as_deref().unwrap_or(&header),&mut [])?;changed.store(true,Ordering::Relaxed);
        #[cfg(test)] if panic_verify{panic!("injected existing-link helper panic");}
        #[cfg(test)] if fail{return Err("injected existing-link post-write verification failure".into());}
        ctl.check()?;verify(&file,new.as_deref(),attrs)?;new.as_deref().map(|data|receipt(&file,data)).transpose()
    });
    let next_receipt=match outcome{Ok(value)=>value,Err(error)=>{
        let restored=if changed.load(Ordering::Relaxed){isolated(parsed.kind,||{
            #[cfg(test)] if fail_restore{return Err("injected old reparse restore failure".into());}
            ioctl(&file,0x900a4,&old,&mut [])?;verify(&file,Some(&old),attrs)
        })}else{Ok(())};
        drop(file);return match tx.rollback(){
            Ok(())=>Err(format!("{error}；已确认事务回滚，原链接及数据保留{}",restored.err().map(|e|format!("（原句柄恢复核验未完成：{e}）")).unwrap_or_default())),
            Err(rollback)=>Err(format!("{error}；事务回滚无法确认，链接可能已修改，请重新查看；{rollback}；原句柄恢复结果：{restored:?}")),
        };
    }};
    drop(file);
    tx.commit_checked().map_err(|failure|match failure.outcome{
        CommitOutcome::ConfirmedRolledBack=>format!("链接事务提交失败；已确认回滚，原链接及数据保留：{}",failure.message.replace("删除已回滚，来源仍保留","链接修改已回滚")),
        CommitOutcome::Unknown=>format!("链接事务提交结果无法确认，链接可能已修改，请重新查看：{}",failure.message),
    })?;
    Ok(EditedLink{path:path.display().to_string(),removed:new.is_none(),receipt:next_receipt})
}
#[cfg(test)] thread_local!{
    static BEFORE_EDIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};
    static FAIL_VERIFY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
    static FAIL_RESTORE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
    static PANIC_VERIFY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
}

#[cfg(test)]
#[path="file_workspace_link_edit_tests.rs"]
mod tests;
