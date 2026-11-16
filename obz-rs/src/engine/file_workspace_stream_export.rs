//! Read-only, handle-bound ADS export to ordinary no-replace file outputs.
use super::*;
use std::io::{Read, Seek, SeekFrom, Write};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StreamExportItem { pub name: String, pub size: u64, pub output_name: String }
#[derive(Debug, Serialize)]
pub struct StreamExportReport { pub destination: String, pub outputs: Vec<String>, pub summary: String }

fn valid_stream(name: &str) -> Result<Vec<u16>, String> {
    if name.is_empty() || name.encode_utf16().count() > 32760 || name.chars().any(|c| matches!(c, ':' | '/' | '\\' | '\0')) {
        return Err("无效的附加数据流名称".into());
    }
    Ok(format!(":{name}:$DATA").encode_utf16().collect())
}
fn output_leaf(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains(['/', '\\']) { return Err("请输入单个有效的输出文件名".into()); }
    let leaf=crate::extract::relative_path(name)?;
    if leaf.components().count()!=1 { return Err("请输入单个有效的输出文件名".into()); }
    Ok(leaf)
}
fn source_ok(file: &File, revision: &str) -> Result<(), String> {
    let meta=file.metadata().map_err(|e|e.to_string())?;
    if meta.file_attributes()&0x400!=0 || (!meta.is_file() && !meta.is_dir()) || inventory_revision(file)?!=revision {
        return Err("所属项目已变化，请刷新附加数据流后重试".into());
    }
    Ok(())
}
fn partial_error(error: String, outputs: &[String], total: usize) -> String {
    let names=outputs.iter().take(8).map(|p|format!("\n{p}")).collect::<String>();
    format!("{error}\n已导出 {} / {total} 项；已完成文件予以保留。{names}{}",outputs.len(),if outputs.len()>8 {"\n其余已完成文件请在输出目录查看。"}else{""})
}

