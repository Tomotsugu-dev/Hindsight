#!/usr/bin/env python3
"""Summarize run/samples.csv: one block per phase, per role and in total."""
import csv, json, os, sys
from collections import defaultdict

RUN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), 'run')
rows = list(csv.DictReader(open(os.path.join(RUN, 'samples.csv'))))
phases = json.load(open(os.path.join(RUN, 'phases.json')))
MB = 2 ** 20

def num(v):
    return float(v) if v not in ('', None) else None

by_phase = defaultdict(list)
for r in rows:
    by_phase[r['phase']].append(r)

def counter_total(rs, field, roles=None):
    """Sum over pids of how much a cumulative counter grew between their first and
    last sample in the phase (a process that lives under 2 s is missed)."""
    first, last = {}, {}
    for r in rs:
        if roles and r['role'] not in roles:
            continue
        v = num(r[field])
        if v is None:
            continue
        p = (r['pid'], r['role'])
        first.setdefault(p, v)
        last[p] = v
    return sum(last[p] - v0 for p, v0 in first.items())

# bench_win.py writes a commit column; on Windows every non-system role is Hindsight's
WIN = 'commit' in rows[0]
HS = ({r['role'] for r in rows if not r['role'].startswith('sys_')} if WIN
      else {'main', 'ocr_worker', 'webkit_WebContent', 'webkit_GPU', 'webkit_Networking'})

out = {}
for ph in phases:
    name, rs = ph['phase'], by_phase[ph['phase']]
    dur = ph['end'] - ph['start']
    hs = [r for r in rs if r['role'] in HS]
    ticks = sorted({r['t'] for r in hs})
    # memory: total footprint per tick, and per role
    per_tick = defaultdict(float)
    role_mem = defaultdict(list)
    tick_role = defaultdict(lambda: defaultdict(float))
    for r in hs:
        per_tick[r['t']] += num(r['footprint'])
        tick_role[r['t']][r['role']] += num(r['footprint'])
    for t in ticks:
        for role in HS:
            role_mem[role].append(tick_role[t].get(role, 0.0))
    mem = list(per_tick.values())
    # CPU per role over the phase, and peak 2 s total
    cpu_role = {role: counter_total(hs, 'cpu_s', {role}) / dur * 100 for role in HS}
    series = defaultdict(dict)
    for r in hs:
        series[(r['pid'], r['role'])][float(r['t'])] = num(r['cpu_s'])
    tick_cpu = defaultdict(float)
    for s in series.values():
        ts = sorted(s)
        for a, b in zip(ts, ts[1:]):
            tick_cpu[b] += (s[b] - s[a]) / (b - a) * 100
    peaks = sorted(tick_cpu.values())
    if WIN:
        # gpu_pct is already a rate over the last 10 s: sum over processes per tick, then average
        tick_gpu = defaultdict(float)
        for r in hs:
            if r['gpu_pct'] != '':
                tick_gpu[r['t']] += num(r['gpu_pct'])
        gpu = sum(tick_gpu.values()) / len(tick_gpu) if tick_gpu else 0
    else:
        gpu = counter_total([r for r in hs if r['gpu_ns'] != ''], 'gpu_ns') / 1e9 / dur * 100
    energy_mw = counter_total(hs, 'energy_nj') / 1e6 / dur
    disk_w = counter_total(hs, 'disk_w') / MB / dur * 3600
    disk_r = counter_total(hs, 'disk_r') / MB / dur * 3600
    wake = counter_total(hs, 'wakeups') / dur
    sysr = [r for r in rs if r['role'].startswith('sys_')]
    ws = counter_total(sysr, 'cpu_s', {'sys_WindowServer'}) / dur * 100
    rp = counter_total(sysr, 'cpu_s', {'sys_replayd'}) / dur * 100
    gu = [num(r['gpu_ns']) for r in sysr if r['role'] == 'sys_gpu_util' and r['gpu_ns'] != '']
    out[name] = {
        'dur_s': round(dur),
        'mem_mb_mean': round(sum(mem) / len(mem) / MB, 1) if mem else 0,
        'mem_mb_max': round(max(mem) / MB, 1) if mem else 0,
        'mem_by_role_mb': {k: round(sum(v) / len(v) / MB, 1) for k, v in role_mem.items() if v and max(v) > 0},
        'cpu_pct': round(sum(cpu_role.values()), 2),
        'cpu_by_role_pct': {k: round(v, 2) for k, v in cpu_role.items() if v > 0},
        'cpu_peak_2s_pct': round(peaks[-1], 1) if peaks else 0,
        'cpu_p95_2s_pct': round(peaks[int(len(peaks) * 0.95) - 1], 1) if peaks else 0,
        'gpu_pct': round(gpu, 2),
        'energy_mw': round(energy_mw, 1),
        'disk_write_mb_per_h': round(disk_w, 1),
        'disk_read_mb_per_h': round(disk_r, 1),
        'wakeups_per_s': round(wake, 1),
        'sys_windowserver_cpu_pct': round(ws, 2),
        'sys_replayd_cpu_pct': round(rp, 2),
        'sys_gpu_util_mean_pct': round(sum(gu) / len(gu), 1) if gu else None,
        'screenshots': ph['shots'], 'ocr_done': ph['ocr_done'], 'ocr_pending_end': ph['ocr_pending_end'],
    }
    if WIN:
        # no energy or wakeup counters on Windows; dwm stands in for WindowServer
        commit = defaultdict(float)
        for r in hs:
            commit[r['t']] += num(r['commit'])
        for k in ('energy_mw', 'wakeups_per_s', 'sys_windowserver_cpu_pct', 'sys_replayd_cpu_pct',
                  'sys_gpu_util_mean_pct'):
            del out[name][k]
        out[name]['commit_mb_mean'] = round(sum(commit.values()) / len(commit) / MB, 1) if commit else 0
        out[name]['sys_dwm_cpu_pct'] = round(counter_total(sysr, 'cpu_s', {'sys_dwm'}) / dur * 100, 2)

json.dump(out, open(os.path.join(RUN, 'summary.json'), 'w'), indent=1, ensure_ascii=False)
for k, v in out.items():
    print(k, json.dumps(v, ensure_ascii=False))
