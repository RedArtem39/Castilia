#!/usr/bin/env python3
"""Text Castilia draws that Chromium does not show (and the reverse), to find leaking hidden content.

usage: textdiff.py URL [--width 1024] [--top 40]

Renders with examples/browse (SVG), extracts the drawn text runs, and compares them with
Chromium's document.body.innerText. Prints our runs missing from Chromium's text, grouped by
first occurrence, then a short list of Chromium lines we never drew.
"""
import os, re, subprocess, sys, tempfile, html
from playwright.sync_api import sync_playwright


def main():
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')
    url = sys.argv[1]
    width, top = 1024, 40
    a = sys.argv[2:]
    for i in range(0, len(a) - 1, 2):
        if a[i] == '--width': width = int(a[i + 1])
        if a[i] == '--top': top = int(a[i + 1])
    exe = os.path.join('target', 'release', 'examples', 'browse.exe' if os.name == 'nt' else 'browse')
    svg = os.path.join(tempfile.gettempdir(), 'castilia_textdiff.svg')
    env = dict(os.environ, CASTILIA_WIDTH=str(width), CASTILIA_MEM_MB=os.environ.get('CASTILIA_MEM_MB', '1500'))
    subprocess.run([exe, url, svg], capture_output=True, env=env, timeout=240,
                   creationflags=getattr(subprocess, 'BELOW_NORMAL_PRIORITY_CLASS', 0))
    src = open(svg, encoding='utf-8').read()
    ours = []
    for m in re.finditer(r'<text[^>]*\by="([\d.-]+)"[^>]*fill-opacity="([\d.]+)"[^>]*>(.*?)</text>', src, re.S):
        if float(m.group(2)) == 0:
            continue
        t = html.unescape(re.sub(r'<[^>]+>', '', m.group(3))).strip()
        if t:
            ours.append((float(m.group(1)), t))
    with sync_playwright() as pw:
        b = pw.chromium.launch(headless=True)
        p = b.new_page(viewport={'width': width, 'height': 900},
                       user_agent='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36')
        p.goto(url, wait_until='load', timeout=60000)
        p.wait_for_timeout(1500)
        ref = p.evaluate("document.body ? document.body.innerText : ''") or ''
    norm = lambda s: re.sub(r'\s+', '', s).lower()
    ref_n = norm(ref)
    extra = [(y, t) for y, t in ours if norm(t) and norm(t) not in ref_n]
    total = sum(len(norm(t)) for _, t in ours)
    extra_chars = sum(len(norm(t)) for _, t in extra)
    print(f'ours {total} chars, chrome {len(ref_n)} chars, ours-not-in-chrome {extra_chars} chars in {len(extra)} runs')
    for y, t in extra[:top]:
        print(f'  y={y:7.0f}  {t[:110]}')
    drawn = norm(' '.join(t for _, t in ours))
    missing = [l.strip() for l in ref.splitlines() if len(norm(l)) > 3 and norm(l) not in drawn]
    print(f'chrome lines we never drew: {len(missing)}')
    for l in missing[:top // 2]:
        print('  ' + l[:110])


if __name__ == '__main__':
    main()
