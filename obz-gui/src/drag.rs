//! Copy-only native Explorer drag. Sources are retained for asynchronous shell consumers.
use windows::{core::{implement, BOOL, HRESULT, PCWSTR}, Win32::{
    Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK},
    System::{Com::{CoTaskMemFree, IDataObject}, Ole::{DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize, DROPEFFECT, DROPEFFECT_COPY}, SystemServices::{MODIFIERKEYS_FLAGS, MK_LBUTTON}},
    UI::{Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON}, Shell::{SHParseDisplayName, SHCreateShellItemArrayFromIDLists, BHID_DataObject, Common::ITEMIDLIST}},
}};
#[implement(IDropSource)]
struct Source;
impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() { DRAGDROP_S_CANCEL } else if keys.0 & MK_LBUTTON.0 == 0 { DRAGDROP_S_DROP } else { S_OK }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT { DRAGDROP_S_USEDEFAULTCURSORS }
}
struct Ole;
impl Drop for Ole {fn drop(&mut self){unsafe{OleUninitialize()}}}
struct Pidls(Vec<*mut ITEMIDLIST>);
impl Drop for Pidls {fn drop(&mut self){for p in &self.0 {unsafe{CoTaskMemFree(Some((*p).cast()))}}}}
fn data_object(paths:Vec<String>) -> Result<IDataObject,String> {
    unsafe {
        let mut pidls=Pidls(Vec::new());
        for path in paths {
            let wide:Vec<u16>=path.encode_utf16().chain(Some(0)).collect();let mut pidl=std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(wide.as_ptr()),None,&mut pidl,0,None).map_err(|e|e.to_string())?;
            pidls.0.push(pidl);
        }
        let pointers:Vec<*const ITEMIDLIST>=pidls.0.iter().map(|p|*p as *const _).collect();
        let items=SHCreateShellItemArrayFromIDLists(&pointers).map_err(|e|e.to_string())?;
        items.BindToHandler(None,&BHID_DataObject).map_err(|e|e.to_string())
    }
}
fn native_drag(paths: Vec<String>) -> Result<bool,String> {
    let paths=crate::desktop::checked_paths(paths)?;
    unsafe {
        // Never start a delayed drag after the user has released the initiating button.
        if GetAsyncKeyState(VK_LBUTTON.0 as i32) >= 0 {return Ok(false)}
        OleInitialize(None).map_err(|e|e.to_string())?; let _ole=Ole;
        let data=data_object(paths)?;
        let source:IDropSource=Source.into();let mut effect=DROPEFFECT(0);
        let hr=DoDragDrop(&data,&source,DROPEFFECT_COPY,&mut effect);
        hr.ok().map_err(|e|e.to_string())?;
        Ok(hr==DRAGDROP_S_DROP && effect.0&DROPEFFECT_COPY.0!=0)
    }
}
#[tauri::command]
pub async fn drag_paths(app:tauri::AppHandle,paths:Vec<String>) -> Result<bool,String> {
    // OLE must run on the STA UI thread, with its native message loop active.
    let (tx,rx)=std::sync::mpsc::channel();
    app.run_on_main_thread(move||{let _=tx.send(native_drag(paths));}).map_err(|e|e.to_string())?;
    tauri::async_runtime::spawn_blocking(move||rx.recv().map_err(|e|e.to_string())?).await.map_err(|e|e.to_string())?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn drop_source_requires_held_button_and_honors_escape(){
        let source:IDropSource=Source.into();unsafe {
            assert_eq!(source.QueryContinueDrag(true, MK_LBUTTON),DRAGDROP_S_CANCEL);
            assert_eq!(source.QueryContinueDrag(false, MODIFIERKEYS_FLAGS(0)),DRAGDROP_S_DROP);
            assert_eq!(source.QueryContinueDrag(false, MK_LBUTTON),S_OK);
            assert_eq!(source.GiveFeedback(DROPEFFECT_COPY),DRAGDROP_S_USEDEFAULTCURSORS);
        }
    }
    #[test] fn shell_drag_payload_contains_unicode_files_and_directories(){
        use windows::Win32::{System::{Com::{FORMATETC,DVASPECT_CONTENT,TYMED_HGLOBAL},Ole::ReleaseStgMedium},UI::Shell::{DragQueryFileW,HDROP}};
        let temp=tempfile::tempdir().unwrap();let file=temp.path().join("中文 file.txt");let folder=temp.path().join("folder");
        std::fs::write(&file,b"unchanged").unwrap();std::fs::create_dir(&folder).unwrap();
        let paths=crate::desktop::checked_paths(vec![file.to_string_lossy().into_owned(),folder.to_string_lossy().into_owned()]).unwrap();
        unsafe {
            OleInitialize(None).unwrap();let _ole=Ole;
            let data=data_object(paths.clone()).unwrap();
            let format=FORMATETC{cfFormat:15,ptd:std::ptr::null_mut(),dwAspect:DVASPECT_CONTENT.0,lindex:-1,tymed:TYMED_HGLOBAL.0 as u32};
            let mut medium=data.GetData(&format).unwrap();let drop=HDROP(medium.u.hGlobal.0);
            let count=DragQueryFileW(drop,u32::MAX,None);let mut actual=Vec::new();
            for i in 0..count {let len=DragQueryFileW(drop,i,None);let mut buf=vec![0u16;len as usize+1];DragQueryFileW(drop,i,Some(&mut buf));actual.push(String::from_utf16_lossy(&buf[..len as usize]));}
            ReleaseStgMedium(&mut medium);
            assert_eq!(actual,paths);assert_eq!(std::fs::read(file).unwrap(),b"unchanged");
        }
    }

}
