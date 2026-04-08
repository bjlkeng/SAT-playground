#!/usr/bin/env python3
"""Verify that a SAT solver's assignment satisfies a CNF formula.

Usage: python3 verify_sat.py <cnf_file> <solver_stdout_file>
       echo "<solver_output>" | python3 verify_sat.py <cnf_file> -

Exit code 0 = assignment satisfies all clauses.
Exit code 1 = verification failure (details on stderr).

The CNF file may be gzip-compressed (.cnf.gz).
"""

import gzip
import sys


def parse_cnf(path):
    """Parse DIMACS CNF file, return list of clauses (each a list of ints)."""
    opener = gzip.open if path.endswith(".gz") else open
    clauses = []
    current = []
    with opener(path, "rt") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("c") or line.startswith("p"):
                continue
            for tok in line.split():
                lit = int(tok)
                if lit == 0:
                    if current:
                        clauses.append(current)
                        current = []
                else:
                    current.append(lit)
    if current:
        clauses.append(current)
    return clauses


def parse_assignment(source):
    """Parse v-lines from solver output, return set of true literals."""
    lits = set()
    for line in source:
        line = line.strip()
        if not line.startswith("v"):
            continue
        for tok in line.split():
            if tok == "v":
                continue
            lit = int(tok)
            if lit == 0:
                continue
            if -lit in lits:
                print(f"VERIFY FAIL: contradictory assignment for variable {abs(lit)}", file=sys.stderr)
                sys.exit(1)
            lits.add(lit)
    if not lits:
        print("VERIFY FAIL: no v-lines found in solver output", file=sys.stderr)
        sys.exit(1)
    return lits


def verify(clauses, true_lits):
    """Check every clause is satisfied. Return (ok, first_failing_clause_index)."""
    for i, clause in enumerate(clauses):
        if not any(lit in true_lits for lit in clause):
            return False, i
    return True, -1


def main():
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        sys.exit(2)

    cnf_path = sys.argv[1]
    output_path = sys.argv[2]

    clauses = parse_cnf(cnf_path)

    if output_path == "-":
        true_lits = parse_assignment(sys.stdin)
    else:
        with open(output_path) as f:
            true_lits = parse_assignment(f)

    ok, fail_idx = verify(clauses, true_lits)
    if ok:
        print("VERIFIED")
    else:
        clause = clauses[fail_idx]
        print(f"VERIFY FAIL: clause {fail_idx + 1} not satisfied: ({' '.join(str(l) for l in clause)})", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
