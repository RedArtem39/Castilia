"""Compare Castilia output with Chromium on small, deterministic HTML reftests.

usage: python3 tools/visual_bench.py [case-name ...] [--width W] [--height H] [--no-report]

For every case in visual_cases/ the same HTML is rendered by Chromium (screenshot of the
page) and by Castilia (SVG produced by the render_svg example, screenshotted by Chromium).
The images are written to visual_out/ together with a side-by-side comparison.
"""

from pathlib import Path
import argparse
import glob
import os
import shutil
import subprocess
import sys

from PIL import Image, ImageChops
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
CASES = ROOT / "visual_cases"
OUTPUT = ROOT / "visual_out"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("cases", nargs="*")
    parser.add_argument("--width", type=int, default=800)
    parser.add_argument("--height", type=int, default=300)
    parser.add_argument("--no-report", action="store_true")
    args = parser.parse_args()
    WIDTH, HEIGHT = args.width, args.height

    OUTPUT.mkdir(exist_ok=True)
    for asset in CASES.glob("*.png"):
        shutil.copy2(asset, OUTPUT / asset.name)
    build = subprocess.run(["cargo", "build", "--quiet", "--release", "--example", "render_svg"], cwd=ROOT, capture_output=True, text=True)
    if build.returncode != 0:
        print(build.stderr)
        sys.exit(1)
    renderer = ROOT / "target" / "release" / "examples" / "render_svg"
    names = args.cases or [p.stem for p in sorted(CASES.glob("*.html"))]
    results = []
    with sync_playwright() as playwright:
        exe = os.environ.get("CHROMIUM") or next(iter(sorted(glob.glob("/opt/pw-browsers/chromium-*/chrome-linux/chrome"))), None)
        browser = playwright.chromium.launch(headless=True, executable_path=exe, args=["--disable-lcd-text", "--font-render-hinting=none", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"])
        page = browser.new_page(viewport={"width": WIDTH, "height": HEIGHT}, device_scale_factor=1)
        for name in names:
            case = CASES / f"{name}.html"
            svg = OUTPUT / f"{name}.svg"
            reference = OUTPUT / f"{name}-reference.png"
            actual = OUTPUT / f"{name}-actual.png"
            difference = OUTPUT / f"{name}-diff.png"
            side = OUTPUT / f"{name}-side.png"
            run = subprocess.run([str(renderer), str(case), str(svg), str(WIDTH), str(HEIGHT)], cwd=ROOT, capture_output=True, text=True)
            if run.returncode != 0:
                print(f"{name}: renderer failed\n{run.stderr}", file=sys.stderr)
                continue
            if run.stderr.strip():
                print(f"{name}: {run.stderr.strip()}", file=sys.stderr)
            page.goto(case.as_uri())
            page.wait_for_timeout(50)
            doc_h = max(HEIGHT, int(page.evaluate("document.documentElement.scrollHeight")))
            page.set_viewport_size({"width": WIDTH, "height": doc_h})
            page.screenshot(path=str(reference))
            page.goto(svg.as_uri())
            page.screenshot(path=str(actual))
            page.set_viewport_size({"width": WIDTH, "height": HEIGHT})
            with Image.open(reference) as ref, Image.open(actual) as got:
                ref = ref.convert("RGB")
                got = got.convert("RGB")
                w = max(ref.width, got.width)
                h = max(ref.height, got.height)
                canvas_a = Image.new("RGB", (w, h), "white")
                canvas_b = Image.new("RGB", (w, h), "white")
                canvas_a.paste(ref, (0, 0))
                canvas_b.paste(got, (0, 0))
                delta = ImageChops.difference(canvas_a, canvas_b)
                mismatch = sum(1 for px in delta.get_flattened_data() if px > 24) if False else sum(1 for px in delta.getdata() if max(px) > 24)
                percent = 100 * mismatch / (w * h)
                delta.save(difference)
                comp = Image.new("RGB", (w * 2 + 10, h), "#888")
                comp.paste(canvas_a, (0, 0))
                comp.paste(canvas_b, (w + 10, 0))
                comp.save(side)
                results.append((name, mismatch, percent, (ref.width, ref.height), (got.width, got.height)))
            print(f"{name:20} diff {percent:6.2f}%  chromium {results[-1][3]}  castilia {results[-1][4]}")
        browser.close()

    if args.no_report or args.cases:
        return
    lines = [
        "# Visual benchmark",
        "",
        f"Reference: Chromium at {WIDTH} x {HEIGHT} CSS pixels (full page). Each case is rendered from the same HTML by Chromium and by Castilia. A pixel is counted as different when any RGB channel differs by more than 24.",
        "",
        "| Case | Different pixels | Image area |",
        "|---|---:|---:|",
    ]
    for name, mismatch, percent, _, _ in results:
        lines.append(f"| {name} | {mismatch} | {percent:.2f}% |")
    lines += ["", "Generated images are in `visual_out/`. This is a focused visual regression suite, not a full browser conformance score."]
    (ROOT / "visual-report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
