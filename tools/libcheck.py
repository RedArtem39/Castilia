#!/usr/bin/env python3
"""Run tools/libtests/*.js against real libraries with examples/jsrun.

usage: libcheck.py LIBDIR [--mode interp|jit|ast]... [--exe path]
Each test names its libraries (`// libs: a.js b.js`) and the console line it must print
(`// expect: ...`). Libraries are not part of the repository; download them into LIBDIR.
"""
import glob, os, subprocess, sys, time

def main():
    args = sys.argv[1:]
    libdir = args[0]
    modes, exe = [], os.path.join('target', 'release', 'examples', 'jsrun.exe' if os.name == 'nt' else 'jsrun')
    i = 1
    while i < len(args):
        if args[i] == '--mode': modes.append(args[i + 1]); i += 2
        elif args[i] == '--exe': exe = args[i + 1]; i += 2
        else: i += 1
    modes = modes or ['interp']
    tests = sorted(glob.glob(os.path.join(os.path.dirname(__file__), 'libtests', '*.js')))
    failures = 0
    for mode in modes:
        ok = 0
        for t in tests:
            head = open(t, encoding='utf-8').read().splitlines()[:2]
            libs = head[0].split(':', 1)[1].split()
            expect = head[1].split(':', 1)[1].strip()
            cmd = [exe, '--mode', mode] + [os.path.join(libdir, l) for l in libs] + [t]
            start = time.time()
            try:
                p = subprocess.run(cmd, creationflags=getattr(subprocess, 'BELOW_NORMAL_PRIORITY_CLASS', 0), capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=120)
                out, err = p.stdout, p.stderr
            except subprocess.TimeoutExpired:
                out, err = '', 'TIMEOUT'
            ms = int((time.time() - start) * 1000)
            lines = [l[5:] if l.startswith('log: ') else l for l in out.splitlines()]
            passed = any(expect == l.strip() for l in lines)
            ok += passed
            if not passed:
                failures += 1
                print(f'FAIL [{mode}] {os.path.basename(t)} ({ms} ms)\n  expected: {expect}\n  got: {lines[:6]}\n  stderr: {err.strip()[-600:]}')
            else:
                print(f'ok   [{mode}] {os.path.basename(t)} ({ms} ms)')
        print(f'== {mode}: {ok}/{len(tests)} passed')
    sys.exit(1 if failures else 0)

main()
