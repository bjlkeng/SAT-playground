## PAR-2 per config (300 s timeout, profiling suite)

| Config | Solved | Timeout | PAR-2 | Δ vs A % |
|---|---:|---:|---:|---:|
| A_baseline | 10 | 0 | 842.3 | +0.0% |
| B_binary_fast | 10 | 0 | 1172.1 | +39.2% |
| C_lbd_tiered | 8 | 2 | 1887.0 | +124.0% |
| D_post_reset | 10 | 0 | 755.2 | -10.3% |
| E_reuse_trail | 7 | 3 | 2326.4 | +176.2% |
| F_combined_kissat | 7 | 3 | 2555.3 | +203.4% |

## Per-instance wall time (s)

| Instance | baseline | binary_fast | lbd_tiered | post_reset | reuse_trail | combined_kissat |
|---|---:|---:|---:|---:|---:|---:|
| sudoku | 232.8 | 219.1 | 192.4 | 191.5 | TIMEOUT | TIMEOUT |
| 6s299b685 | 17.8 | 18.0 | 16.5 | 16.0 | 87.3 | 101.8 |
| REGRandom | 59.7 | 60.7 | 164.2 | 56.9 | 262.1 | TIMEOUT |
| mp1 | 44.9 | 225.4 | TIMEOUT | 42.3 | 0.7 | 202.4 |
| Kakuro | 241.0 | 164.3 | 119.0 | 210.3 | 51.5 | 295.3 |
| SCPC | 13.9 | 13.5 | 17.4 | 13.7 | 29.5 | 42.5 |
| velev | 71.4 | 199.7 | 35.9 | 66.0 | 80.9 | 64.9 |
| brocard | 9.3 | 10.3 | 8.7 | 8.7 | 14.4 | 13.8 |
| battleship | 23.2 | 129.6 | 132.9 | 23.0 | TIMEOUT | 34.6 |
| case9 | 128.4 | 131.4 | TIMEOUT | 126.9 | TIMEOUT | TIMEOUT |

## Work × Speed decomposition (vs A_baseline)

Legend: work = conflicts_cfg / conflicts_A, speed = (props/s)_A / (props/s)_cfg, net = work × speed (predicted wall ratio).

