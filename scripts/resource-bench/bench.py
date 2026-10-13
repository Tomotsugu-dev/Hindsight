#!/usr/bin/env python3
"""Resource benchmark for the installed Hindsight (macOS only).

Phases: baseline (Hindsight quit), then for each setting (screenshots + OCR,
screenshots only, both off): warm-up, window open, window closed.
Hindsight is quit through its tray menu, its two settings are changed in the
database while it is not running, and the original values are put back at the end.

Build the two helpers in this folder first:
    swiftc -O -o axctl axctl.swift
    swiftc -O -o activity activity.swift
The terminal needs Accessibility permission. A full run takes about 45 minutes.

Writes samples.csv (one row per process per sample) and phases.json to run/;
`python3 analyze.py` then writes summary.json there.
Env: BENCH_SCALE=0.1 shortens every phase (smoke test).
"""
import ctypes, csv, glob, json, os, plistlib, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.environ.get('BENCH_OUT', os.path.join(HERE, 'run'))
APP = '/Applications/hindsight.app'
DATA = os.path.expanduser('~/Library/Application Support/Hindsight')
DB = [p for p in glob.glob(DATA + '/hindsight.*.sqlite')][0]
MEMDB = glob.glob(DATA + '/hindsight-memory.*.sqlite')[0]
SHOTS = DATA + '/screenshots'
DOCS = os.path.join(HERE, 'docs')

S = float(os.environ.get('BENCH_SCALE', '1'))
BASELINE, WARMUP, MEASURE, SETTLE = 300 * S, 120 * S, 300 * S, 30 * S
TICK, SLOW = 2.0, 5          # rusage every 2 s; GPU and system processes every 5th tick
CONFIGS = [('shots_ocr', True, True), ('shots', True, False), ('off', False, False)]

libproc = ctypes.CDLL('/usr/lib/libproc.dylib')
libc = ctypes.CDLL(None)
libc.responsibility_get_pid_responsible_for_pid.restype = ctypes.c_int
libc.responsibility_get_pid_responsible_for_pid.argtypes = [ctypes.c_int]
class _TB(ctypes.Structure):
    _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]
_tb = _TB(); libc.mach_timebase_info(ctypes.byref(_tb))
TICKS_TO_S = _tb.numer / _tb.denom / 1e9


def log(*a):
    print(time.strftime('%H:%M:%S'), *a, flush=True)


def sh(*args, check=True):
    return subprocess.run(args, capture_output=True, text=True, check=check).stdout


def rusage(pid):
    """proc_pid_rusage(RUSAGE_INFO_V6); None if the process is gone."""
    buf = (ctypes.c_uint64 * 80)()
    if libproc.proc_pid_rusage(pid, 6, ctypes.byref(buf)) != 0:
        return None
    f = lambda i: buf[2 + i]          # skip the 16-byte uuid
    return {'cpu_s': (f(0) + f(1)) * TICKS_TO_S, 'footprint': f(7), 'resident': f(6),
            'disk_r': f(16), 'disk_w': f(17), 'energy_nj': f(40),
            'wakeups': f(2) + f(3), 'start': f(8) * TICKS_TO_S}


def processes():
    rows = []
    for line in sh('ps', '-axo', 'pid=,ppid=,comm=').splitlines():
        pid, ppid, comm = line.strip().split(None, 2)
        rows.append((int(pid), int(ppid), comm))
    return rows


def hindsight_procs():
    """pid -> role for the app, its OCR worker and the WebKit processes it owns."""
    rows = processes()
    exe = lambda c: c.lower().endswith('/contents/macos/hindsight')
    mains = [p for p, pp, c in rows if exe(c) and pp == 1]
    if not mains:
        return {}, None
    main = mains[0]
    roles = {main: 'main'}
    for p, pp, c in rows:
        if exe(c) and pp == main:
            roles[p] = 'ocr_worker'
        elif 'com.apple.WebKit.' in c and libc.responsibility_get_pid_responsible_for_pid(p) == main:
            roles[p] = 'webkit_' + c.rsplit('com.apple.WebKit.', 1)[1]
    return roles, main


def gpu_ns():
    """pid -> accumulated GPU time (ns) over all its Metal clients."""
    out = {}
    for c in plistlib.loads(subprocess.run(['ioreg', '-a', '-r', '-c', 'AGXDeviceUserClient'],
                                           capture_output=True).stdout or b'<plist><array/></plist>'):
        m = re.match(r'pid (\d+)', c.get('IOUserClientCreator', ''))
        if m:
            pid = int(m.group(1))
            out[pid] = out.get(pid, 0) + sum(u.get('accumulatedGPUTime', 0) for u in c.get('AppUsage', []))
    return out


def gpu_util():
    m = re.search(r'"Device Utilization %"=(\d+)', sh('ioreg', '-r', '-d', '1', '-c', 'IOAccelerator'))
    return int(m.group(1)) if m else None


def cputime(names):
    """Cumulative CPU seconds of system processes (ps works for other users' processes)."""
    tot = {n: 0.0 for n in names}
    for line in sh('ps', '-axo', 'time=,comm=').splitlines():
        t, comm = line.strip().split(None, 1)
        n = comm.rsplit('/', 1)[-1]
        if n in tot:
            parts = [float(x) for x in t.replace('-', ':').split(':')]
            s = 0.0
            for x in parts:
                s = s * 60 + x
            tot[n] += s
    return tot


def counts():
    """Screenshot files on disk and OCR frame states, read only."""
    shots = sum(len(fs) for _, _, fs in os.walk(SHOTS))
    q = "SELECT ocr_state, count(*) FROM frames GROUP BY ocr_state"
    states = dict(l.split('|') for l in sh('sqlite3', '-readonly', MEMDB, q).split())
    return {'shots': shots, 'ocr_done': int(states.get('1', 0)), 'ocr_pending': int(states.get('0', 0))}


