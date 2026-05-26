# AnalyzeSAT Preprocess/Order Matrix

## Config Summary

| config | rows | solved | status regressions | PAR-2 on measured rows | notes |
|---|---:|---:|---:|---:|---|
| default | 10 | 10 | 0 | 869.765 |  |
| no_bve | 1 | 0 | 1 | 600.000 | stopped at 0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12 (UNSAT -> UNKNOWN) |
| no_full_bsr | 3 | 2 | 1 | 794.416 | stopped at 46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized (UNSAT -> UNKNOWN) |
| no_simplification | 1 | 0 | 1 | 600.000 | stopped at 0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12 (UNSAT -> UNKNOWN) |
| input_order | 10 | 10 | 0 | 717.513 |  |
| raw_order | 10 | 10 | 0 | 687.588 |  |
| proof_off | 10 | 10 | 0 | 729.089 | diagnostic only: UNSAT rows violate proof requirement |

## Full-Suite Deltas vs Default

| config | PAR-2 | delta vs default | solved | largest win | largest loss |
|---|---:|---:|---:|---|---|
| input_order | 717.513 | -152.252 | 10/10 | 5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7 -205.753s | 6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7 +31.971s |
| raw_order | 687.588 | -182.177 | 10/10 | 5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7 -205.047s | 6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7 +44.637s |
| proof_off | 729.089 | -140.676 | 10/10 | 0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12 -52.560s | none +0.000s |

## Work x Speed Decomposition

work_ratio = candidate conflicts / default conflicts. speed_ratio = default propagation throughput / candidate propagation throughput, using search_sec. net = work_ratio * speed_ratio; wall_ratio = candidate bench time / default bench time.

| config | instance | wall | work | speed | net | conflicts default -> cfg | props/s default -> cfg |
|---|---|---:|---:|---:|---:|---:|---:|
| input_order | 5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7 | 0.196 | 0.051 | 0.693 | 0.035 | 732107 -> 37074 | 2888978 -> 4171176 |
| raw_order | 5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7 | 0.198 | 0.051 | 0.782 | 0.040 | 732107 -> 37074 | 2888978 -> 3692992 |
| input_order | 6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7 | 1.412 | 2.044 | 0.981 | 2.005 | 179968 -> 367810 | 9105789 -> 9283201 |
| raw_order | 6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7 | 1.576 | 2.044 | 1.151 | 2.351 | 179968 -> 367810 | 9105789 -> 7914377 |
| no_full_bsr | 46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized | 4.866 | 5.001 | 1.217 | 6.087 | 1607608 -> 8040136 | 587144 -> 482412 |
| proof_off | 0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12 | 0.778 | 1.000 | 0.777 | 0.777 | 259775 -> 259775 | 5655007 -> 7276388 |
| proof_off | 46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized | 0.839 | 1.000 | 0.853 | 0.853 | 1607608 -> 1607608 | 587144 -> 688149 |

## Preprocess Counters

| config | instance | preprocess_s | search_s | bve_vars | bsr_subsumed | final_original_clauses | final_original_lits | proof_bytes |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| default | 5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7 | 34.780 | 213.797 | 56214 | 4868640 | 14738579 | 52880887 | 738198813 |
| input_order | 5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7 | 35.086 | 8.278 | 56214 | 4868640 | 14742137 | 52891600 | 620757460 |
| raw_order | 5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7 | 35.021 | 9.350 | 56214 | 4868640 | 14742137 | 52891600 | 620757455 |
| default | 46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized | 9.105 | 51.458 | 512 | 0 | 823936 | 2748656 | 2074144226 |
| no_full_bsr | 46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized | 0.674 | 294.264 | 512 | 0 | 825216 | 3100416 | 8640562991 |
| proof_off | 0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12 | 2.888 | 180.369 | 430052 | 59003 | 1722657 | 5840034 | 0 |
