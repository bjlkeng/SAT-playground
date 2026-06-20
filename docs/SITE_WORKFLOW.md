# Static Site Workflow

The repo has a static benchmark site under `docs/`, deployed at:

- Live page: `https://bjlkeng.io/SAT-playground/`
- Main page source: `docs/index.html`
- Generated benchmark data: `docs/data/medium-par2.json`
- Generated README chart asset: `docs/assets/medium-cumulative.svg`
- Solver detail pages: `docs/solvers/*.html`
- Shared solver-page stylesheet: `docs/solver-pages.css`

## Content Conventions

- The hero title is `Fun with Boolean SAT`.
- The intro paragraph should:
  - include one sentence describing Boolean SAT with a link to Wikipedia;
  - say the goal is to understand SAT solvers more deeply;
  - mention that all code in the repo was generated with AI coding tools.
- Benchmark language currently refers to:
  - 100 randomly selected instances from the SAT Competition 2025 main-track set;
  - local benchmark limits of 1800 seconds and 16 GB RAM;
  - SAT Competition 2025 output/proof format via the official output page.
- The main page theme is light grey / white with blue and red accents.
- Fixed-width / monospace text is intentional for labels, legends, and benchmark
  metadata.
- The Solver Information cards should use `infoUrl` for detail pages or
  external references and `sourceUrl` for source buttons.
- Source buttons should point to GitHub directory `tree` URLs, not `blob` URLs.
- The small machine footnote currently describes this host:
  - AMD Ryzen 5 5600, 6 cores / 12 threads;
  - 62 GiB RAM reported by `free -h`.

## Generated Data

Regenerate the site payload and README chart with:

```bash
python3 tools/build_site_data.py
```

That script is the canonical source for:

- latest matching medium run per solver;
- benchmark metadata shown in site notes;
- the `SOLVERS` list, including local solver detail page paths;
- `infoUrl` and `sourceUrl` used by Solver Information cards;
- the static SVG chart embedded near the top of `README.md`.

The site uses the latest available run per solver matching 100 instances and an
1800 second timeout. If a local solver has no matching run, the generator omits
it from plotted output until matching benchmark data exists.

## Updating Solver Detail Pages

When adding or revising local solver pages:

- Keep them as simple static HTML pages under `docs/solvers/`.
- Include a brief description of the implemented technique.
- Link to Wikipedia, papers, or references where appropriate.
- Include high-level pseudocode.
- Include a short code-level optimization diffs section derived from the solver
  README.
- Use existing pages under `docs/solvers/` as the style baseline.

## README Integration

`README.md` links to the live site and embeds
`docs/assets/medium-cumulative.svg` near the top.

If benchmark sample, chart styling, or site framing changes materially, update:

1. `docs/index.html`
2. `tools/build_site_data.py`
3. `README.md`

## Verification

At minimum:

```bash
python3 tools/build_site_data.py

python3 - <<'PY'
from pathlib import Path
html = Path('docs/index.html').read_text()
start = html.index('<script>') + len('<script>')
end = html.index('</script>', start)
Path('/tmp/sat-playground-site.js').write_text(html[start:end].strip() + '\n')
PY
node --check /tmp/sat-playground-site.js
```

For visual or interactive changes, use the `debug-web-visualizations` skill and
check the page in a browser.

## Tracking Site Benchmark Logs

If the user wants exact benchmark logs committed for the site, track the exact
`summary.log` and `results.csv` used to generate charts. `log/` is ignored, so
use `git add -f` for those files.