# ── settings and app control ──

def read_settings():
    q = "SELECT json_extract(data,'$.screenshotEnabled'), json_extract(data,'$.memoryOcrResident') FROM settings_store WHERE id=1"
    a, b = sh('sqlite3', '-readonly', DB, q).strip().split('|')
    return a == '1', b == '1'


def write_settings(shots, ocr):
    assert hindsight_procs()[1] is None, 'Hindsight must not be running'
    j = lambda v: "json('true')" if v else "json('false')"
    sh('sqlite3', DB, f"UPDATE settings_store SET data = json_set(data, '$.screenshotEnabled', {j(shots)}, "
                      f"'$.memoryOcrResident', {j(ocr)}) WHERE id = 1")
    assert read_settings() == (shots, ocr)


def ax(cmd, pid):
    return subprocess.run([os.path.join(HERE, 'axctl'), cmd, str(pid)], capture_output=True, text=True).stdout.strip()


def window_count(pid):
    m = re.search(r'windows=(\d+)', ax('dump', pid))
    return int(m.group(1)) if m else 0


def quit_app():
    _, main = hindsight_procs()
    if main is None:
        return
    log('quit:', ax('quit', main))
    for _ in range(60):
        time.sleep(0.5)
        if not hindsight_procs()[0]:
            return
    raise RuntimeError('Hindsight did not quit')


def launch_app():
    sh('open', '-a', APP)
    for _ in range(120):
        time.sleep(0.5)
        _, main = hindsight_procs()
        if main and window_count(main) > 0:
            return main
    raise RuntimeError('Hindsight window did not appear')


def close_window(main):
    log('close:', ax('close', main))
    time.sleep(1)
    if window_count(main) != 0:
        raise RuntimeError('window still open')


# ── sampling ──

def sample(phase, secs, writer, phases):
    log(f'phase {phase} ({secs:.0f} s)')
    start, c0 = time.time(), counts()
    n = 0
    while True:
        now = time.time()
        if now - start >= secs:
            break
        roles, _ = hindsight_procs()
        slow = n % SLOW == 0
        g = gpu_ns() if slow else {}
        for pid, role in roles.items():
            r = rusage(pid)
            if r:
                writer.writerow({'t': f'{now:.3f}', 'phase': phase, 'pid': pid, 'role': role, **r,
                                 'gpu_ns': g.get(pid, 0) if slow else ''})
        if slow:
            sysrow = {'t': f'{now:.3f}', 'phase': phase, 'pid': 0, 'role': 'system',
                      'gpu_ns': gpu_util()}
            for name, v in cputime(['WindowServer', 'replayd']).items():
                writer.writerow({**sysrow, 'role': 'sys_' + name, 'cpu_s': v, 'gpu_ns': ''})
            writer.writerow({**sysrow, 'role': 'sys_gpu_util'})
        n += 1
        time.sleep(max(0, start + n * TICK - time.time()))
    c1 = counts()
    phases.append({'phase': phase, 'start': start, 'end': time.time(),
                   **{k: c1[k] - c0[k] for k in ('shots', 'ocr_done')}, 'ocr_pending_end': c1['ocr_pending']})


def main():
    os.makedirs(OUT, exist_ok=True)
    os.makedirs(DOCS + '/hs-bench-files', exist_ok=True)
    for name in ('hs-bench-a.txt', 'hs-bench-b.txt'):
        open(os.path.join(DOCS, name), 'a').close()
    for i in range(5):
        open(os.path.join(DOCS, 'hs-bench-files', f'note-{i}.txt'), 'w').write('bench file\n')
    try:
        os.remove(DOCS + '/STOP')
    except FileNotFoundError:
        pass

    original = read_settings()
    json.dump({'screenshotEnabled': original[0], 'memoryOcrResident': original[1]},
              open(os.path.join(OUT, 'original_settings.json'), 'w'))
    log('original settings', original, 'scale', S)

    fields = ['t', 'phase', 'pid', 'role', 'cpu_s', 'footprint', 'resident', 'disk_r', 'disk_w',
              'energy_nj', 'wakeups', 'start', 'gpu_ns']
    f = open(os.path.join(OUT, 'samples.csv'), 'w', newline='')
    writer = csv.DictWriter(f, fieldnames=fields)
    writer.writeheader()
    phases = []

    caff = subprocess.Popen(['caffeinate', '-di'])
    act = subprocess.Popen([os.path.join(HERE, 'activity'), DOCS])
    try:
        quit_app()
        sample('baseline', BASELINE, writer, phases)
        for name, shots, ocr in CONFIGS:
            write_settings(shots, ocr)
            main_pid = launch_app()
            log(name, 'launched', main_pid)
            sample(f'warmup_{name}', WARMUP, writer, phases)
            sample(f'open_{name}', MEASURE, writer, phases)
            close_window(main_pid)
            sample(f'settle_{name}', SETTLE, writer, phases)
            sample(f'closed_{name}', MEASURE, writer, phases)
            quit_app()
    finally:
        f.flush()
        json.dump(phases, open(os.path.join(OUT, 'phases.json'), 'w'), indent=1)
        open(DOCS + '/STOP', 'w').close()
        act.wait(timeout=60)
        try:
            quit_app()
            write_settings(*original)
            main_pid = launch_app()
            close_window(main_pid)       # it was running with its window closed
            log('restored', read_settings())
        finally:
            caff.terminate()


if __name__ == '__main__':
    main()
