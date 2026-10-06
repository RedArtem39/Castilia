"""Compare Castilia output with Chromium on small, deterministic HTML reftests."""

from pathlib import Path
import subprocess
import shutil

from PIL import Image, ImageChops
from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[1]
CASES = ROOT / "visual_cases"
OUTPUT = ROOT / "visual_out"
WIDTH = 800
HEIGHT = 300


def main() -> None:
    OUTPUT.mkdir(exist_ok=True)
    for asset in CASES.glob("*.png"):
        shutil.copy2(asset, OUTPUT / asset.name)
    subprocess.run(["cargo", "build", "--quiet", "--release", "--example", "render_svg"], cwd=ROOT, check=True)
    renderer = ROOT / "target" / "release" / "examples" / "render_svg.exe"
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": WIDTH, "height": HEIGHT}, device_scale_factor=1)
        results = []
        for case in sorted(CASES.glob("*.html")):
            svg = OUTPUT / f"{case.stem}.svg"
            reference = OUTPUT / f"{case.stem}-reference.png"
            actual = OUTPUT / f"{case.stem}-actual.png"
            difference = OUTPUT / f"{case.stem}-diff.png"
            subprocess.run([str(renderer), str(case), str(svg)], cwd=ROOT, check=True, capture_output=True)
            page.goto(case.as_uri())
            page.screenshot(path=str(reference))
            page.goto(svg.as_uri())
            page.screenshot(path=str(actual))
            with Image.open(reference) as ref, Image.open(actual) as got:
                delta = ImageChops.difference(ref.convert("RGB"), got.convert("RGB"))
                mismatch = sum(any(channel > 5 for channel in pixel) for pixel in delta.get_flattened_data())
                percent = 100 * mismatch / (WIDTH * HEIGHT)
                delta.save(difference)
                results.append((case.stem, mismatch, percent))
        browser.close()

    lines = ["# Visual benchmark", "", "Reference: Chromium at 800 x 300 CSS pixels. Each case is rendered from the same HTML by Chromium and Castilia. A pixel is counted as different when any RGB channel differs by more than 5.", "", "| Case | Different pixels | Image area |", "|---|---:|---:|"]
    for name, mismatch, percent in results:
        lines.append(f"| {name} | {mismatch} | {percent:.2f}% |")
    lines += ["", "Generated images are in `visual_out/`. This is a focused visual regression suite, not a full browser conformance score."]
    (ROOT / "visual-report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
