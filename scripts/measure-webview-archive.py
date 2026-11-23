"""Measure archive browsing in packaged Tauri windows, using disposable profiles."""

import argparse
import base64
import ctypes
from ctypes import wintypes as wt
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import time
import urllib.request

import win32gui
import win32process
from pywintypes import error as WindowError
from websockets.sync.client import connect


class Process(ctypes.Structure):
    _fields_ = [("size", wt.DWORD), ("usage", wt.DWORD), ("pid", wt.DWORD),
                ("heap", ctypes.c_size_t), ("module", wt.DWORD), ("threads", wt.DWORD),
                ("parent", wt.DWORD), ("priority", wt.LONG), ("flags", wt.DWORD),
                ("name", wt.WCHAR * 260)]


def process_entries():
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = wt.HANDLE
    kernel.Process32FirstW.argtypes = [wt.HANDLE, ctypes.POINTER(Process)]
    kernel.Process32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(Process)]
    kernel.CloseHandle.argtypes = [wt.HANDLE]
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    listing = {}
    try:
        p = Process()
        p.size = ctypes.sizeof(p)
        present = kernel.Process32FirstW(snapshot, ctypes.byref(p))
        while present:
            listing[p.pid] = (p.parent, p.name.lower())
            present = kernel.Process32NextW(snapshot, ctypes.byref(p))
    finally:
        kernel.CloseHandle(snapshot)
    return listing


def process_memory(root):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    kernel.OpenProcess.restype = wt.HANDLE
    kernel.K32GetProcessMemoryInfo.argtypes = [wt.HANDLE, ctypes.POINTER(ProcessMemoryCounters), wt.DWORD]
    kernel.CloseHandle.argtypes = [wt.HANDLE]
    listing = process_entries()
    if root not in listing:
        raise RuntimeError("Measured native application has exited")
    members = {root}
    while True:
        descendants = {pid for pid, (parent, _) in listing.items() if parent in members}
        if descendants <= members:
            break
        members.update(descendants)
    sampled = []
    for pid in members:
        handle = kernel.OpenProcess(0x0400 | 0x0010, False, pid)
        if not handle:
            continue
        try:
            data = ProcessMemoryCounters()
            data.size = ctypes.sizeof(data)
            if kernel.K32GetProcessMemoryInfo(handle, ctypes.byref(data), data.size):
                sampled.append((pid, listing[pid][1], data.private_commit, data.working_set))
        finally:
            kernel.CloseHandle(handle)
    webviews = sum(name == "msedgewebview2.exe" for _, name, _, _ in sampled)
    if not webviews or not any(pid == root for pid, _, _, _ in sampled):
        raise RuntimeError("Expected native application and WebView2 process descendants")
    return {"sampled_processes": len(sampled), "unsampled_processes": len(members) - len(sampled),
            "webview_processes": webviews, "private_commit_bytes": sum(p[2] for p in sampled),
            "working_set_bytes": sum(p[3] for p in sampled)}


STATE = """(() => {
  const viewport = document.querySelector('#oa-archive-scroll');
  const table = document.querySelector('#qa-rows')?.closest('table');
  if (!viewport || !table) return null;
  const bounds = viewport.getBoundingClientRect();
  const header = table.querySelector('thead')?.getBoundingClientRect();
  const rows = [...table.querySelectorAll('tr[data-row]')];
  const visible = rows.filter(row => {
    const r = row.getBoundingClientRect();
    return r.height > 0 && r.bottom > Math.max(bounds.top, header?.bottom ?? 0) && r.top < bounds.bottom;
  });
  return {
    first: visible[0]?.dataset.row ?? null,
    mounted: rows.length,
    visible: visible.length,
    logical: Number(table.getAttribute('aria-rowcount')) - 1,
    sort: table.querySelector('th[data-column=name]')?.getAttribute('aria-sort'),
    scroll_top: viewport.scrollTop,
    device_pixel_ratio: devicePixelRatio,
    viewport_css_pixels: [Math.round(bounds.width), Math.round(bounds.height)],
    document_visible: document.visibilityState === 'visible',
  };
})()"""


