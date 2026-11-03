# Count and alldifferent lemmas: plan (2026-11-01, epic SAT-playground-iin)

Owner's decision (bead `iin.3`, 2026-11-01): cover every form of count and every
variation of alldifferent. Extract them, prove them, add lemmas where they help,
with effort guardrails. Experiment with the guardrails and recommend a setting.
This overrides the earlier suggestion to park the work.

## The idea in plain terms

Both problems are the same sum. A count says "at least d of these literals". A
clause is d = 1. "At most K of these" is "at least n - K of their negations". An
alldifferent column is K = 1. If some constraints want more trues than others
allow, adding them up gives a contradiction. Unit propagation cannot see this,
because it reads one clause at a time.

So there is one engine and one kind of constraint:

- **Card**: at least `need` of `lits`, with a proof id.
- **Network**: some Cards used as rows (they want trues) and some used as caps
  (they allow few trues). A row literal and the same variable's opposite literal
  in a cap cancel when summed.

At a search state, a matching between rows and caps picks a set of rows R and a
set of caps V that look too tight. The matching is only a guess at what to sum.
The soundness comes from arithmetic on the sum itself:

1. S = the sum of R and V, with opposite literals cancelled, is a linear constraint
   implied by the formula.
2. Literals of S that are false now form the lemma. If everything else in S
   together cannot reach its right-hand side, the lemma is a contradiction.
3. If an open literal of S has a coefficient above the slack, it is forced.

The engine checks 2 and 3 exactly before it uses anything. The proof is the same
two facts for VeriPB: one `pol` line for S and one `rup` line for the clause.

## Forms of count to read

Each form needs a reader and a proof that its clauses imply the Card.

| Form | Proof route |
|---|---|
| Sinz sequential counter, standard, one way, both ways | One linear constraint per register, each from the ones before |
| Totalizer, full, capped, direct leaves, one way, both ways | One linear constraint per output, a chain over the splits |
| Decision diagram, one way, both ways, no third clause | Per node, over groups or the board (`structdd` has the group form) |
| Comparator, sorting and cardinality networks | Each comparator gives a + b <= max + min; the sum telescopes |
| Binary adders with a compare | Each adder gives a + b + c = s + 2 carry; the sum telescopes |
| Direct subset | Sums of the subsets divided up, one size at a time |
| Anything else with a known input set and bound | A solver sub-call against the opposite count, replayed as `rup` lines |

## Variations of alldifferent to read

Rows: exact-one, plain at-least-one, guarded, partial, any signs. Columns: pair
clauses, ladder, commander or any other exclusion unit propagation can show, and
capacities above one (a global cardinality constraint). Tight grids also give a
clause per column: if n rows need n columns, every column is used.

The reader goes by behaviour, as `tools/alldiff_census.py` does.

## Delivery

The earlier test added long clauses at level 0 at restarts, and they did little.
A lemma is more useful where it arises. So the engine runs at a propagation
fixpoint: a contradiction goes to conflict analysis, which learns a short clause
from it, and a forced literal is assigned with the lemma as its reason.

## Guardrails to test

| Guardrail | What it limits |
|---|---|
| Check interval | Conflicts between checks |
| Lemma length | Longest lemma used |
| Lemmas per check | How many a check may add |
| Effort share | Engine ticks as a share of search ticks |
| Read budget | Ticks for finding constraints |
| Proof budget | Proof lines for proving a Card, and in total |
| Yield | Stops when lemmas per tick fall too low |

## Work order

1. Prototype engine fed from the Python tools, no proofs: does it help, and which
   settings matter? (`iin.6`)
2. Engine in the solver with proofs, option off by default. (`iin.4`)
3. Rust readers and proofs for every count form (`iin.7`) and alldifferent
   variation (`iin.8`), each with toys, fuzz through the gate, and budget tests.
4. Guardrail experiments on the formulas where the engine fires, then a
   recommended setting. (`iin.9`)
5. The 400-cell paired run at two seeds with that setting. (`iin.5`) Promotion is
   the owner's call, and the 2026 holdout is not consulted without it.

A setting of "off" is a valid recommendation if the lemmas do not help. Results go
in the solver README, "Count and alldifferent lemmas".

## Where it stands

Steps 1 to 5 are done (2026-11-02). The engine, seven count forms and every
alldifferent variation are in the solver with proofs, option `card`, off by default.
The paired run (step 5) solved 645 against 651 over two seeds, with both PAR-2s about
2% worse. Eight fresh seeds on the 47 cells where the engine changes the search, under
a work budget, show that loss was luck: 346 against 341. The read before search costs
0.3% of the PAR-2 sum, and the search runs the same per conflict. So the engine is
neutral, and the recommended setting stays off. For a formula known to be made of counts,
`--card=1` with the options' defaults is the setting to use. The experiments and the
run are in the solver README. What was
planned and is not built, and why, is under "Measured and rejected" in that section;
the sub-call route for "anything else with a known input set" was not needed, because
every form the census reads as a plain count has its own proof.
