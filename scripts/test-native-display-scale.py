"""Exercise the already-200 decision with mocks, not a real display."""
import base64
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import native_display_scale as scale


class ExistingScaleTest(unittest.TestCase):
    def test_existing_200_records_real_observation_without_transition(self):
        native = {'window_dpi': 192, 'awareness': 2, 'window_title': 'qing-zip',
                  'visible_package_windows': 1, 'monitor_pixels': [3840, 2160],
                  'remote_session': False, 'primary_monitor': True}
        calls = []

        def settings(command, **kwargs):
            self.assertEqual(command[command.index('-SelectPercent') + 1], '0')
            Path(command[command.index('-Receipt') + 1]).write_text(
                json.dumps({'current': 200, 'options': [{'percent': 200, 'enabled': True}]}),
                encoding='utf8')
            return SimpleNamespace(returncode=0, stderr='')

        def cdp(method, params=None):
            calls.append(method)
            return {'data': base64.b64encode(b'mocked screenshot').decode()} if method == 'Page.captureScreenshot' else {}

        def js(expression, promise=False):
            if 'devicePixelRatio' in expression:
                return {'dpr': 2, 'width': 900, 'height': 700}
            if "document.querySelector('#oa-menu').hidden" in expression:
                return False
            if 'getBoundingClientRect' in expression:
                return {'hidden': False, 'left': 10, 'top': 10, 'right': 210,
                        'bottom': 310, 'width': 200, 'height': 300, 'items': 7}
            return True

        with TemporaryDirectory() as directory:
            out = Path(directory)
            exe = out / 'qing-zip.exe'
            exe.write_bytes(b'mocked executable')
            with patch.object(scale, 'check_host'), patch.object(scale, 'window_metrics', return_value=lambda: native), patch.object(scale.subprocess, 'run', side_effect=settings), patch.dict(scale.os.environ, {'SystemRoot': str(out)}):
                scale.verify_scale(cdp, js, out, exe, 123)
            result = json.loads((out / 'system-scale.json').read_text(encoding='utf8'))
            self.assertEqual(result['status'], 'PASS_EXISTING_200')
            self.assertFalse(result['transition_tested'])
            self.assertFalse(result['change_attempted'])
            self.assertTrue(result['restored'])
            self.assertEqual(result['observations']['at-200']['native']['window_dpi'], 192)
            self.assertEqual(result['observations']['at-200']['webview']['dpr'], 2)
            self.assertTrue((out / 'menu-at-200.png').is_file())
            self.assertGreaterEqual(calls.count('Page.captureScreenshot'), 2)


if __name__ == '__main__':
    unittest.main()