pub fn export_streams(text: &str, revision: &str, items: &[StreamExportItem], destination: &str, opts: &EngineOptions) -> Result<StreamExportReport, String> {
    if opts.overwrite!="error" { return Err("数据流导出不允许覆盖已有文件".into()); }
    if revision.is_empty() || items.is_empty() || items.len()>4096 { return Err("请刷新并选择 1–4096 个附加数据流".into()); }
    let ctl=opts.control.clone().unwrap_or_default();ctl.check()?;
    let path=checked_path(text)?;
    let parent=path.parent().ok_or("不能导出卷根目录的数据流")?;
    let name=path.file_name().ok_or("不能导出卷根目录的数据流")?;
    let destination=checked_path(destination)?;
    let mut stream_names=HashSet::new();let mut output_names=HashSet::new();let mut plan=Vec::new();
    for item in items {
        let stream=valid_stream(&item.name)?;let leaf=output_leaf(&item.output_name)?;
        if !stream_names.insert(item.name.to_uppercase()) || !output_names.insert(item.output_name.to_uppercase()) { return Err("重复的数据流或输出文件名；请更改名称后重试".into()); }
        plan.push((stream,leaf));
    }
    let mut guards=CheckedDirectories::default();guards.check(parent)?;guards.check(&destination)?;guards.allow_publication(&destination)?;
    let source=links::child(&guards.held.get(parent).ok_or("无法固定来源父目录")?.0,name,None,0x80100080,1)
        .map_err(|e|format!("无法读取数据流所属项目：{e}"))?;
    source_ok(&source,revision)?;
    let inventory=query(&source)?;
    for item in items {
        if !inventory.iter().any(|s|s.name==item.name && s.size==item.size) { return Err("附加数据流已变化，请刷新后重试".into()); }
    }
    // Keep every selected stream pinned before publishing anything. The sharing
    // check is per stream; a base-file pin alone cannot exclude ADS writers.
    let mut pins=Vec::with_capacity(items.len());
    for (item,(stream,_)) in items.iter().zip(&plan) {
        ctl.check()?;
        let file=super::super::super::file_workspace_compression::stream_shared(&source,stream,false,1)?;
        if file.metadata().map_err(|e|e.to_string())?.len()!=item.size {return Err("附加数据流已变化，请刷新后重试".into());}
        pins.push(file);
    }
    source_ok(&source,revision)?;
    #[cfg(test)] BEFORE_COPY.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run(&source,&pins);}});
    let total=items.iter().try_fold(0u64,|sum,item|sum.checked_add(item.size).ok_or("导出大小超过限制"))?;
    let mut copied=0u64;let mut outputs=Vec::new();let mut buffer=vec![0u8;1024*1024];
    for (index,((item,(_,leaf)),input)) in items.iter().zip(&plan).zip(pins.iter_mut()).enumerate() {
        let target=destination.join(leaf);
        let mut publish_attempted=false;
        let result=with_output_temp(&destination,&guards,|output|{
            ctl.check()?;source_ok(&source,revision)?;
            let mut digest=Sha256::new();let mut count=0u64;
            loop {
                ctl.check()?;
                let n=input.read(&mut buffer).map_err(|e|e.to_string())?;if n==0 {break;}
                count=count.checked_add(n as u64).ok_or("数据流过大")?;
                if count>item.size {return Err("附加数据流在导出时已变化".into());}
                output.write_all(&buffer[..n]).map_err(|e|e.to_string())?;digest.update(&buffer[..n]);
                ctl.set("exporting-streams",Some(if total==0{0}else{(((copied+count) as u128*100)/(total as u128)).min(99) as u8}),&format!("导出 {} / {}：{}",index+1,items.len(),item.name));
                #[cfg(test)] AFTER_CHUNK.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run();}});
            }
            if count!=item.size || input.metadata().map_err(|e|e.to_string())?.len()!=item.size {return Err("附加数据流在导出时已变化".into());}
            output.flush().map_err(|e|e.to_string())?;
            output.as_file_mut().seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
            let mut verified=Sha256::new();let mut length=0u64;
            loop {ctl.check()?;let n=output.as_file_mut().read(&mut buffer).map_err(|e|e.to_string())?;if n==0 {break;}length+=n as u64;verified.update(&buffer[..n]);}
            if length!=count || digest.finalize()!=verified.finalize() {return Err("导出内容核对失败".into());}
            source_ok(&source,revision)?;ctl.check()?;
            publish_attempted=true;drop(publish_output(output,&target,false)?);Ok(())
        });
        if let Err(error)=result {let state=if publish_attempted{"当前项目可能已发布，请检查输出"}else{"当前项目未发布"};return Err(partial_error(format!("{error}\n{state}：{}",target.display()),&outputs,items.len()));}
        copied+=item.size;outputs.push(target.display().to_string());
        #[cfg(test)] AFTER_PUBLISH.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run();}});
    }
    ctl.set("complete",Some(100),"附加数据流已导出");
    Ok(StreamExportReport{destination:destination.display().to_string(),summary:format!("已导出 {} 个附加数据流，来源已保留。",outputs.len()),outputs})
}

#[cfg(test)]
thread_local! {
    static BEFORE_COPY:std::cell::RefCell<Option<Box<dyn FnOnce(&File,&[File])>>>=const{std::cell::RefCell::new(None)};
    static AFTER_CHUNK:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};
    static AFTER_PUBLISH:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};
}

