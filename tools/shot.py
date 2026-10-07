#!/usr/bin/env python3
"""Render a URL with Castilia (examples/browse) and with Chromium, save both and a side-by-side PNG.

usage: shot.py URL NAME [--width 1280] [--height 2000] [--out DIR]
"""
import os, subprocess, sys
from pathlib import Path
from playwright.sync_api import sync_playwright
from PIL import Image

def main():
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    url, name = sys.argv[1], sys.argv[2]
    width, height, out = 1280, 2000, 'shots'
    a = sys.argv[3:]
    for i in range(0, len(a) - 1, 2):
        if a[i] == '--width': width = int(a[i + 1])
        if a[i] == '--height': height = int(a[i + 1])
        if a[i] == '--out': out = a[i + 1]
    os.makedirs(out, exist_ok=True)
    exe = os.path.join('target', 'release', 'examples', 'browse.exe' if os.name == 'nt' else 'browse')
    svg = os.path.abspath(os.path.join(out, f'{name}.svg'))
    env = dict(os.environ, CASTILIA_WIDTH=str(width))
    p = subprocess.run([exe, url, svg], capture_output=True, text=True, encoding='utf-8', errors='replace', env=env, timeout=180,
                       creationflags=getattr(subprocess, 'BELOW_NORMAL_PRIORITY_CLASS', 0))
    print('\n'.join(l[:300] for l in p.stdout.splitlines()[:40]))
    print(p.stderr.strip()[-1500:])
    ours, ref, side = (os.path.join(out, f'{name}-{k}.png') for k in ('castilia', 'chrome', 'side'))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True)
        pg = b.new_page(viewport={'width': width, 'height': height})
        pg.goto(Path(svg).as_uri())
        pg.screenshot(path=ours, clip={'x': 0, 'y': 0, 'width': width, 'height': height})
        pg = b.new_page(viewport={'width': width, 'height': height}, user_agent='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36')
        try:
            pg.goto(url, wait_until='load', timeout=60000)
            pg.wait_for_timeout(2000)
        except Exception as e:
            print('chrome:', e)
        pg.screenshot(path=ref)
        b.close()
    a, b = Image.open(ours).convert('RGB'), Image.open(ref).convert('RGB')
    h = max(a.height, b.height)
    s = Image.new('RGB', (a.width + b.width + 16, h), 'white')
    s.paste(a, (0, 0)); s.paste(b, (a.width + 16, 0))
    s.save(side)
    print('saved', side)

main()
