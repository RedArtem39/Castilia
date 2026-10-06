"""Save Chromium/Castilia comparisons for a small set of real websites.

Live pages change over time, so pixel differences are diagnostic rather than a
pass/fail compatibility score. The Rust live benchmark provides the hard gate.
"""

from pathlib import Path
import os
import subprocess

from PIL import Image, ImageChops
from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "live_visual_out"
WIDTH = 800
HEIGHT = 600
SITES = [
    ("rust", "https://www.rust-lang.org/"),
    ("w3c", "https://www.w3.org/"),
    ("hacker-news", "https://news.ycombinator.com/"),
]


def main() -> None:
    OUTPUT.mkdir(exist_ok=True)
    subprocess.run(
        ["cargo", "build", "--quiet", "--release", "--features", "host-net", "--example", "browse"],
        cwd=ROOT,
        check=True,
    )
    suffix = ".exe" if os.name == "nt" else ""
    renderer = ROOT / "target" / "release" / "examples" / f"browse{suffix}"
    results = []

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": WIDTH, "height": HEIGHT}, device_scale_factor=1)
        page.emulate_media(reduced_motion="reduce")
        for name, url in SITES:
            svg = OUTPUT / f"{name}.svg"
            reference = OUTPUT / f"{name}-chromium.png"
            actual = OUTPUT / f"{name}-castilia.png"
            comparison = OUTPUT / f"{name}-comparison.png"
            subprocess.run(
                [str(renderer), url, str(svg)], cwd=ROOT, check=True,
                capture_output=True, text=True, timeout=60,
            )
            page.goto(url, wait_until="domcontentloaded", timeout=30000)
            page.screenshot(path=str(reference), animations="disabled")
            page.goto(svg.as_uri())
            page.screenshot(path=str(actual), animations="disabled")

            with Image.open(reference) as ref, Image.open(actual) as got:
                ref = ref.convert("RGB")
                got = got.convert("RGB")
                delta = ImageChops.difference(ref, got)
                mismatch = sum(any(channel > 5 for channel in pixel) for pixel in delta.get_flattened_data())
                percent = 100 * mismatch / (WIDTH * HEIGHT)
                pair = Image.new("RGB", (WIDTH * 2, HEIGHT), "white")
                pair.paste(ref, (0, 0))
                pair.paste(got, (WIDTH, 0))
                pair.save(comparison)
            results.append((name, url, percent))
        browser.close()

    lines = [
        "# Live visual comparison", "",
        "Chromium is on the left; Castilia is on the right. Both use an 800x600 viewport. Live content, fonts and network responses can differ between captures. Pixel difference is diagnostic, not a browser compatibility score.",
        "", "| Site | Different pixels | Comparison |", "|---|---:|---|",
    ]
    for name, url, percent in results:
        lines.append(f"| [{name}]({url}) | {percent:.2f}% | [open](live_visual_out/{name}-comparison.png) |")
    lines.append("")
    (ROOT / "live-visual-report.md").write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
