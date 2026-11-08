"""Separate resource profiles using Windows job accounting, not benchmark timing.

Includes CLI and inherited child processes. PeakJobMemoryUsed is committed memory,
not resident set; I/O counters include cache and are not physical disk traffic.
The job is assigned immediately after spawning; very early activity before that
assignment may be missed. Ordinary benchmark timings remain in benchmark.py.
"""
import ctypes as c
from ctypes import wintypes as w
import hashlib, json, subprocess, time
from pathlib import Path


class IO(c.Structure):
    _fields_ = [(key, c.c_ulonglong) for key in ['read_ops','write_ops','other_ops','read_bytes','write_bytes','other_bytes']]
class Accounting(c.Structure):
    _fields_ = [(key,c.c_longlong) for key in ['user','kernel','period_user','period_kernel']] + [(key,w.DWORD) for key in ['faults','processes','active','terminated']]
class Usage(c.Structure):
    _fields_ = [('cpu',Accounting),('io',IO)]
class Limits(c.Structure):
    _fields_ = [('process_time',c.c_longlong),('job_time',c.c_longlong),('flags',w.DWORD),('min_working',c.c_size_t),('max_working',c.c_size_t),('process_count',w.DWORD),('affinity',c.c_size_t),('priority',w.DWORD),('scheduling',w.DWORD)]
class Extended(c.Structure):
    _fields_ = [('limits',Limits),('io',IO),('process_limit',c.c_size_t),('job_limit',c.c_size_t),('peak_process_commit',c.c_size_t),('peak_job_commit',c.c_size_t)]

k=c.WinDLL('kernel32',use_last_error=True)
k.CreateJobObjectW.argtypes=[c.c_void_p,w.LPCWSTR]; k.CreateJobObjectW.restype=w.HANDLE
k.OpenProcess.argtypes=[w.DWORD,w.BOOL,w.DWORD]; k.OpenProcess.restype=w.HANDLE
k.AssignProcessToJobObject.argtypes=[w.HANDLE,w.HANDLE]; k.AssignProcessToJobObject.restype=w.BOOL
k.SetInformationJobObject.argtypes=[w.HANDLE,c.c_int,c.c_void_p,w.DWORD]; k.SetInformationJobObject.restype=w.BOOL
k.QueryInformationJobObject.argtypes=[w.HANDLE,c.c_int,c.c_void_p,w.DWORD,c.c_void_p]; k.QueryInformationJobObject.restype=w.BOOL
k.CloseHandle.argtypes=[w.HANDLE]; k.CloseHandle.restype=w.BOOL

def check(ok):
    if not ok: raise c.WinError(c.get_last_error())

