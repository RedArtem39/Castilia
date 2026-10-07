#!/usr/bin/env python3
"""Time tools/bench/*.js in each execution mode: run.py [--mode interp|jit|ast]..."""
import glob, os, subprocess, sys, time
modes = [a for i, a in enumerate(sys.argv) if i > 0 and sys.argv[i - 1] == '--mode'] or ['ast', 'interp', 'jit']
exe = os.path.join('target', 'release', 'examples', 'jsrun.exe' if os.name == 'nt' else 'jsrun')
for t in sorted(glob.glob(os.path.join(os.path.dirname(__file__), '*.js'))):
    row = []
    for m in modes:
        best = None
        out = ''
        for _ in range(2):
            s = time.time()
            p = subprocess.run([exe, '--mode', m, t], capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=600,
                               creationflags=getattr(subprocess, 'BELOW_NORMAL_PRIORITY_CLASS', 0))
            d = time.time() - s
            best = d if best is None else min(best, d)
            out = [l for l in p.stdout.splitlines() if l.startswith('log:')]
        row.append(f'{m}={best:.2f}s {out[-1][5:] if out else p.stderr.strip()[-200:]}')
    print(os.path.basename(t).ljust(14), ' | '.join(row))
