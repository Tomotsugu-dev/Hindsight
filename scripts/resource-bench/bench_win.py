#!/usr/bin/env python3
"""Resource benchmark for the installed Hindsight (Windows only).

Same phases as bench.py: baseline (Hindsight quit), then for each setting
(screenshots + OCR, screenshots only, both off): warm-up, window open, window
closed. Hindsight is quit through its tray menu, its two settings are changed in
the database while it is not running, and the original values are put back at the
end. A background thread fakes a user: it switches between Notepad, an Explorer
folder and Calculator, types into Notepad and moves the mouse.
Do not use the computer during the run; a full run takes about 45 minutes.

    uv run --with psutil --with pywin32 --with uiautomation python bench_win.py

Writes samples.csv (one row per process per sample) and phases.json to run/;
`python analyze.py` then writes summary.json there.
Memory (footprint) is the private working set, Task Manager's "Memory" column;
commit holds private bytes. Windows has no per-process energy counter, so
energy_nj stays empty. GPU is the summed utilization of the process's GPU engines.
Env: BENCH_SCALE=0.1 shortens every phase (smoke test).
"""
import csv, ctypes, glob, json, os, random, re, sqlite3, subprocess, threading, time
from collections import defaultdict
from ctypes import wintypes

import psutil
import uiautomation as auto
import win32pdh

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.environ.get('BENCH_OUT', os.path.join(HERE, 'run'))
APP = os.path.expandvars(r'%LOCALAPPDATA%\hindsight\hindsight.exe')
DATA = os.path.expandvars(r'%APPDATA%\Hindsight')
DB = glob.glob(DATA + r'\hindsight.*.sqlite')[0]
MEMDB = glob.glob(DATA + r'\hindsight-memory.*.sqlite')[0]
SHOTS = DATA + r'\screenshots'
DOCS = os.path.join(HERE, 'docs')

S = float(os.environ.get('BENCH_SCALE', '1'))
BASELINE, WARMUP, MEASURE, SETTLE = 300 * S, 120 * S, 300 * S, 30 * S
TICK, SLOW = 2.0, 5          # processes every 2 s; GPU and dwm every 5th tick
CONFIGS = [('shots_ocr', True, True), ('shots', True, False), ('off', False, False)]

user32 = ctypes.WinDLL('user32', use_last_error=True)
WM_CLOSE, WM_CHAR = 0x0010, 0x0102
ENUM_PROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
# Held by the fake user for each keystroke, mouse move and app switch, and by the
# tray, launch and close steps, so a switch cannot dismiss the tray menu.
UI = threading.Lock()


def log(*a):
    print(time.strftime('%H:%M:%S'), *a, flush=True)


# ── windows ──

def title(h):
    buf = ctypes.create_unicode_buffer(512)
    user32.GetWindowTextW(h, buf, 512)
    return buf.value


def class_name(h):
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(h, buf, 256)
    return buf.value


def top_windows():
    """[(hwnd, pid, class, title)] of visible top-level windows."""
    out = []

    def cb(h, _):
        if user32.IsWindowVisible(h):
            pid = wintypes.DWORD()
            user32.GetWindowThreadProcessId(h, ctypes.byref(pid))
            out.append((h, pid.value, class_name(h), title(h)))
        return True
    user32.EnumWindows(ENUM_PROC(cb), 0)
    return out


def child_windows(parent):
    out = []

    def cb(h, _):
        out.append(h)
        return True
    user32.EnumChildWindows(parent, ENUM_PROC(cb), 0)
    return out


def ctrl_s():
    with UI:
        for vk, up in ((0x11, 0), (0x53, 0), (0x53, 2), (0x11, 2)):
            user32.keybd_event(vk, 0, up, 0)
            time.sleep(0.05)                     # Notepad drops the chord without gaps


def activate(h):
    with UI:
        if user32.IsIconic(h):
            user32.ShowWindow(h, 9)              # SW_RESTORE
        # A background process may only take the foreground right after a key event.
        user32.keybd_event(0x12, 0, 0, 0)
        user32.keybd_event(0x12, 0, 2, 0)
        user32.SetForegroundWindow(h)


# ── fake user activity ──

WORDS = ('the quick brown fox jumps over the lazy dog while a small team reviews the weekly plan '
         'budget numbers look fine but the release date may move by two days so we write notes '
         'meeting at three about the new design draft and the open questions from last sprint').split()
CALC_TITLES = ('電卓', 'Calculator', '计算器', '小算盤')


