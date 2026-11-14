#!/usr/bin/env python3
"""Fetch prior SAT Competition main tracks from GBD (bead SAT-playground-d96.11).

The structure censuses (d96.8.3, d96.9.3) look past the 2025 set at the main
tracks of earlier years.  The 2026 track is the holdout (plan section 8): no
2026 file is read here.  Its hashes and isomorphism hashes come from GBD's
metadata (benchmarks/gbd/meta.db, the getdatabase/meta download), and every
prior instance whose hash or isomorphism hash is also a 2026 instance's is
dropped.  Instances already in the 2025 set are not fetched again.

Per instance the table benchmarks/rl/prior_tracks.tsv records the hash, the
tracks it was in, GBD's family, author and result, the file name, where it
was stored and its status (fetched, in-2025, dropped-2026, dropped-2026-iso,
failed).  An instance in several prior tracks is stored once, under the
earliest year.  Files are named as the 2025 suite's (hash-filename, the name
GBD's content disposition gives), checked with xz -t, and fetched again on a
rerun only when missing or broken.

Usage:
  python3 tools/fetch_prior_tracks.py [--years 2020,2021,2022,2023,2024] [--workers 8] [--dry]
"""
from __future__ import annotations

import argparse
import csv
import sqlite3
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BENCH = ROOT / "benchmarks"
META = BENCH / "gbd" / "meta.db"
TABLE = BENCH / "rl" / "prior_tracks.tsv"
URL = "http://benchmark-database.de/file/{}"
FIELDS = ["hash", "tracks", "family", "author", "result", "filename", "file", "status", "bytes"]


def xz_ok(path):
    return path.exists() and path.stat().st_size > 0 and \
        subprocess.run(["xz", "-t", str(path)], capture_output=True).returncode == 0


def fetch(h, dest, tries=3):
    tmp = dest.with_suffix(dest.suffix + ".part")
    for k in range(tries):
        r = subprocess.run(["curl", "-s", "-f", "-L", "-m", "1800", "-o", str(tmp), URL.format(h)],
                           capture_output=True)
        if r.returncode == 0 and xz_ok(tmp):
            tmp.rename(dest)
            return True
        time.sleep(5 * (k + 1))
    if tmp.exists():
        tmp.unlink()
    return False


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--years", default="2020,2021,2022,2023,2024")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--dry", action="store_true", help="plan only: write the table, fetch nothing")
    a = ap.parse_args(argv)
    years = sorted(int(y) for y in a.years.split(","))
    if 2026 in years or 2025 in years:
        sys.exit("only prior years: 2025 is the training set and 2026 the holdout")
    if not META.exists():
        sys.exit(f"{META} missing: curl -o {META} https://benchmark-database.de/getdatabase/meta")
    db = sqlite3.connect(META)
    tracks = {}
    for h, t in db.execute("select hash, value from track"):
        tracks.setdefault(h, set()).add(t)
    feats = {r[0]: r[1:] for r in db.execute("select hash, family, author, result, isohash from features")}
    names = dict(db.execute("select hash, value from filename"))
    in26 = {h for h, ts in tracks.items() if "main_2026" in ts}
    iso26 = {feats[h][3] for h in in26 if h in feats and feats[h][3]}
    in25 = {h for h, ts in tracks.items() if "main_2025" in ts}
    iso25 = {feats[h][3] for h in in25 if h in feats and feats[h][3]}
    want = {}
    for y in years:
        hs = sorted(h for h, ts in tracks.items() if f"main_{y}" in ts)
        print(f"main_{y}: {len(hs)} instances in GBD's metadata")
        (BENCH / f"track_main_{y}.uri").write_text("".join(URL.format(h) + "\n" for h in hs))
        for h in hs:
            want.setdefault(h, y)            # the earliest year
    rows = []
    jobs = []
    for h, y in sorted(want.items()):
        fam, auth, res, iso = feats.get(h, ("", "", "", ""))
        name = names.get(h, "")
        row = {"hash": h, "tracks": ",".join(sorted(t for t in tracks.get(h, ()) if t.startswith("main_"))),
               "family": fam or "", "author": auth or "", "result": res or "", "filename": name,
               "file": "", "status": "", "bytes": ""}
        if h in in26:
            row["status"] = "dropped-2026"
        elif iso and iso in iso26:
            row["status"] = "dropped-2026-iso"
        elif h in in25 or (iso and iso in iso25):
            row["status"] = "in-2025"
        elif not name:
            row["status"] = "failed"          # no file name in the metadata
        else:
            dest = BENCH / f"sat-comp-{y}" / f"{h}-{name}"
            row["file"] = str(dest.relative_to(ROOT))
            jobs.append((row, dest))
        rows.append(row)
    print(f"{len(want)} distinct instances; {len(jobs)} to fetch; "
          f"{sum(r['status'] == 'dropped-2026' for r in rows)} dropped as 2026 instances, "
          f"{sum(r['status'] == 'dropped-2026-iso' for r in rows)} as isomorphic to one, "
          f"{sum(r['status'] == 'in-2025' for r in rows)} already in 2025")
    if not a.dry:
        for y in years:
            d = BENCH / f"sat-comp-{y}"
            d.mkdir(parents=True, exist_ok=True)
        todo = [(r, d) for r, d in jobs if not xz_ok(d)]
        for r, d in jobs:
            if (r, d) not in todo:
                r["status"] = "fetched"
        print(f"{len(jobs) - len(todo)} present and sound, fetching {len(todo)}", flush=True)
        with ThreadPoolExecutor(a.workers) as ex:
            futs = {ex.submit(fetch, r["hash"], d): (r, d) for r, d in todo}
            for k, fu in enumerate(as_completed(futs), 1):
                r, d = futs[fu]
                r["status"] = "fetched" if fu.result() else "failed"
                if k % 50 == 0 or k == len(todo):
                    print(f"  {k}/{len(todo)} done, {sum(x['status'] == 'failed' for x, _ in todo)} failed", flush=True)
        for r, d in jobs:
            if r["status"] == "fetched":
                r["bytes"] = d.stat().st_size
        for y in years:
            d = BENCH / f"sat-comp-{y}"
            mine = [r for r in rows if r["file"].startswith(f"benchmarks/sat-comp-{y}/") and r["status"] == "fetched"]
            (d / "MANIFEST.txt").write_text(
                f"# SAT Competition {y} - Main Track instances (bead SAT-playground-d96.11)\n"
                f"# Source: GBD, track=main_{y}; tools/fetch_prior_tracks.py; benchmarks/rl/prior_tracks.tsv\n"
                f"# An instance in several tracks is stored under its earliest year; instances that\n"
                f"# are also in the 2026 holdout (by hash or isomorphism hash) are not fetched.\n"
                f"# Fetched {time.strftime('%Y-%m-%d')}: {len(mine)} files. Not committed to git.\n#\n"
                + "".join(Path(r["file"]).name + "\n" for r in mine))
    else:
        for r, d in jobs:
            r["status"] = "planned"
    TABLE.parent.mkdir(parents=True, exist_ok=True)
    with open(TABLE, "w", newline="") as f:
        w = csv.DictWriter(f, FIELDS, delimiter="\t")
        w.writeheader()
        w.writerows(rows)
    print(f"wrote {TABLE.relative_to(ROOT)}: " + ", ".join(
        f"{s} {sum(r['status'] == s for r in rows)}" for s in sorted({r['status'] for r in rows})))


if __name__ == "__main__":
    main()
