"""Make sure incomplete observations cannot pass the ZIP64 desktop feedback check."""
import copy
import json
import unittest
from pathlib import Path

from desktop import verify_task_feedback


class TaskFeedback(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).resolve().with_name("desktop-x64.json")
        self.result = json.loads(path.read_text(encoding="utf8"))["task"]

    def check(self, result):
        verify_task_feedback(result["samples"], result["running_screenshot"], result["completion_action"])

    def test_complete_real_observations(self):
        self.check(self.result)

    def test_incomplete_observations(self):
        original = self.result["samples"]
        decode = [sample for sample in original if sample["phase"] == "验证并解码"]
        for label, samples in {
            "no initial wait": decode + [original[-1]],
            "no decode stage": [original[0], original[-1]],
            "stalled decode": [original[0], decode[0], decode[0], original[-1]],
            "decreasing percentage": [original[0], *reversed(decode), original[-1]],
            "no completion": original[:-1],
        }.items():
            with self.subTest(label=label), self.assertRaises(RuntimeError):
                self.check(dict(self.result, samples=samples))

    def test_screenshot_and_output_action(self):
        for modification in ({"running_screenshot": False}, {"completion_action": ""}):
            with self.subTest(modification=modification), self.assertRaises(RuntimeError):
                self.check(dict(self.result, **modification))

    def test_completion_percentage(self):
        result = copy.deepcopy(self.result)
        result["samples"][-1]["percent"] = "99"
        with self.assertRaises(RuntimeError):
            self.check(result)


if __name__ == "__main__":
    unittest.main()