class Activity(threading.Thread):
    """Notepad (type) → Explorer folder → Calculator → Notepad (type), until stopped.
    Characters are posted to the edit control of a Notepad window whose title starts
    with hs-bench, so they cannot land in another app. The mouse only moves."""

    def __init__(self):
        super().__init__(daemon=True)
        self.stop = threading.Event()
        self.w = self.on_line = 0
        random.seed(42)
        self.before = {p.info['name'] for p in psutil.process_iter(['name'])}

    def idle(self, secs):
        """Waits, moving the mouse twice a second (keeps the idle timer at zero)."""
        end = time.time() + secs
        while time.time() < end and not self.stop.is_set():
            pt = wintypes.POINT()
            user32.GetCursorPos(ctypes.byref(pt))
            cx, cy = user32.GetSystemMetrics(0) // 2, user32.GetSystemMetrics(1) // 2
            dx = random.randint(-40, 40) + (cx - pt.x) // 20
            dy = random.randint(-30, 30) + (cy - pt.y) // 20
            with UI:
                user32.mouse_event(0x0001, dx, dy, 0, 0)  # MOUSEEVENTF_MOVE, relative
            time.sleep(0.5)

    def show(self, match, launch):
        find = lambda: next((h for h, _, c, t in top_windows() if match(c, t)), None)
        h = find()
        if h is None:
            launch()
            for _ in range(20):
                time.sleep(0.5)
                h = find()
                if h:
                    break
        if h:
            activate(h)

    def notepad(self, name):
        path = os.path.join(DOCS, name)
        self.show(lambda c, t: c == 'Notepad' and t.lstrip('*').startswith('hs-bench'),
                  lambda: subprocess.Popen(['notepad.exe', path]))
        # Opening the file again switches Notepad to its tab.
        subprocess.Popen(['notepad.exe', path])
        self.idle(1.5)

    def editor(self):
        """The visible edit control of the foreground Notepad, if it shows our file."""
        h = user32.GetForegroundWindow()
        if class_name(h) != 'Notepad' or not title(h).lstrip('*').startswith('hs-bench'):
            return None
        return next((c for c in child_windows(h)
                     if class_name(c) == 'RichEditD2DPT' and user32.IsWindowVisible(c)), None)

    def type(self, secs):
        """Types words at about six characters a second, then saves."""
        end = time.time() + secs
        while time.time() < end and not self.stop.is_set():
            edit = self.editor()
            if not edit:
                self.idle(0.5)
                continue
            word = WORDS[self.w % len(WORDS)] + ' '
            self.w += 1
            for ch in word:
                with UI:
                    user32.PostMessageW(edit, WM_CHAR, ord(ch), 0)
                time.sleep(0.12 + random.random() * 0.08)
            self.on_line += 1
            if self.on_line == 12:
                user32.PostMessageW(edit, WM_CHAR, 13, 0)
                self.on_line = 0
            if random.random() < 0.3:
                self.idle(0.5)
        if self.editor():                         # only while our file is in front
            ctrl_s()

    def run(self):
        folder = os.path.join(DOCS, 'hs-bench-files')
        while not self.stop.is_set():
            self.notepad('hs-bench-a.txt')
            self.type(25)
            self.show(lambda c, t: c == 'CabinetWClass' and t.startswith('hs-bench-files'),
                      lambda: subprocess.Popen(['explorer.exe', folder]))
            self.idle(15)
            self.show(lambda c, t: c == 'ApplicationFrameWindow' and t in CALC_TITLES,
                      lambda: subprocess.Popen(['calc.exe']))
            self.idle(10)
            self.notepad('hs-bench-b.txt')
            self.type(20)

    def cleanup(self):
        """Save both files, then close the windows we opened; leave apps that were
        already running."""
        for name in ('hs-bench-a.txt', 'hs-bench-b.txt'):
            subprocess.Popen(['notepad.exe', os.path.join(DOCS, name)])
            time.sleep(1.5)
            if self.editor():
                ctrl_s()
                time.sleep(1)
        for h, _, c, t in top_windows():
            if (c == 'CabinetWClass' and t.startswith('hs-bench-files')
                    or c == 'ApplicationFrameWindow' and t in CALC_TITLES and 'CalculatorApp.exe' not in self.before
                    or c == 'Notepad' and 'Notepad.exe' not in self.before):
                user32.PostMessageW(h, WM_CLOSE, 0, 0)


# ── measuring ──

def role_of(p, main):
    if p.pid == main:
        return 'main'
    name, cmd = p.name().lower(), ' '.join(p.cmdline())
    if name == 'hindsight.exe':
        return 'ocr_worker' if '--ocr-worker' in cmd else 'hindsight_child'
    if name == 'msedgewebview2.exe':
        t = re.search(r'--type=(\S+)', cmd)
        u = re.search(r'--utility-sub-type=(\w+)', cmd)
        if not t:
            return 'webview_browser'
        if t.group(1) == 'utility' and u:
            return 'webview_' + u.group(1)       # network / storage
        return 'webview_' + t.group(1).replace('-process', '').replace('-handler', '')
    return name.removesuffix('.exe')


