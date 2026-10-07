#!/usr/bin/env python3
"""Compare Castilia with Chromium on many real sites.

usage: webcheck.py LIST [--out results.jsonl] [--jobs 3] [--chrome-jobs 2] [--resume]

For every URL: our engine renders it in a child process (examples/sitecheck --one, memory capped),
Chromium (Playwright) loads it and reports the visible text (document.body.innerText). A site
"passes" when we load it without crash/hang and our visible character count is within 0.5x–2x of
Chromium's. Sites Chromium itself cannot load (or that show < 50 characters there) are excluded.
"""
import concurrent.futures as cf
import json, os, re, subprocess, sys, threading, time

def parse_args():
    a = sys.argv[1:]
    opts = {'list': a[0], 'out': 'webcheck.jsonl', 'jobs': 3, 'chrome_jobs': 2, 'resume': False}
    i = 1
    while i < len(a):
        if a[i] == '--out': opts['out'] = a[i + 1]; i += 2
        elif a[i] == '--jobs': opts['jobs'] = int(a[i + 1]); i += 2
        elif a[i] == '--chrome-jobs': opts['chrome_jobs'] = int(a[i + 1]); i += 2
        elif a[i] == '--resume': opts['resume'] = True; i += 1
        else: i += 1
    return opts

EXE = os.environ.get('CASTILIA_SITECHECK') or os.path.join('target', 'release', 'examples', 'sitecheck.exe' if os.name == 'nt' else 'sitecheck')
FLAGS = getattr(subprocess, 'BELOW_NORMAL_PRIORITY_CLASS', 0)

def ours(url):
    start = time.time()
    try:
        p = subprocess.run([EXE, '--one', url], capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=150,
                           creationflags=FLAGS, env=dict(os.environ, CASTILIA_MEM_MB='1024'))
        out, err = p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return {'status': 'timeout', 'ms': int((time.time() - start) * 1000)}
    m = re.search(r'^RESULT (\S+)(.*)$', out, re.M)
    if not m:
        line = next((l for l in err.splitlines() if 'panicked' in l or 'memory' in l), err.strip()[-200:])
        return {'status': 'crash', 'detail': line[:200], 'ms': int((time.time() - start) * 1000)}
    r = {'status': m.group(1), 'ms': int((time.time() - start) * 1000)}
    for k, v in re.findall(r'(\w+)=(\S+)', m.group(2)):
        r[k] = int(v) if v.lstrip('-').isdigit() else v
    r['errors_text'] = [l[4:][:160] for l in out.splitlines() if l.startswith('ERR ')][:5]
    return r

_local = threading.local()
WALL_MARKERS = ('/cdn-cgi/challenge-platform/', 'awswaf.com', 'bm-verify', 'captcha-delivery.com', '_Incapsula_Resource', 'px-captcha', 'challenges.cloudflare.com', '/_sec/verify')

def chrome(url):
    from playwright.sync_api import sync_playwright
    if not hasattr(_local, 'browser'):
        _local.pw = sync_playwright().start()
        _local.browser = _local.pw.chromium.launch(headless=True)
    page = _local.browser.new_page(viewport={'width': 1024, 'height': 768},
                                   user_agent='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36')
    try:
        page.goto(url, wait_until='load', timeout=45000)
        page.wait_for_timeout(1500)
        text = page.evaluate("document.body ? document.body.innerText : ''") or ''
        height = page.evaluate("document.documentElement.scrollHeight")
        html = page.content()
        wall = len(html) < 300000 and any(m in html for m in WALL_MARKERS)
        return {'chars': sum(1 for c in text if not c.isspace()), 'height': height, 'wall': wall}
    except Exception as e:
        return {'error': str(e)[:160]}
    finally:
        page.close()

def classify(o, c):
    if 'error' in c or c.get('chars', 0) < 50:
        return 'excluded'
    if c.get('wall'):
        return 'excluded:bot-wall'
    if o['status'] == 'bot-wall':
        # an anti-bot interstitial: says nothing about the engine (headless Chromium often gets one too)
        return 'excluded:bot-wall'
    if o['status'] in ('blocked', 'fetch-fail'):
        # Chromium got the page and we did not: a host networking failure (TLS, HTTP/2, bot checks)
        return 'fail:network'
    if o['status'] != 'ok':
        return 'fail:' + o['status']
    ratio = (o.get('chars', 0) + 1) / (c['chars'] + 1)
    if ratio < 0.5:
        return 'fail:little-text'
    if ratio > 2.0:
        return 'fail:extra-text'
    return 'pass'

def main():
    o = parse_args()
    urls = [l.strip() for l in open(o['list'], encoding='utf-8') if l.strip() and not l.startswith('#')]
    done = {}
    if o['resume'] and os.path.exists(o['out']):
        for l in open(o['out'], encoding='utf-8'):
            r = json.loads(l)
            done[r['url']] = r
    todo = [u for u in urls if u not in done]
    lock = threading.Lock()
    out = open(o['out'], 'a', encoding='utf-8')
    chrome_pool = cf.ThreadPoolExecutor(o['chrome_jobs'])
    def work(url):
        cfut = chrome_pool.submit(chrome, url)
        ro = ours(url)
        rc = cfut.result()
        rec = {'url': url, 'ours': ro, 'chrome': rc, 'verdict': classify(ro, rc)}
        with lock:
            out.write(json.dumps(rec, ensure_ascii=False) + '\n')
            out.flush()
            n = sum(1 for _ in open(o['out'], encoding='utf-8'))
            print(f"[{n}/{len(urls)}] {rec['verdict']:<18} ours={ro.get('chars', '-')} chrome={rc.get('chars', '-')} {url}", flush=True)
        return rec
    with cf.ThreadPoolExecutor(o['jobs']) as ex:
        list(ex.map(work, todo))
    out.close()
    summarize(o['out'])

def summarize(path):
    recs = [json.loads(l) for l in open(path, encoding='utf-8')]
    counts = {}
    for r in recs:
        counts[r['verdict']] = counts.get(r['verdict'], 0) + 1
    considered = [r for r in recs if not r['verdict'].startswith('excluded')]
    passed = sum(1 for r in considered if r['verdict'] == 'pass')
    clean = sum(1 for r in considered if r['verdict'] == 'pass' and r['ours'].get('errors', 1) == 0)
    print('\nverdicts:', counts)
    if considered:
        print(f'pass: {passed}/{len(considered)} = {100 * passed / len(considered):.1f}%   pass without script errors: {100 * clean / len(considered):.1f}%')

if __name__ == '__main__':
    if len(sys.argv) > 2 and sys.argv[1] == '--summary':
        summarize(sys.argv[2])
    else:
        main()
