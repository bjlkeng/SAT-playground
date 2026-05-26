#!/usr/bin/env python3
"""Shuffle DIMACS CNF clause and literal order without changing semantics."""

from __future__ import annotations

import argparse
import lzma
import random
import sys
import tempfile
from pathlib import Path


def open_text(path: Path, mode: str):
    if path.suffix == ".xz":
        return lzma.open(path, mode + "t", encoding="utf-8")
    return path.open(mode, encoding="utf-8")


def parse_dimacs(path: Path) -> tuple[list[str], int, list[list[int]]]:
    comments: list[str] = []
    num_vars: int | None = None
    expected_clauses: int | None = None
    clauses: list[list[int]] = []
    current: list[int] = []

    with open_text(path, "r") as fh:
        for lineno, line in enumerate(fh, 1):
            stripped = line.strip()
            if not stripped:
                continue
            if stripped.startswith("c"):
                comments.append(stripped)
                continue
            if stripped.startswith("p"):
                parts = stripped.split()
                if len(parts) != 4 or parts[1] != "cnf":
                    raise ValueError(f"{path}:{lineno}: expected 'p cnf <vars> <clauses>'")
                num_vars = int(parts[2])
                expected_clauses = int(parts[3])
                continue

            for token in stripped.split():
                lit = int(token)
                if lit == 0:
                    clauses.append(current)
                    current = []
                else:
                    current.append(lit)

    if current:
        raise ValueError(f"{path}: unterminated final clause")
    if num_vars is None:
        num_vars = max((abs(lit) for clause in clauses for lit in clause), default=0)
    if expected_clauses is not None and expected_clauses != len(clauses):
        raise ValueError(
            f"{path}: header declares {expected_clauses} clauses, parsed {len(clauses)}"
        )
    return comments, num_vars, clauses


def write_dimacs(
    path: Path,
    comments: list[str],
    num_vars: int,
    clauses: list[list[int]],
    *,
    source: Path,
    seed: int,
    shuffle_clauses: bool,
    shuffle_literals: bool,
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open_text(path, "w") as fh:
        for comment in comments:
            fh.write(comment)
            fh.write("\n")
        fh.write(
            "c shuffled_by=tools/shuffle_cnf.py "
            f"seed={seed} shuffle_clauses={int(shuffle_clauses)} "
            f"shuffle_literals={int(shuffle_literals)} source={source}\n"
        )
        fh.write(f"p cnf {num_vars} {len(clauses)}\n")
        for clause in clauses:
            if clause:
                fh.write(" ".join(str(lit) for lit in clause))
                fh.write(" 0\n")
            else:
                fh.write("0\n")


def shuffled(
    clauses: list[list[int]],
    rng: random.Random,
    *,
    shuffle_clauses: bool,
    shuffle_literals: bool,
) -> list[list[int]]:
    result = [list(clause) for clause in clauses]
    if shuffle_literals:
        for clause in result:
            rng.shuffle(clause)
    if shuffle_clauses:
        rng.shuffle(result)
    return result


def run_self_test() -> None:
    sample = """c sample
p cnf 4 4
1 -2 3 0
4
0
1 2
3 0
-1 0
"""
    with tempfile.TemporaryDirectory() as tmp:
        src = Path(tmp) / "sample.cnf"
        dst = Path(tmp) / "shuffled.cnf"
        src.write_text(sample, encoding="utf-8")
        comments, num_vars, clauses = parse_dimacs(src)
        shuffled_clauses = shuffled(
            clauses,
            random.Random(7),
            shuffle_clauses=True,
            shuffle_literals=True,
        )
        write_dimacs(
            dst,
            comments,
            num_vars,
            shuffled_clauses,
            source=src,
            seed=7,
            shuffle_clauses=True,
            shuffle_literals=True,
        )
        _, parsed_vars, parsed_shuffled = parse_dimacs(dst)

    assert parsed_vars == 4
    assert sorted(sorted(clause) for clause in parsed_shuffled) == sorted(
        sorted(clause) for clause in [[1, -2, 3], [4], [1, 2, 3], [-1]]
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", nargs="?", type=Path)
    parser.add_argument("output", nargs="?", type=Path)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--no-shuffle-clauses", action="store_true")
    parser.add_argument("--no-shuffle-literals", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        run_self_test()
        return 0
    if args.input is None or args.output is None:
        parser.error("input and output are required unless --self-test is used")

    comments, num_vars, clauses = parse_dimacs(args.input)
    shuffle_clauses = not args.no_shuffle_clauses
    shuffle_literals = not args.no_shuffle_literals
    shuffled_clauses = shuffled(
        clauses,
        random.Random(args.seed),
        shuffle_clauses=shuffle_clauses,
        shuffle_literals=shuffle_literals,
    )
    write_dimacs(
        args.output,
        comments,
        num_vars,
        shuffled_clauses,
        source=args.input,
        seed=args.seed,
        shuffle_clauses=shuffle_clauses,
        shuffle_literals=shuffle_literals,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