_roles = {}


def hindsight_procs():
    """[(process, role)] for the app and everything it started; main pid or None."""
    mains = [p for p in psutil.process_iter(['name', 'cmdline'])
             if p.info['name'] == 'hindsight.exe' and '--ocr-worker' not in (p.info['cmdline'] or [])]
    if not mains:
        return [], None
    main = mains[0]
    out = []
    for p in [main] + main.children(recursive=True):
        try:
            key = (p.pid, p.create_time())
            if key not in _roles:
                _roles[key] = role_of(p, main.pid)
            out.append((p, _roles[key]))
        except psutil.Error:
            pass
    return out, main.pid


class Gpu:
    def __init__(self):
        self.q = win32pdh.OpenQuery()
        self.c = win32pdh.AddEnglishCounter(self.q, r'\GPU Engine(*)\Utilization Percentage')
        win32pdh.CollectQueryData(self.q)

    def per_pid(self):
        """pid -> GPU % since the last call, summed over its engines."""
        win32pdh.CollectQueryData(self.q)
        out = defaultdict(float)
        for k, v in win32pdh.GetFormattedCounterArray(self.c, win32pdh.PDH_FMT_DOUBLE).items():
            m = re.match(r'pid_(\d+)_', k)
            if m:
                out[int(m.group(1))] += v
        return out


def dwm_cpu():
    return sum(sum(p.cpu_times()[:2]) for p in psutil.process_iter(['name']) if p.info['name'] == 'dwm.exe')


def counts():
    """Screenshot files on disk and OCR frame states, read only."""
    shots = sum(len(fs) for _, _, fs in os.walk(SHOTS))
    con = sqlite3.connect(f'file:{MEMDB}?mode=ro', uri=True)
    states = dict(con.execute('SELECT ocr_state, count(*) FROM frames GROUP BY ocr_state').fetchall())
    con.close()
    return {'shots': shots, 'ocr_done': states.get(1, 0), 'ocr_pending': states.get(0, 0)}


# ── settings and app control ──

def read_settings():
    con = sqlite3.connect(f'file:{DB}?mode=ro', uri=True)
    a, b = con.execute("SELECT json_extract(data,'$.screenshotEnabled'), json_extract(data,'$.memoryOcrResident') "
                       "FROM settings_store WHERE id=1").fetchone()
    con.close()
    return a == 1, b == 1


def write_settings(shots, ocr):
    assert hindsight_procs()[1] is None, 'Hindsight must not be running'
    j = lambda v: "json('true')" if v else "json('false')"
    con = sqlite3.connect(DB)
    con.execute(f"UPDATE settings_store SET data = json_set(data, '$.screenshotEnabled', {j(shots)}, "
                f"'$.memoryOcrResident', {j(ocr)}) WHERE id = 1")
    con.commit()
    con.close()
    assert read_settings() == (shots, ocr)


def main_window(pid):
    return next((h for h, p, _, t in top_windows() if p == pid and t == 'hindsight'), None)


QUIT_WORDS = ('退出', '結束', 'Quit', 'Exit', '終了', 'Salir', 'Sair')


def quit_app():
    """Right-click the tray icon and press its quit item, as a user would."""
    if hindsight_procs()[1] is None:
        return
    with UI:
        _quit_from_tray()
    for _ in range(60):
        time.sleep(0.5)
        if hindsight_procs()[1] is None:
            return
    raise RuntimeError('Hindsight did not quit')


def _quit_from_tray():
    bar = auto.PaneControl(ClassName='Shell_TrayWnd', searchDepth=1)
    overflow = auto.PaneControl(searchDepth=1, ClassName='TopLevelWindowForOverflowXamlIsland')
    for _ in range(3):
        # On the taskbar, or among the hidden icons if their flyout is open.
        icon = next((i for root in (bar, overflow) if root.Exists(0.5)
                     for i in [root.ButtonControl(searchDepth=8, RegexName='^Hindsight')] if i.Exists(0.5)), None)
        if icon:
            break
        # The first tray button opens (or closes) the hidden-icons flyout.
        bar.ButtonControl(searchDepth=8, ClassName='SystemTray.NormalButton').GetInvokePattern().Invoke()
        time.sleep(1)
    else:
        raise RuntimeError('tray icon not found')
    icon.RightClick(simulateMove=False)
    menu = auto.MenuControl(searchDepth=1, ClassName='#32768')
    items = menu.GetChildren() if menu.Exists(3) else []
    item = next((i for i in items if any(w in i.Name for w in QUIT_WORDS)), None)
    if item is None:
        auto.SendKeys('{Esc}')
        raise RuntimeError('quit item not found')
    log('quit:', item.Name)
    item.GetInvokePattern().Invoke()