class Window:
    def __init__(self, exe, archive, isolated, allow_unfocused):
        if any(name in {"qing-zip.exe", "obz-gui.exe"} for _, name in process_entries().values()):
            raise RuntimeError("Another qing-zip instance is active; its windows were left untouched")
        self.socket = None
        self.sequence = 0
        self.hwnd = None
        self.allow_unfocused = allow_unfocused
        self.profile = isolated / "webview"
        self.local = isolated / "local"
        self.local.mkdir(parents=True, exist_ok=True)
        for previous in self.profile.rglob("DevToolsActivePort"):
            previous.unlink()
        env = dict(os.environ, LOCALAPPDATA=str(self.local), WEBVIEW2_USER_DATA_FOLDER=str(self.profile),
                   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=(
                       "--remote-debugging-port=0 --remote-debugging-address=127.0.0.1"))
        self.started = time.perf_counter()
        self.proc = subprocess.Popen([exe, archive], cwd=isolated, env=env)

    def connect(self):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and self.proc.poll() is None:
            port_file = next(self.profile.rglob("DevToolsActivePort"), None)
            if port_file:
                try:
                    port = int(port_file.read_text().splitlines()[0])
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=1) as request:
                        pages = json.load(request)
                    page = next(p for p in pages if p.get("type") == "page" and "tauri.localhost" in p["url"])
                    self.socket = connect(page["webSocketDebuggerUrl"], max_size=2 * 1024 * 1024,
                                          open_timeout=2, close_timeout=1)
                    self.hwnd = self.native_window()
                    if self.hwnd and not self.allow_unfocused and not self.is_foreground():
                        try:
                            win32gui.SetForegroundWindow(self.hwnd)
                        except WindowError:
                            pass
                    return
                except (OSError, ValueError, IndexError, StopIteration):
                    pass
            time.sleep(.025)
        raise RuntimeError("Packaged WebView2 did not expose its Tauri document over loopback")

    def call(self, method, **params):
        self.sequence += 1
        self.socket.send(json.dumps({"id": self.sequence, "method": method, "params": params}))
        while True:
            answer = json.loads(self.socket.recv(timeout=10))
            if answer.get("id") == self.sequence:
                if "error" in answer:
                    raise RuntimeError(str(answer["error"]))
                return answer["result"]

    def js(self, expression):
        evaluated = self.call("Runtime.evaluate", expression=expression, returnByValue=True)
        if "exceptionDetails" in evaluated:
            raise RuntimeError(str(evaluated["exceptionDetails"]))
        return evaluated["result"].get("value")

    def native_window(self):
        windows = []
        def collect(hwnd, _):
            if win32process.GetWindowThreadProcessId(hwnd)[1] == self.proc.pid and win32gui.IsWindowVisible(hwnd):
                windows.append(hwnd)
        win32gui.EnumWindows(collect, None)
        return next(iter(windows), None)

    def is_foreground(self):
        self.hwnd = self.hwnd or self.native_window()
        foreground = win32gui.GetForegroundWindow()
        return bool(self.hwnd and foreground and win32process.GetWindowThreadProcessId(foreground)[1] == self.proc.pid
                    and not win32gui.IsIconic(self.hwnd))

    def active_view(self):
        self.hwnd = self.hwnd or self.native_window()
        return bool(self.hwnd and win32gui.IsWindowVisible(self.hwnd) and not win32gui.IsIconic(self.hwnd)
                    and (self.allow_unfocused or self.is_foreground()))

    def wait(self, predicate):
        deadline = time.monotonic() + 15
        state = None
        while time.monotonic() < deadline:
            state = self.js(STATE)
            if state and state["document_visible"] and self.active_view() and predicate(state):
                return {**state, "win32_foreground": self.is_foreground()}
            time.sleep(.012)
        raise RuntimeError(f"Expected visible foreground content (foreground={self.is_foreground()}), last state: {state}")

    def position(self, selector):
        return self.js(f"""(() => {{
          const element = document.querySelector({json.dumps(selector)});
          if (!element || element.hidden) return null;
          const r = element.getBoundingClientRect();
          const x = r.left + r.width / 2, y = r.top + r.height / 2;
          const top = document.elementFromPoint(x, y);
          return r.height && top && (top === element || element.contains(top)) ? [x, y] : null;
        }})()""")

    def click(self, selector, count=1):
        point = self.position(selector)
        if point is None or not self.active_view():
            raise RuntimeError(f"Control is not active and visible: {selector}")
        for clicks in range(1, count + 1):
            for event in ("mousePressed", "mouseReleased"):
                self.call("Input.dispatchMouseEvent", type=event, x=point[0], y=point[1],
                          button="left", buttons=int(event == "mousePressed"), clickCount=clicks)

    def wheel(self, pixels):
        x, y = self.js("""(() => {const r=document.querySelector('#oa-archive-scroll').getBoundingClientRect();
                                   return [r.left+r.width/2,r.top+r.height/2]})()""")
        if not self.active_view():
            raise RuntimeError("Measured view is unavailable before scroll")
        self.call("Input.dispatchMouseEvent", type="mouseWheel", x=x, y=y, deltaX=0, deltaY=pixels)

    def search(self, text):
        self.click("#qa-search")
        for event in ("rawKeyDown", "keyUp"):
            self.call("Input.dispatchKeyEvent", type=event, key="a", code="KeyA",
                      modifiers=2, windowsVirtualKeyCode=65)
        self.call("Input.insertText", text=text)

    def capture(self, screenshot):
        result = self.call("Page.captureScreenshot", format="png", captureBeyondViewport=False)
        screenshot.write_bytes(base64.b64decode(result["data"]))

    def close(self):
        if self.socket is not None:
            self.socket.close()
        if self.proc.poll() is None:
            hwnd = self.hwnd or self.native_window()
            if hwnd:
                win32gui.PostMessage(hwnd, 0x0010, 0, 0)
            try:
                self.proc.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.proc.terminate()
                self.proc.wait(timeout=5)


