//! One bounded cancellable writer per hash-verification session, not per chunk.
use super::*;
#[cfg(test)]
thread_local! {pub(super) static WRITERS_STARTED:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
pub(super) struct AckWriter {
    sender:Option<mpsc::SyncSender<(u8,Vec<u8>)>>,
    results:Receiver<Result<(),String>>,
    thread:Option<JoinHandle<()>>,
    child:OwnedHandle,
    pending:bool,
}
impl AckWriter {
    pub(super) fn new(session:&Session)->Result<Self,String> {
        let mut pipe=session.input.as_ref().ok_or("worker input closed")?.try_clone().map_err(|e|e.to_string())?;
        let child=session.child.try_clone().map_err(|e|e.to_string())?;
        let (tx,rx)=mpsc::sync_channel::<(u8,Vec<u8>)>(1);let (done,results)=mpsc::sync_channel(1);
        let thread=std::thread::Builder::new().name("verification-ack".into()).spawn(move||{
            while let Ok((kind,bytes))=rx.recv(){
                let result=pipe.write_all(&[kind]).and_then(|_|pipe.write_all(&(bytes.len() as u32).to_le_bytes()))
                    .and_then(|_|pipe.write_all(&bytes)).and_then(|_|pipe.flush()).map_err(|e|e.to_string());
                let failed=result.is_err();if done.send(result).is_err()||failed{break;}
            }
        }).map_err(|e|e.to_string())?;
        #[cfg(test)]
        WRITERS_STARTED.with(|v|v.set(v.get()+1));
        Ok(Self{sender:Some(tx),results,thread:Some(thread),child,pending:false})
    }
    pub(super) fn send(&mut self,kind:u8,body:Vec<u8>,ctl:&JobControl)->Result<(),String>{
        // Every ACK is nonce(16), index(4), offset(8); no secret or payload.
        if body.len()!=28||!matches!(kind,13|15|17)||self.pending{return Err("invalid verification ACK".into());}
        ctl.check()?;self.sender.as_ref().ok_or("ACK writer closed")?.try_send((kind,body)).map_err(|_|"ACK queue unavailable")?;self.pending=true;
        let result=loop{
            if ctl.check().is_err(){unsafe{TerminateProcess(self.child.as_raw_handle(),2);CancelSynchronousIo(self.thread.as_ref().unwrap().as_raw_handle());}}
            match self.results.recv_timeout(Duration::from_millis(25)){
                Ok(result)=>break result,
                Err(mpsc::RecvTimeoutError::Timeout)=>continue,
                Err(_)=>break Err("ACK writer ended without result".into()),
            }
        };
        self.pending=false;ctl.check()?;result
    }
}
impl Drop for AckWriter {
    fn drop(&mut self){
        self.sender.take();
        if let Some(thread)=self.thread.take(){
            if self.pending{unsafe{TerminateProcess(self.child.as_raw_handle(),2);CancelSynchronousIo(thread.as_raw_handle());}}
            let _=thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[link(name="kernel32")]
    extern "system" {fn CreatePipe(read:*mut Handle,write:*mut Handle,security:*mut c_void,size:u32)->i32;}
    fn pipe()->(File,File){let(mut read,mut write)=(std::ptr::null_mut(),std::ptr::null_mut());assert_ne!(unsafe{CreatePipe(&mut read,&mut write,std::ptr::null_mut(),4096)},0);unsafe{(File::from_raw_handle(read),File::from_raw_handle(write))}}
    fn session()->Session{let exe=std::env::current_exe().unwrap().with_file_name("obz-worker.exe");let exe=if exe.is_file(){exe}else{exe.parent().unwrap().parent().unwrap().join("obz-worker.exe")};Session::start_mode(&exe,3).unwrap()}
    #[test]
    fn persistent_ack_writer_serves_16384_chunks_on_one_thread() {
        let mut session=session();let(mut read,write)=pipe();let _original=session.input.replace(write);let mut writer=AckWriter::new(&session).unwrap();
        let handle=writer.thread.as_ref().unwrap().as_raw_handle();
        let reader=std::thread::spawn(move||{for index in 0..16384u64{let(kind,bytes)=read_frame(&mut read,64).unwrap().unwrap();assert_eq!(kind,15);assert_eq!(bytes.len(),28);assert_eq!(u64::from_le_bytes(bytes[20..28].try_into().unwrap()),index*65536);}});
        for index in 0..16384u64{let mut body=vec![42;16];body.extend_from_slice(&0u32.to_le_bytes());body.extend_from_slice(&(index*65536).to_le_bytes());writer.send(15,body,&JobControl::new()).unwrap();assert_eq!(writer.thread.as_ref().unwrap().as_raw_handle(),handle);}
        drop(writer);reader.join().unwrap();let child=session.child.try_clone().unwrap();drop(session);assert_eq!(unsafe{WaitForSingleObject(child.as_raw_handle(),0)},0);
    }
    #[test]
    fn persistent_ack_blocked_pipe_cancels_and_broken_pipe_returns_error() {
        for cancelled in [false,true]{let mut session=session();let(read,mut write)=pipe();if cancelled{write.write_all(&vec![0;4096]).unwrap();}
            let _original=session.input.replace(write);let mut writer=AckWriter::new(&session).unwrap();let ctl=JobControl::new();let cancel=ctl.clone();
            let held_reader=if cancelled{Some(read)}else{drop(read);None};
            let trigger=cancelled.then(||std::thread::spawn(move||{std::thread::sleep(Duration::from_millis(30));cancel.cancel();}));
            let started=std::time::Instant::now();let result=writer.send(15,vec![0;28],&ctl);assert!(result.is_err());assert!(started.elapsed()<Duration::from_secs(5));
            if let Some(trigger)=trigger{trigger.join().unwrap();}else{assert!(ctl.check().is_ok(),"broken pipe must return independently of cancellation");}drop(writer);drop(held_reader);let child=session.child.try_clone().unwrap();drop(session);assert_eq!(unsafe{WaitForSingleObject(child.as_raw_handle(),0)},0);
        }
    }
}