def launch_app():
    # Through Explorer, so the app is not a child of this script (or of its terminal).
    with UI:
        subprocess.Popen(['explorer.exe', APP])
        for _ in range(120):
            time.sleep(0.5)
            _, main = hindsight_procs()
            if main and main_window(main):
                return main
    raise RuntimeError('Hindsight window did not appear')


def close_window(main):
    with UI:
        user32.PostMessageW(main_window(main), WM_CLOSE, 0, 0)
        time.sleep(1)
    if main_window(main):
        raise RuntimeError('window still open')
    log('closed window')


# ── sampling ──

def sample(phase, secs, writer, phases, gpu):
    log(f'phase {phase} ({secs:.0f} s)')
    start, c0 = time.time(), counts()
    n = 0
    while True:
        now = time.time()
        if now - start >= secs:
            break
        procs, _ = hindsight_procs()
        slow = n % SLOW == 0
        g = gpu.per_pid() if slow else {}
        for p, role in procs:
            try:
                with p.oneshot():
                    cpu, mem, io = p.cpu_times(), p.memory_full_info(), p.io_counters()
            except psutil.Error:
                continue
            writer.writerow({'t': f'{now:.3f}', 'phase': phase, 'pid': p.pid, 'role': role,
                             'cpu_s': cpu.user + cpu.system, 'footprint': mem.uss, 'resident': mem.wset,
                             'commit': mem.private, 'disk_r': io.read_bytes, 'disk_w': io.write_bytes,
                             'start': p.create_time(), 'gpu_pct': g.get(p.pid, 0) if slow else ''})
        if slow:
            writer.writerow({'t': f'{now:.3f}', 'phase': phase, 'pid': 0, 'role': 'sys_dwm', 'cpu_s': dwm_cpu()})
        n += 1
        time.sleep(max(0, start + n * TICK - time.time()))
    c1 = counts()
    phases.append({'phase': phase, 'start': start, 'end': time.time(),
                   **{k: c1[k] - c0[k] for k in ('shots', 'ocr_done')}, 'ocr_pending_end': c1['ocr_pending']})


def main():
    os.makedirs(OUT, exist_ok=True)
    os.makedirs(DOCS + r'\hs-bench-files', exist_ok=True)
    for name in ('hs-bench-a.txt', 'hs-bench-b.txt'):
        open(os.path.join(DOCS, name), 'a').close()
    for i in range(5):
        open(os.path.join(DOCS, 'hs-bench-files', f'note-{i}.txt'), 'w').write('bench file\n')

    original = read_settings()
    json.dump({'screenshotEnabled': original[0], 'memoryOcrResident': original[1]},
              open(os.path.join(OUT, 'original_settings.json'), 'w'))
    log('original settings', original, 'scale', S)

    fields = ['t', 'phase', 'pid', 'role', 'cpu_s', 'footprint', 'resident', 'commit', 'disk_r', 'disk_w',
              'energy_nj', 'wakeups', 'start', 'gpu_ns', 'gpu_pct']
    f = open(os.path.join(OUT, 'samples.csv'), 'w', newline='')
    writer = csv.DictWriter(f, fieldnames=fields)
    writer.writeheader()
    phases = []
    gpu = Gpu()

    ctypes.windll.kernel32.SetThreadExecutionState(0x80000003)   # no sleep, display on
    act = Activity()
    act.start()
    try:
        quit_app()
        sample('baseline', BASELINE, writer, phases, gpu)
        for name, shots, ocr in CONFIGS:
            write_settings(shots, ocr)
            main_pid = launch_app()
            log(name, 'launched', main_pid)
            sample(f'warmup_{name}', WARMUP, writer, phases, gpu)
            sample(f'open_{name}', MEASURE, writer, phases, gpu)
            close_window(main_pid)
            sample(f'settle_{name}', SETTLE, writer, phases, gpu)
            sample(f'closed_{name}', MEASURE, writer, phases, gpu)
            quit_app()
    finally:
        f.flush()
        json.dump(phases, open(os.path.join(OUT, 'phases.json'), 'w'), indent=1)
        act.stop.set()
        act.join(timeout=60)
        act.cleanup()
        try:
            quit_app()
            write_settings(*original)
            main_pid = launch_app()
            close_window(main_pid)       # it was running with its window closed
            log('restored', read_settings())
        finally:
            ctypes.windll.kernel32.SetThreadExecutionState(0x80000000)


if __name__ == '__main__':
    main()
