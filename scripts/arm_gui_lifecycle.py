"""Failure-safe lifetime for the disposable hosted GUI test's registry override."""
from contextlib import contextmanager
import json

@contextmanager
def guarded_run(registry, policy, name, arguments, evidence, receipt):
    state = {'proc': None}
    old = None
    old_known = False
    body_failed = False
    errors = []
    try:
        try:
            old = registry.QueryValueEx(policy, name)
        except FileNotFoundError:
            pass
        old_known = True
        registry.SetValueEx(policy, name, 0, registry.REG_SZ, arguments)
        evidence['debug_override_prior_value_present'] = old is not None
        yield state
    except BaseException as error:
        body_failed = True
        evidence['passed'] = False
        evidence['driver_error'] = repr(error)
        raise
    finally:
        # Process cleanup must never prevent policy restoration or the receipt.
        try:
            proc = state['proc']
            if proc is not None:
                if proc.poll() is None:
                    proc.terminate()
                proc.wait(timeout=15)
                evidence['process_terminal'] = proc.returncode
        except BaseException as error:
            errors.append('process cleanup: ' + repr(error))
        try:
            if old_known:
                if old is None:
                    try:
                        registry.DeleteValue(policy, name)
                    except FileNotFoundError:
                        pass
                else:
                    registry.SetValueEx(policy, name, 0, old[1], old[0])
                evidence['debug_override_restored'] = True
        except BaseException as error:
            evidence['debug_override_restored'] = False
            errors.append('policy restoration: ' + repr(error))
        finally:
            try:
                registry.CloseKey(policy)
            except BaseException as error:
                errors.append('policy handle close: ' + repr(error))
            if errors:
                evidence['passed'] = False
                evidence['cleanup_errors'] = errors
            receipt.write_text(json.dumps(evidence, ensure_ascii=False, indent=2), encoding='utf-8')
        if errors and not body_failed:
            raise RuntimeError('; '.join(errors))
