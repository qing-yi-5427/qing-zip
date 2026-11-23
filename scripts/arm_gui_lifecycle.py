"""Failure-safe lifetime for the disposable hosted GUI test's registry override."""
from contextlib import contextmanager
import json


@contextmanager
def hosted_debug_session(executable, receipt):
    """Connect only to the named GUI on the disposable runner's loopback interface."""
    import os
    import socket
    import winreg

    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("The per-executable WebView2 test override requires a disposable hosted runner")
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    arguments = f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
    key = winreg.CreateKeyEx(winreg.HKEY_LOCAL_MACHINE,
                             r"SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments",
                             0, winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY)
    evidence = {"passed": False, "executable": executable.name, "port": port}
    with guarded_run(winreg, key, executable.name, arguments, evidence, receipt) as lifetime:
        lifetime.update(port=port, arguments=arguments)
        yield lifetime
        evidence["passed"] = True

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
