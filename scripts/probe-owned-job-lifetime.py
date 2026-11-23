"""Probe Windows Job Object lifetime using only owned Python child processes.

This is a mechanism experiment, not a test of the qing-zip executable.
"""
import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

kernel = ctypes.WinDLL('kernel32', use_last_error=True)
kernel.CreateJobObjectW.argtypes = [wintypes.LPVOID, wintypes.LPCWSTR]
kernel.CreateJobObjectW.restype = wintypes.HANDLE
kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]
kernel.SetInformationJobObject.restype = wintypes.BOOL
kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
kernel.AssignProcessToJobObject.restype = wintypes.BOOL
kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel.OpenProcess.restype = wintypes.HANDLE
kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
kernel.WaitForSingleObject.restype = wintypes.DWORD
kernel.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
kernel.TerminateProcess.restype = wintypes.BOOL
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
kernel.CloseHandle.restype = wintypes.BOOL

class Basic(ctypes.Structure):
    _fields_ = [('process_time',ctypes.c_int64),('job_time',ctypes.c_int64),('flags',wintypes.DWORD),
                ('min',ctypes.c_size_t),('max',ctypes.c_size_t),('active',wintypes.DWORD),
                ('affinity',ctypes.c_size_t),('priority',wintypes.DWORD),('scheduling',wintypes.DWORD)]
class Io(ctypes.Structure):
    _fields_ = [(name,ctypes.c_uint64) for name in ('read','write','other','read_bytes','write_bytes','other_bytes')]
class Extended(ctypes.Structure):
    _fields_ = [('basic',Basic),('io',Io),('process_memory',ctypes.c_size_t),('job_memory',ctypes.c_size_t),
                ('peak_process',ctypes.c_size_t),('peak_job',ctypes.c_size_t)]

def writer(path):
    with open(path,'wb',buffering=0) as out:
        while True:
            out.write(b'owned fixture\n'*512)
            time.sleep(.02)

def controller(path):
    job = kernel.CreateJobObjectW(None,None)
    if not job: raise ctypes.WinError(ctypes.get_last_error())
    limits = Extended(); limits.basic.flags = 0x2000  # KILL_ON_JOB_CLOSE
    if not kernel.SetInformationJobObject(job,9,ctypes.byref(limits),ctypes.sizeof(limits)):
        raise ctypes.WinError(ctypes.get_last_error())
    child = subprocess.Popen([sys.executable,__file__,'writer',path],creationflags=subprocess.CREATE_NO_WINDOW)
    if not kernel.AssignProcessToJobObject(job,int(child._handle)):
        code = ctypes.get_last_error()
        child.kill(); child.wait(); raise ctypes.WinError(code)
    print(json.dumps({'child_pid':child.pid}),flush=True)
    child.wait()
    kernel.CloseHandle(job)

def main():
    with tempfile.TemporaryDirectory(prefix='qz-owned-job-probe-') as tmp:
        root=Path(tmp)
        for number in range(3):
            controlled = root/f'controlled-{number}.bin'; independent = root/f'independent-{number}.bin'
            other=subprocess.Popen([sys.executable,__file__,'writer',str(independent)],creationflags=subprocess.CREATE_NO_WINDOW)
            parent=subprocess.Popen([sys.executable,__file__,'controller',str(controlled)],stdout=subprocess.PIPE,text=True,creationflags=subprocess.CREATE_NO_WINDOW)
            handle=None
            try:
                pid=json.loads(parent.stdout.readline())['child_pid']
                handle=kernel.OpenProcess(0x100001,False,pid)  # SYNCHRONIZE | TERMINATE on this exact child
                if not handle: raise ctypes.WinError(ctypes.get_last_error())
                deadline=time.monotonic()+10
                while time.monotonic()<deadline and (not controlled.exists() or not independent.exists() or controlled.stat().st_size==0 or independent.stat().st_size==0):
                    time.sleep(.01)
                assert controlled.stat().st_size>0 and independent.stat().st_size>0 and other.poll() is None
                at=time.monotonic(); parent.kill(); parent.wait(timeout=10)
                wait=kernel.WaitForSingleObject(handle,5000)
                assert wait==0, f'owned child did not exit: wait={wait}'
                elapsed=time.monotonic()-at
                size=controlled.stat().st_size; other_size=independent.stat().st_size
                time.sleep(.3)
                assert controlled.stat().st_size==size and independent.stat().st_size>other_size and other.poll() is None
                print(json.dumps({'round':number+1,'owned_child_exit_seconds':round(elapsed,4),
                                  'controlled_bytes_at_exit':size,'controlled_growth_after_300ms':0,
                                  'independent_growth_after_300ms':independent.stat().st_size-other_size,
                                  'independent_alive':True}),flush=True)
            finally:
                if parent.poll() is None: parent.kill(); parent.wait(timeout=10)
                if handle:
                    if kernel.WaitForSingleObject(handle,0)==0x102:
                        kernel.TerminateProcess(handle,1)
                        kernel.WaitForSingleObject(handle,5000)
                    kernel.CloseHandle(handle)
                if other.poll() is None: other.kill(); other.wait(timeout=10)

if __name__=='__main__':
    if len(sys.argv)>1 and sys.argv[1]=='writer': writer(sys.argv[2])
    elif len(sys.argv)>1 and sys.argv[1]=='controller': controller(sys.argv[2])
    else: main()
