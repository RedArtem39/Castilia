"""Build the local real-world-style corpus in corpus/ (git-ignored).

Real websites cannot be fetched from every environment, so the corpus is made of pages built
on top of popular CSS frameworks installed from npm (Bootstrap, Bulma, Tailwind, Pure,
github-markdown-css, normalize.css) plus hand-written article/dashboard pages. They are
compared against Chromium with tools/visual_bench.py --dir corpus.
"""
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "corpus"


def run(cmd, **kw):
    subprocess.run(cmd, cwd=OUT, check=True, **kw)


def main():
    OUT.mkdir(exist_ok=True)
    if not (OUT / "node_modules").exists():
        (OUT / "package.json").write_text('{"name":"corpus","private":true}')
        run(["npm", "install", "--silent", "bootstrap@5", "bulma", "github-markdown-css", "marked", "normalize.css", "purecss", "tailwindcss@3"])
    for page in (ROOT / "tools" / "corpus").glob("*.html"):
        shutil.copy2(page, OUT / page.name)
    # Tailwind: generate the utility CSS for the template
    (OUT / "tailwind.config.js").write_text("module.exports = { content: ['./tailwind-page.html'], theme: { extend: {} }, plugins: [] }")
    (OUT / "tw-in.css").write_text("@tailwind base;\n@tailwind components;\n@tailwind utilities;\n")
    run(["npx", "tailwindcss", "-i", "tw-in.css", "-o", "tailwind.css", "--minify"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    # GitHub README: render a real README with marked
    readme = (OUT / "node_modules" / "marked" / "README.md").read_text(encoding="utf-8")
    script = "const {marked}=require('marked');process.stdout.write(marked.parse(require('fs').readFileSync(0,'utf8')))"
    html = subprocess.run(["node", "-e", script], cwd=OUT, input=readme, capture_output=True, text=True, check=True).stdout
    page = (OUT / "github-readme.html").read_text(encoding="utf-8").replace("@@README@@", html)
    (OUT / "github-readme.html").write_text(page, encoding="utf-8")
    print("corpus ready:", ", ".join(sorted(p.name for p in OUT.glob("*.html"))))


if __name__ == "__main__":
    main()
