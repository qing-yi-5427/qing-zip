"""Fault injection only: no Windows API, real registry, process, or GUI touched."""
import json,tempfile,unittest
from pathlib import Path
from arm_gui_lifecycle import guarded_run

class Registry:
    REG_SZ=1
    def __init__(self,old=None,restore_error=False):self.value=old;self.closed=False;self.restore_error=restore_error;self.sets=0
    def QueryValueEx(self,*_):
        if self.value is None:raise FileNotFoundError()
        return self.value
    def SetValueEx(self,key,name,reserved,kind,value):
        self.sets+=1
        if self.restore_error and self.sets>1:raise PermissionError('restore denied')
        self.value=(value,kind)
    def DeleteValue(self,*_):self.value=None
    def CloseKey(self,*_):self.closed=True
class Process:
    returncode=None
    def __init__(self,fail):self.fail=fail
    def poll(self):return None
    def terminate(self):
        if self.fail=='terminate':raise OSError('terminate injected')
    def wait(self,timeout):
        if self.fail=='wait':raise TimeoutError('wait injected')
        self.returncode=0
class CleanupTests(unittest.TestCase):
    def test_launch_failure_restores_absent_and_existing_values(self):
        for old in [None,('original',1)]:
            with self.subTest(old=old),tempfile.TemporaryDirectory() as d:
                reg=Registry(old);receipt=Path(d)/'receipt.json';e={'passed':False}
                with self.assertRaisesRegex(OSError,'launch injected'):
                    with guarded_run(reg,'fake','fake.exe','debug',e,receipt):raise OSError('launch injected')
                self.assertEqual(reg.value,old);self.assertTrue(reg.closed)
                self.assertTrue(json.loads(receipt.read_text())['debug_override_restored'])
    def test_process_cleanup_errors_still_restore_and_write_receipt(self):
        for failure in ['terminate','wait']:
            with self.subTest(failure=failure),tempfile.TemporaryDirectory() as d:
                reg=Registry(('original',1));receipt=Path(d)/'receipt.json';e={'passed':True}
                with self.assertRaises(RuntimeError):
                    with guarded_run(reg,'fake','fake.exe','debug',e,receipt) as state:state['proc']=Process(failure)
                self.assertEqual(reg.value,('original',1));self.assertTrue(reg.closed)
                result=json.loads(receipt.read_text());self.assertFalse(result['passed']);self.assertTrue(result['debug_override_restored']);self.assertIn('process cleanup',result['cleanup_errors'][0])
    def test_restore_failure_is_recorded_not_claimed_success(self):
        with tempfile.TemporaryDirectory() as d:
            reg=Registry(('original',1),True);receipt=Path(d)/'receipt.json'
            with self.assertRaises(RuntimeError):
                with guarded_run(reg,'fake','fake.exe','debug',{'passed':True},receipt):pass
            result=json.loads(receipt.read_text());self.assertFalse(result['passed']);self.assertFalse(result['debug_override_restored']);self.assertTrue(reg.closed)
    def test_primary_error_remains_visible_if_cleanup_also_fails(self):
        with tempfile.TemporaryDirectory() as d:
            reg=Registry();receipt=Path(d)/'receipt.json'
            with self.assertRaisesRegex(ValueError,'body injected'):
                with guarded_run(reg,'fake','fake.exe','debug',{'passed':False},receipt) as state:
                    state['proc']=Process('wait');raise ValueError('body injected')
            result=json.loads(receipt.read_text());self.assertIn('body injected',result['driver_error']);self.assertTrue(result['cleanup_errors']);self.assertIsNone(reg.value)
if __name__=='__main__':unittest.main()
