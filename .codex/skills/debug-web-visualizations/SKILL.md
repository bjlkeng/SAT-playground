---
name: debug-web-visualizations
description: Review, debug, and fix interactive web visualizations or static HTML documentation pages by opening them in a real browser, clicking through step/play/reset controls, comparing visual state with supporting text, checking responsive layout and overflow, and reporting precise issues. Use for tasks involving walkthroughs, diagrams, charts, Cytoscape/Canvas/SVG/HTML visualizations, solver pages, or any request to inspect a web visualization carefully in a browser.
---

# Debug Web Visualizations

## Workflow

1. Read the source first.
   - Identify the page entry point, scripts, data model, step definitions, controls, and expected semantic story.
   - Read nearby docs or implementation notes when correctness depends on domain facts.

2. Exercise the page in a real browser.
   - Prefer `scripts/browser_walkthrough.py` when Firefox and `geckodriver` are available.
   - Capture DOM state and screenshots at every step.
   - Click `Step` through all states; also test `Back`, `Reset`, and `Play` when present.

3. Inspect screenshots, not just DOM text.
   - Use `view_image` on captured screenshots.
   - Look for overlaps, clipped labels, hidden controls, blank canvases, misleading highlights, unreadable contrast, and mismatched legends.
   - Run at least one desktop viewport and one mobile/narrow viewport.

4. Compare visualization against the supporting text.
   - Check that every caption, legend, highlighted element, and side panel describes the same state.
   - Verify temporal consistency: a clause, conflict, selected node, or result should not be shown before the step says it exists.
   - Verify domain semantics, not only UI mechanics. For SAT/CDCL pages, check clause truth values, decision levels, reason clauses, learned clauses, UIP wording, and backjump/assert behavior.

5. Report or fix.
   - For a review request, lead with findings ordered by severity and include file/line references.
   - For a fix request, patch only the relevant page/CSS/script, then rerun browser checks.
   - Leave unrelated working-tree changes untouched.

## Browser Script

Run from the repo root:

```bash
python3 .codex/skills/debug-web-visualizations/scripts/browser_walkthrough.py \
  --file docs/solvers/02-cdcl.html \
  --root '#cdcl-demo' \
  --steps 6 \
  --out /tmp/solver-02-walkthrough
```

Useful options:

- `--url https://...` instead of `--file path/to/page.html`
- `--capture name=selector` to add page-specific text captures
- `--desktop-size 1440x1100`
- `--mobile-size 430x1200`
- `--mobile-step 4`
- `--no-mobile` to skip responsive checks

The script writes `states.json`, desktop screenshots, and a mobile screenshot under `--out`.

## Validation Checks

For static pages with inline scripts, extract and syntax-check scripts when practical:

```bash
node -e "const fs=require('fs'); const html=fs.readFileSync('docs/solvers/02-cdcl.html','utf8'); const s=html.indexOf('<script>')+8; const e=html.indexOf('</script>', s); new Function(html.slice(s,e));"
```

For layout changes, confirm:

- no browser console/runtime errors from the page load
- all expected steps are reachable
- buttons have sensible disabled states at the first and last step
- `document.documentElement.scrollWidth` does not exceed the viewport/page width unexpectedly
- screenshots show the intended visual state without overlap or clipping