def detect(window, action, predicate):
    started = time.perf_counter()
    action()
    state = window.wait(predicate)
    return {"ms": round((time.perf_counter() - started) * 1000, 3), **state}


def trial(exe, archive, count, isolated, allow_unfocused, screenshot=None):
    window = Window(exe, archive, isolated, allow_unfocused)
    wide_selector = 'button[data-entry="wide/"]'
    try:
        window.connect()
        first = window.wait(lambda state: state["first"] == f"group-{count // 1000:03}/"
                            and state["logical"] == count // 1000 + 1)
        root = {"ms": round((time.perf_counter() - window.started) * 1000, 3), **first}
        root["memory"] = process_memory(window.proc.pid)
        if screenshot:
            window.capture(screenshot)
        if window.position(wide_selector) is None:
            window.wheel(6000)
            window.wait(lambda s: s["scroll_top"] > 0 and window.position(wide_selector) is not None)
        wide = detect(window, lambda: window.click(wide_selector, count=2),
                      lambda s: s["first"] == "wide/member-000000.txt" and s["logical"] == count // 2)
        search = detect(window, lambda: window.search("target-needle"),
                        lambda s: s["first"] == "wide/target-needle.txt" and s["logical"] == 1)
        outside = f"member-{count-1:06}"
        window.search(outside)
        window.wait(lambda s: s["logical"] == 0)
        whole = detect(window, lambda: window.click("#oa-search-whole"),
                       lambda s: s["logical"] == 1 and s["first"] == f"group-{(count-1)//500:03}/nested/{outside}.txt")
        window.click("#oa-search-clear")
        window.wait(lambda s: s["logical"] == count // 2 and s["first"] == "wide/member-000000.txt")
        sort = detect(window, lambda: window.click('button[data-sort="name"]'),
                      lambda s: s["sort"] == "descending" and s["first"] == "wide/target-needle.txt")
        scroll = detect(window, lambda: window.wheel(1230),
                        lambda s: s["scroll_top"] > 100 and (s["first"] or "").startswith("wide/member-"))
        scroll["memory"] = process_memory(window.proc.pid)
        for row in (wide, sort, scroll):
            if not 0 < row["visible"] <= row["mounted"] <= row["visible"] + 20:
                raise RuntimeError(f"Wide view was not bounded by the viewport: {row}")
        return {"open": root, "wide": wide, "search": search, "whole": whole, "sort": sort,
                "scroll": scroll, "webview": window.call("Browser.getVersion")["product"]}
    finally:
        window.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--rounds", type=int, choices=(3, 4, 5), default=3)
    parser.add_argument("--allow-unfocused", action="store_true",
                        help="Measure a visible, non-minimized WebView and record actual Win32 focus")
    parser.add_argument("--screenshots", type=Path, help="New directory for warm-up first-screen evidence")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if args.screenshots:
        args.screenshots.mkdir()
    exes = {"baseline": args.baseline.resolve(strict=True), "candidate": args.candidate.resolve(strict=True)}
    evidence = {"gui_sha256": {label: sha256(exe.read_bytes()) for label, exe in exes.items()},
                "allow_unfocused": args.allow_unfocused, "cases": []}
    with tempfile.TemporaryDirectory(prefix="qing-native-browse-") as temporary:
        work = Path(temporary).resolve()
        for count in (10_000, 50_000):
            archive = work / f"wide-{count}.zip"
            fixture(archive, count)
            original = sha256(archive.read_bytes())
            rounds = []
            for index in range(args.rounds + 1):
                order = ["baseline", "candidate"] if index % 2 else ["candidate", "baseline"]
                results = {label: trial(exes[label], archive, count, work / label, args.allow_unfocused,
                                        args.screenshots / f"{count}-{label}.png"
                                        if args.screenshots and index == 0 else None)
                           for label in order}
                if sha256(archive.read_bytes()) != original:
                    raise RuntimeError("Archive content changed")
                print(f"{count} entries, round {index}: visible rows, navigation, filter, sort and scroll PASS", flush=True)
                rounds.append({"order": order, **results})
            evidence["cases"].append({"entries": count, "archive_sha256": original,
                                      "warmup": rounds[0], "samples": rounds[1:]})
    with args.receipt.open("x", encoding="utf-8") as output:
        print(json.dumps(evidence, indent=2), file=output)


browse = runpy.run_path(str(Path(__file__).with_name("benchmark-archive-browse.py")))
fixture, sha256 = browse["fixture"], browse["sha256"]
ProcessMemoryCounters = browse["ProcessMemoryCounters"]

if __name__ == "__main__":
    main()