#[cfg(test)] mod tests {
    use super::*;
    struct Fixture { _temp:tempfile::TempDir, source:PathBuf, dest:PathBuf }
    impl Fixture {
        fn new(directory:bool)->Self {
            let temp=tempfile::tempdir().unwrap();let source=temp.path().join("所属项目");let dest=temp.path().join("out");fs::create_dir(&dest).unwrap();
            if directory {fs::create_dir(&source).unwrap();fs::write(source.join("child"),b"child untouched").unwrap();}else{fs::write(&source,b"main untouched").unwrap();}
            fs::write(format!("{}:一.bin",source.display()),vec![0x45;1024*1024+31]).unwrap();
            fs::write(format!("{}:empty",source.display()),b"").unwrap();fs::write(format!("{}:other",source.display()),b"other untouched").unwrap();
            Self{_temp:temp,source,dest}
        }
        fn plan(&self)->(String,Vec<StreamExportItem>){
            let list=list_streams(self.source.to_str().unwrap()).unwrap();
            (list.revision,list.streams.into_iter().filter(|s|s.name!="other").map(|s|StreamExportItem{name:s.name.clone(),size:s.size,output_name:s.name}).collect())
        }
        fn run(&self,revision:&str,items:&[StreamExportItem],opts:&EngineOptions)->Result<StreamExportReport,String>{export_streams(self.source.to_str().unwrap(),revision,items,self.dest.to_str().unwrap(),opts)}
        fn proof(&self)->[u8;32]{let file=File::options().read(true).share_mode(7).custom_flags(0x02200000).open(&self.source).unwrap();streams(&file,None,&JobControl::new()).unwrap()}
    }
    fn security(path:&Path)->Vec<u8>{
        use std::os::windows::ffi::OsStrExt;
        #[link(name="advapi32")]unsafe extern "system"{fn GetFileSecurityW(path:*const u16,info:u32,data:*mut u8,size:u32,needed:*mut u32)->i32;}
        let path=path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();let mut n=0;unsafe{GetFileSecurityW(path.as_ptr(),7,std::ptr::null_mut(),0,&mut n);}let mut data=vec![0;n as usize];assert_ne!(unsafe{GetFileSecurityW(path.as_ptr(),7,data.as_mut_ptr(),n,&mut n)},0);data
    }
    fn put_ea(path:&Path){
        let name=b"QING.EXPORT";let value=b"keep EA";let mut data=vec![0u8;8];data[5]=name.len() as u8;data[6..8].copy_from_slice(&(value.len() as u16).to_le_bytes());data.extend(name);data.push(0);data.extend(value);
        let file=File::options().write(true).share_mode(7).custom_flags(0x02200000).open(path).unwrap();
        let mut writer=Backup{file:&file,context:std::ptr::null_mut(),writing:true};let mut header=vec![0u8;20];header[..4].copy_from_slice(&2u32.to_le_bytes());header[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes());writer.write(&header).unwrap();writer.write(&data).unwrap();
    }
    #[test] fn file_and_directory_single_multi_empty_preserve_source_payload_ea_acl_and_times(){
        for directory in [false,true]{
            let f=Fixture::new(directory);put_ea(&f.source);
            let before=f.proof();let acl=security(&f.source);let meta=fs::metadata(&f.source).unwrap();let (revision,items)=f.plan();
            let result=f.run(&revision,&items,&EngineOptions::default()).unwrap();assert_eq!(result.outputs.len(),2);
            for item in &items {assert_eq!(fs::read(f.dest.join(&item.output_name)).unwrap(),fs::read(format!("{}:{}",f.source.display(),item.name)).unwrap());}
            assert_eq!(f.proof(),before);assert_eq!(security(&f.source),acl);let after=fs::metadata(&f.source).unwrap();
            assert_eq!(after.creation_time(),meta.creation_time());assert_eq!(after.last_write_time(),meta.last_write_time());assert_eq!(after.file_attributes(),meta.file_attributes());
            let (revision,mut one)=f.plan();one.truncate(1);one[0].output_name="用户指定.txt".into();assert_eq!(f.run(&revision,&one,&EngineOptions::default()).unwrap().outputs.len(),1);
        }
    }
    #[test] fn typed_export_129_streams_and_request_validation(){
        let f=Fixture::new(false);for i in 0..129 {fs::write(format!("{}:s{i:03}",f.source.display()),b"").unwrap();}
        let list=crate::operations::execute(serde_json::from_value(serde_json::json!({"operation":"fs-streams","path":f.source})).unwrap()).unwrap();
        let selected=list["streams"].as_array().unwrap().iter().filter(|s|s["name"].as_str().unwrap().starts_with('s')).map(|s|serde_json::json!({"name":s["name"],"size":s["size"],"output_name":s["name"]})).collect::<Vec<_>>();assert_eq!(selected.len(),129);
        let body=serde_json::json!({"operation":"fs-export-streams","path":f.source,"dest":f.dest,"stream_revision":list["revision"],"stream_exports":selected});
        let mut missing=body.clone();missing.as_object_mut().unwrap().remove("stream_revision");assert!(crate::operations::execute(serde_json::from_value(missing).unwrap()).is_err());
        let mut invalid=body.clone();invalid["stream_exports"][0]["extra"]=true.into();assert!(serde_json::from_value::<crate::operations::Request>(invalid).is_err());
        let result=crate::operations::execute(serde_json::from_value(body).unwrap()).unwrap();assert_eq!(result["outputs"].as_array().unwrap().len(),129);assert_eq!(fs::read_dir(&f.dest).unwrap().count(),129);
        for i in 0..129 {assert_eq!(fs::read(f.dest.join(format!("s{i:03}"))).unwrap(),b"");assert_eq!(fs::read(format!("{}:s{i:03}",f.source.display())).unwrap(),b"");}
    }
    #[test] fn same_size_content_changed_before_pinning_is_current_content_not_a_snapshot(){
        let f=Fixture::new(false);let (revision,items)=f.plan();let old_time=fs::metadata(&f.source).unwrap().last_write_time();
        let item=items.iter().find(|s|s.name=="一.bin").unwrap().clone();let current=vec![0x62;item.size as usize];fs::write(format!("{}:一.bin",f.source.display()),&current).unwrap();
        #[link(name="kernel32")]unsafe extern "system"{fn SetFileTime(file:Handle,creation:*const u64,access:*const u64,write:*const u64)->i32;}
        let metadata=File::options().access_mode(0x100).share_mode(7).open(&f.source).unwrap();assert_ne!(unsafe{SetFileTime(metadata.as_raw_handle(),std::ptr::null(),std::ptr::null(),&old_time)},0);drop(metadata);
        assert_eq!(list_streams(f.source.to_str().unwrap()).unwrap().revision,revision);
        f.run(&revision,&[item],&EngineOptions::default()).unwrap();assert_eq!(fs::read(f.dest.join("一.bin")).unwrap(),current);
    }
    #[test] fn selected_stream_pins_block_existing_and_later_payload_writers_but_allow_metadata_writers(){
        let f=Fixture::new(false);let ads=PathBuf::from(format!("{}:一.bin",f.source.display()));let (revision,items)=f.plan();
        let mut writer=File::options().write(true).share_mode(7).open(&ads).unwrap();
        assert!(f.run(&revision,&items,&EngineOptions::default()).is_err());writer.write_all(b"SAME").unwrap();drop(writer);
        assert_eq!(fs::read_dir(&f.dest).unwrap().count(),0);
        let (revision,items)=f.plan();let later=ads.clone();let base=f.source.clone();
        BEFORE_COPY.with(|h|*h.borrow_mut()=Some(Box::new(move|_,_|{
            assert!(File::options().write(true).share_mode(7).open(&later).is_err());
            assert!(fs::remove_file(&later).is_err());assert!(fs::rename(&base,base.with_extension("renamed")).is_err());
            let metadata_writer=File::options().access_mode(0x100).share_mode(7).open(&base).unwrap();drop(metadata_writer);
        })));
        f.run(&revision,&items,&EngineOptions::default()).unwrap();assert_eq!(&fs::read(f.dest.join("一.bin")).unwrap()[..4],b"SAME");
    }
    #[test] fn stale_identity_membership_size_and_metadata_changes_are_rejected(){
        let f=Fixture::new(false);let (revision,items)=f.plan();let moved=f.source.with_extension("original");fs::rename(&f.source,&moved).unwrap();fs::write(&f.source,b"replacement").unwrap();assert!(f.run(&revision,&items,&EngineOptions::default()).is_err());fs::remove_file(&f.source).unwrap();fs::rename(moved,&f.source).unwrap();
        let (revision,mut items)=f.plan();items[0].size+=1;assert!(f.run(&revision,&items,&EngineOptions::default()).is_err());
        let (revision,items)=f.plan();fs::remove_file(format!("{}:empty",f.source.display())).unwrap();assert!(f.run(&revision,&items,&EngineOptions::default()).is_err());
        let (revision,items)=f.plan();let base=f.source.clone();BEFORE_COPY.with(|h|*h.borrow_mut()=Some(Box::new(move|_,_|{
            let writer=File::options().access_mode(0x100).share_mode(7).open(base).unwrap();#[link(name="kernel32")]unsafe extern "system"{fn SetFileTime(file:Handle,creation:*const u64,access:*const u64,write:*const u64)->i32;}let time=123456789000000000u64;assert_ne!(unsafe{SetFileTime(writer.as_raw_handle(),std::ptr::null(),std::ptr::null(),&time)},0);
        })));
        assert!(f.run(&revision,&items,&EngineOptions::default()).unwrap_err().contains("已变化"));assert_eq!(fs::read_dir(&f.dest).unwrap().count(),0);
    }
    #[test] fn invalid_names_mappings_options_and_missing_receipt_never_publish(){
        let f=Fixture::new(false);let (revision,items)=f.plan();
        for name in ["", ".", "..", "a/b", "a\\b", "x:ads", "CON", "a.", "a "] {let mut one=items[..1].to_vec();one[0].output_name=name.into();assert!(f.run(&revision,&one,&EngineOptions::default()).is_err(),"{name}");}
        for name in ["", "::$DATA", "a/b", "a\\b", "x:ads", "bad\0"] {let mut one=items[..1].to_vec();one[0].name=name.into();assert!(f.run(&revision,&one,&EngineOptions::default()).is_err());}
        let mut duplicate=items.clone();duplicate[0].output_name="Same".into();duplicate[1].output_name="sAME".into();assert!(f.run(&revision,&duplicate,&EngineOptions::default()).is_err());
        assert!(f.run("",&items,&EngineOptions::default()).is_err());let mut opts=EngineOptions::default();opts.overwrite="overwrite".into();assert!(f.run(&revision,&items,&opts).is_err());assert_eq!(fs::read_dir(&f.dest).unwrap().count(),0);
    }
    #[test] fn cancellation_before_during_and_after_first_publish_only_cleans_owned_output(){
        for phase in 0..3 {
            let f=Fixture::new(false);let (revision,mut items)=f.plan();items.sort_by(|a,b|b.size.cmp(&a.size));let ctl=JobControl::new();let cancel=ctl.clone();let mut opts=EngineOptions::default();opts.control=Some(ctl);
            match phase {0=>cancel.cancel(),1=>AFTER_CHUNK.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel()))),_=>AFTER_PUBLISH.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel())))}
            let error=f.run(&revision,&items,&opts).unwrap_err();assert!(error.contains("operation cancelled"));let expected=usize::from(phase==2);assert_eq!(fs::read_dir(&f.dest).unwrap().count(),expected,"{error}");
            if phase==2{assert!(error.contains("已导出 1 / 2"));assert!(error.contains("当前项目未发布"));assert!(f.dest.join(&items[0].output_name).is_file());}assert_eq!(fs::read(format!("{}:other",f.source.display())).unwrap(),b"other untouched");
        }
    }
    #[test] fn collision_and_reparse_output_preserve_existing_objects_and_partial_result(){
        let f=Fixture::new(false);let (revision,items)=f.plan();fs::write(f.dest.join(&items[1].output_name),b"existing").unwrap();
        let error=f.run(&revision,&items,&EngineOptions::default()).unwrap_err();assert!(error.contains("已导出 1 / 2"),"{error}");assert_eq!(fs::read(f.dest.join(&items[1].output_name)).unwrap(),b"existing");assert_eq!(fs::read_dir(&f.dest).unwrap().count(),2);
        let junction=f._temp.path().join("junction");let response=std::process::Command::new("cmd.exe").args(["/c","mklink","/J"]).arg(&junction).arg(&f.dest).output().unwrap();assert!(response.status.success());
        assert!(export_streams(f.source.to_str().unwrap(),&revision,&items,junction.to_str().unwrap(),&EngineOptions::default()).is_err());fs::remove_dir(junction).unwrap();
        let source_link=f._temp.path().join("source-junction");let target=f._temp.path().join("target");fs::create_dir(&target).unwrap();fs::write(format!("{}:ads",target.display()),b"outside").unwrap();let response=std::process::Command::new("cmd.exe").args(["/c","mklink","/J"]).arg(&source_link).arg(&target).output().unwrap();assert!(response.status.success());
        assert!(export_streams(source_link.to_str().unwrap(),&revision,&items,f.dest.to_str().unwrap(),&EngineOptions::default()).is_err());fs::remove_dir(source_link).unwrap();
    }
}