| Instance | Config | conflicts | props/s | work | speed | net | measured | dominant |
|---|---|---:|---:|---:|---:|---:|---:|---|
| sudoku | B_binary_fast | 230401 | 5254845 | 0.89 | 1.07 | 0.95 | 0.94 | work |
| sudoku | C_lbd_tiered | 249000 | 7204447 | 0.96 | 0.78 | 0.75 | 0.83 | speed |
| sudoku | D_post_reset | 259775 | 6853748 | 1.00 | 0.82 | 0.82 | 0.82 | speed |
| _A_baseline_ | A_baseline | 259775 | 5638492 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| 6s299b685 | B_binary_fast | 5479 | 1428514 | 1.46 | 1.11 | 1.61 | 1.01 | WORK |
| 6s299b685 | C_lbd_tiered | 3764 | 1702673 | 1.00 | 0.93 | 0.93 | 0.93 | noise |
| 6s299b685 | D_post_reset | 3764 | 1756433 | 1.00 | 0.90 | 0.90 | 0.90 | speed |
| 6s299b685 | E_reuse_trail | 5097 | 309608 | 1.35 | 5.10 | 6.91 | 4.91 | SPEED |
| 6s299b685 | F_combined_kissat | 11899 | 627669 | 3.16 | 2.52 | 7.96 | 5.73 | WORK |
| _A_baseline_ | A_baseline | 3764 | 1580355 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| REGRandom | B_binary_fast | 1411240 | 425653 | 0.88 | 1.19 | 1.04 | 1.02 | SPEED |
| REGRandom | C_lbd_tiered | 4505032 | 495900 | 2.80 | 1.02 | 2.86 | 2.75 | WORK |
| REGRandom | D_post_reset | 1607608 | 530654 | 1.00 | 0.95 | 0.95 | 0.95 | noise |
| REGRandom | E_reuse_trail | 4938263 | 344728 | 3.07 | 1.47 | 4.51 | 4.39 | WORK |
| _A_baseline_ | A_baseline | 1607608 | 506298 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| mp1 | B_binary_fast | 1742302 | 5562921 | 4.10 | 1.16 | 4.76 | 5.02 | WORK |
| mp1 | D_post_reset | 425229 | 6856967 | 1.00 | 0.94 | 0.94 | 0.94 | noise |
| mp1 | E_reuse_trail | 13681 | 5825947 | 0.03 | 1.11 | 0.04 | 0.02 | work |
| mp1 | F_combined_kissat | 2961839 | 4731243 | 6.97 | 1.37 | 9.51 | 4.51 | WORK |
| _A_baseline_ | A_baseline | 425229 | 6461247 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| Kakuro | B_binary_fast | 415769 | 2156213 | 0.57 | 1.19 | 0.67 | 0.68 | work |
| Kakuro | C_lbd_tiered | 367536 | 2573682 | 0.50 | 1.00 | 0.50 | 0.49 | work |
| Kakuro | D_post_reset | 732107 | 2937118 | 1.00 | 0.87 | 0.87 | 0.87 | speed |
| Kakuro | E_reuse_trail | 25040 | 517736 | 0.03 | 4.95 | 0.17 | 0.21 | SPEED |
| Kakuro | F_combined_kissat | 672532 | 2142515 | 0.92 | 1.20 | 1.10 | 1.23 | SPEED |
| _A_baseline_ | A_baseline | 732107 | 2562769 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| SCPC | B_binary_fast | 188144 | 1021702 | 1.00 | 0.98 | 0.98 | 0.98 | noise |
| SCPC | C_lbd_tiered | 178890 | 749726 | 0.95 | 1.33 | 1.27 | 1.25 | SPEED |
| SCPC | D_post_reset | 188144 | 1011909 | 1.00 | 0.99 | 0.99 | 0.99 | noise |
| SCPC | E_reuse_trail | 309987 | 745695 | 1.65 | 1.34 | 2.20 | 2.13 | WORK |
| SCPC | F_combined_kissat | 311567 | 511753 | 1.66 | 1.95 | 3.23 | 3.07 | SPEED |
| _A_baseline_ | A_baseline | 188144 | 997819 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| velev | B_binary_fast | 470505 | 5571186 | 2.61 | 1.10 | 2.87 | 2.80 | WORK |
| velev | C_lbd_tiered | 48682 | 2897203 | 0.27 | 2.11 | 0.57 | 0.50 | SPEED |
| velev | D_post_reset | 179968 | 6608587 | 1.00 | 0.92 | 0.92 | 0.92 | noise |
| velev | E_reuse_trail | 216677 | 6261269 | 1.20 | 0.98 | 1.18 | 1.13 | WORK |
| velev | F_combined_kissat | 70185 | 2605525 | 0.39 | 2.35 | 0.91 | 0.91 | SPEED |
| _A_baseline_ | A_baseline | 179968 | 6112064 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| brocard | B_binary_fast | 513 | 2463518 | 1.27 | 0.98 | 1.25 | 1.11 | WORK |
| brocard | C_lbd_tiered | 403 | 2599881 | 1.00 | 0.93 | 0.93 | 0.93 | noise |
| brocard | D_post_reset | 403 | 2611296 | 1.00 | 0.93 | 0.93 | 0.93 | noise |
| brocard | E_reuse_trail | 673 | 3707772 | 1.67 | 0.65 | 1.09 | 1.54 | WORK |
| brocard | F_combined_kissat | 685 | 3358011 | 1.70 | 0.72 | 1.23 | 1.48 | WORK |
| _A_baseline_ | A_baseline | 403 | 2422635 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| battleship | B_binary_fast | 2732102 | 455825 | 4.61 | 1.23 | 5.66 | 5.58 | WORK |
| battleship | C_lbd_tiered | 3679927 | 592536 | 6.21 | 0.94 | 5.86 | 5.72 | WORK |
| battleship | D_post_reset | 593019 | 566140 | 1.00 | 0.99 | 0.99 | 0.99 | noise |
| battleship | F_combined_kissat | 1122732 | 787029 | 1.89 | 0.71 | 1.35 | 1.49 | WORK |
| _A_baseline_ | A_baseline | 593019 | 559562 | 1.00 | 1.00 | 1.00 | 1.00 | -- |
| case9 | B_binary_fast | 4186969 | 3319655 | 1.00 | 1.02 | 1.02 | 1.02 | noise |
| case9 | D_post_reset | 4186969 | 3437920 | 1.00 | 0.99 | 0.99 | 0.99 | noise |
| _A_baseline_ | A_baseline | 4186969 | 3398358 | 1.00 | 1.00 | 1.00 | 1.00 | -- |

