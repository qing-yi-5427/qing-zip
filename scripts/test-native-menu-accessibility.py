"""Deterministic checks for the native accessibility state-wait logic."""
import unittest
from unittest.mock import patch

import native_menu_accessibility as native


def state(high, animations):
    return native.HighContrast(12, int(high), None), animations


class NativeWaitTests(unittest.TestCase):
    def test_waits_for_delayed_native_values(self):
        samples = iter([state(True, True), state(False, True), state(False, False)])
        with patch.object(native.time, "sleep"):
            actual = native.wait_os(lambda: next(samples), False, False)
        self.assertEqual(actual, {"os_high": False, "os_animations": False})

    def test_timeout_reports_last_real_value(self):
        with self.assertRaisesRegex(RuntimeError, "'os_high': True, 'os_animations': True"):
            native.wait_os(lambda: state(True, True), False, False, timeout=0)

    def test_contrast_change_rechecks_animations(self):
        samples = iter([state(False, True), state(True, False),
                        state(True, False), state(True, True)])
        calls = []
        actual = native.apply_os(lambda: next(samples), lambda *args: calls.append(args[0]), True, True)
        self.assertEqual(calls, [0x0043, 0x1043])
        self.assertEqual(actual, {"os_high": True, "os_animations": True})


if __name__ == "__main__":
    unittest.main()
