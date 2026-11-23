"""Alternate verified R3 and candidate ZIP conflict feedback in a real WebView."""

import argparse
import json
from pathlib import Path
import statistics
import subprocess
import sys


def inspect(receipts):
    values = [receipt["timing"]["failure_card_ms"] for receipt in receipts]
    return {"median": round(statistics.median(values), 3), "min": round(min(values), 3), "max": round(max(values), 3)}


def run(label, spec, out, staged):
    exe, source = spec[0], spec[-1]
    command = [sys.executable, str(Path(__file__).with_name("check-nested-multi-layer.py")),
               "--exe", exe, "--source", source, "--out", str(out)]
    if staged:
        command.extend(["--staged-sha256", *spec[1:-1]])
    else:
        command.extend(["--hardening", spec[1], "--package", spec[2]])
    if label == "candidate":
        command.append("--expect-feedback")
    subprocess.run(command, check=True, timeout=150)
    receipt = json.loads((out / "receipt.json").read_text(encoding="utf-8"))
    assert receipt["status"] == "passed" and receipt["owned_fixture_cleaned"]
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    packaged = ("GUI", "HARDENING_JSON", "PORTABLE_ZIP", "SOURCE_SHA")
    parser.add_argument("--baseline", nargs=4, required=True, metavar=packaged)
    choice = parser.add_mutually_exclusive_group(required=True)
    choice.add_argument("--candidate", nargs=4, metavar=packaged)
    choice.add_argument("--candidate-stage", nargs=5,
                        metavar=("GUI", "GUI_SHA", "CLI_SHA", "WORKER_SHA", "SOURCE_SHA"))
    parser.add_argument("--pairs", type=int, default=5)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    if args.pairs <= 0:
        parser.error("--pairs must be positive")
    args.out.mkdir(parents=True, exist_ok=False)
    pairs = []
    for iteration in range(args.pairs):
        order = ("baseline", "candidate") if iteration % 2 == 0 else ("candidate", "baseline")
        receipts = {}
        for label in order:
            out = args.out / f"{iteration + 1:02d}-{label}"
            staged = label == "candidate" and args.candidate_stage is not None
            spec = args.candidate_stage if staged else getattr(args, label)
            receipts[label] = run(label, spec, out, staged)
        pair = {"iteration": iteration + 1, "order": order, "receipts": receipts}
        pairs.append(pair)
        print(json.dumps({label: receipts[label]["timing"] for label in order}), flush=True)

    batch = {label: [pair["receipts"][label] for pair in pairs] for label in ("baseline", "candidate")}
    paired = [pair["receipts"]["candidate"]["timing"]["failure_card_ms"] -
              pair["receipts"]["baseline"]["timing"]["failure_card_ms"] for pair in pairs]
    candidate_tab = [item["timing"]["correct_middle_tab_ms"] for item in batch["candidate"]]
    summary = {"metric": "trusted Save click capture to the first visible failed task card DOM text",
               "unit": "milliseconds", "compared_pairs": len(pairs),
               "sources": {label: {"git": receipts[0]["source"], "binaries_sha256": receipts[0]["binaries_sha256"]}
                           for label, receipts in batch.items()},
               "failure_card": {label: inspect(receipts) for label, receipts in batch.items()},
               "candidate_minus_baseline_failure_card": {"median": statistics.median(paired),
                                                          "min": min(paired), "max": max(paired)},
               "candidate_correct_middle_tab": {"median": statistics.median(candidate_tab),
                                                "min": min(candidate_tab), "max": max(candidate_tab)},
               "baseline_correct_middle_tab_observations": sum(item["timing"]["correct_middle_tab_ms"] is not None
                                                                for item in batch["baseline"]),
               "raw": [{"iteration": pair["iteration"], "order": pair["order"],
                        "timing": {label: receipt["timing"] for label, receipt in pair["receipts"].items()}}
                       for pair in pairs]}
    (args.out / "timing.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=True), flush=True)


if __name__ == "__main__":
    main()
