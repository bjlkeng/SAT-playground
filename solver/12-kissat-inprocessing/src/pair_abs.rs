use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const MAX_ABSTRACT_CLAUSE_PAIRS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AbsLitKey {
    pair: usize,
    positive: bool,
}

#[derive(Clone, Copy, Debug)]
struct PairBinding {
    pair: usize,
    dense: usize,
    fresh: i32,
}

#[derive(Clone, Debug)]
struct PairAbsGroup {
    lits: Vec<AbsLitKey>,
    original_clauses: Vec<Vec<i32>>,
    abstract_clause: Vec<i32>,
    lifted_clause: Vec<i32>,
}

#[derive(Clone, Debug)]
pub(crate) struct PairAbsFormula {
    pub(crate) abstract_num_vars: usize,
    pub(crate) abstract_clauses: Vec<Vec<i32>>,
    pub(crate) dense_to_fresh: Vec<i32>,
    bindings: Vec<PairBinding>,
    groups: Vec<PairAbsGroup>,
}

impl PairAbsFormula {
    pub(crate) fn pair_count(&self) -> usize {
        self.bindings.len()
    }

    pub(crate) fn abstract_clause_count(&self) -> usize {
        self.groups.len()
    }

    pub(crate) fn build_lift_proof(&self) -> Option<Vec<Vec<i32>>> {
        let mut proof = Vec::new();
        for binding in &self.bindings {
            let x = pair_x(binding.pair)? as i32;
            let y = x + 1;
            let p = binding.fresh;
            proof.push(canonical_clause(vec![-p, x, y])?);
            proof.push(canonical_clause(vec![-p, -x, -y])?);
            proof.push(canonical_clause(vec![p, x, -y])?);
            proof.push(canonical_clause(vec![p, -x, y])?);
        }

        for group in &self.groups {
            lift_group(group, &self.bindings, &mut proof)?;
        }
        Some(proof)
    }
}

pub(crate) fn extract_pair_abs_formula(
    clauses: &[Vec<i32>],
    num_vars: usize,
) -> Option<PairAbsFormula> {
    if clauses.is_empty() || num_vars == 0 || num_vars % 2 != 0 {
        return None;
    }

    let mut raw_groups: HashMap<Vec<AbsLitKey>, HashSet<Vec<i32>>> = HashMap::new();
    let mut used_pairs = BTreeSet::new();
    for clause in clauses {
        let (key, concrete) = parse_pair_clause(clause, num_vars)?;
        for lit in &key {
            used_pairs.insert(lit.pair);
        }
        raw_groups.entry(key).or_default().insert(concrete);
    }

    if raw_groups.is_empty() || used_pairs.is_empty() {
        return None;
    }
    if num_vars.checked_add(used_pairs.len())? > i32::MAX as usize {
        return None;
    }

    let mut pair_to_binding = vec![None; num_vars / 2];
    let mut dense_to_fresh = vec![0i32; used_pairs.len() + 1];
    let mut bindings = Vec::with_capacity(used_pairs.len());
    for (dense_idx, pair) in used_pairs.into_iter().enumerate() {
        let dense = dense_idx + 1;
        let fresh = (num_vars + dense) as i32;
        pair_to_binding[pair] = Some(PairBinding { pair, dense, fresh });
        dense_to_fresh[dense] = fresh;
        bindings.push(PairBinding { pair, dense, fresh });
    }

    let mut groups: Vec<PairAbsGroup> = Vec::with_capacity(raw_groups.len());
    let mut sorted_raw: Vec<(Vec<AbsLitKey>, HashSet<Vec<i32>>)> =
        raw_groups.into_iter().collect();
    sorted_raw.sort_by(|(lhs, _), (rhs, _)| lhs.cmp(rhs));

    for (key, concrete_set) in sorted_raw {
        let width = key.len();
        if width == 0 || width > MAX_ABSTRACT_CLAUSE_PAIRS {
            return None;
        }
        let expected = 1usize << width;
        if concrete_set.len() != expected {
            return None;
        }

        let mut abstract_clause = Vec::with_capacity(width);
        let mut lifted_clause = Vec::with_capacity(width);
        for lit in &key {
            let binding = pair_to_binding[lit.pair]?;
            let dense_lit = if lit.positive {
                binding.dense as i32
            } else {
                -(binding.dense as i32)
            };
            let fresh_lit = if lit.positive {
                binding.fresh
            } else {
                -binding.fresh
            };
            abstract_clause.push(dense_lit);
            lifted_clause.push(fresh_lit);
        }
        abstract_clause = canonical_clause(abstract_clause)?;
        lifted_clause = canonical_clause(lifted_clause)?;

        let mut original_clauses: Vec<Vec<i32>> = concrete_set.into_iter().collect();
        original_clauses.sort();
        groups.push(PairAbsGroup {
            lits: key,
            original_clauses,
            abstract_clause,
            lifted_clause,
        });
    }

    let abstract_clauses = groups
        .iter()
        .map(|group| group.abstract_clause.clone())
        .collect();
    Some(PairAbsFormula {
        abstract_num_vars: bindings.len(),
        abstract_clauses,
        dense_to_fresh,
        bindings,
        groups,
    })
}

fn parse_pair_clause(clause: &[i32], num_vars: usize) -> Option<(Vec<AbsLitKey>, Vec<i32>)> {
    if clause.is_empty()
        || clause.len() % 2 != 0
        || clause.len() / 2 > MAX_ABSTRACT_CLAUSE_PAIRS
    {
        return None;
    }

    let mut entries: Vec<(usize, [Option<bool>; 2])> = Vec::new();
    for &lit in clause {
        let var = lit.unsigned_abs() as usize;
        if var == 0 || var > num_vars {
            return None;
        }
        let pair = (var - 1) / 2;
        let side = (var - 1) % 2;
        let entry_idx = entries
            .iter()
            .position(|(entry_pair, _)| *entry_pair == pair)
            .unwrap_or_else(|| {
                entries.push((pair, [None, None]));
                entries.len() - 1
            });
        if entries[entry_idx].1[side].is_some() {
            return None;
        }
        entries[entry_idx].1[side] = Some(lit > 0);
    }

    if entries.len() * 2 != clause.len() {
        return None;
    }
    entries.sort_by_key(|(pair, _)| *pair);

    let mut key = Vec::with_capacity(entries.len());
    for (pair, signs) in entries {
        let x_pos = signs[0]?;
        let y_pos = signs[1]?;
        let falsified_x = !x_pos;
        let falsified_y = !y_pos;
        let parity_true = falsified_x ^ falsified_y;
        key.push(AbsLitKey {
            pair,
            positive: !parity_true,
        });
    }

    Some((key, canonical_clause(clause.to_vec())?))
}

fn lift_group(
    group: &PairAbsGroup,
    bindings: &[PairBinding],
    proof: &mut Vec<Vec<i32>>,
) -> Option<()> {
    let mut current = group.original_clauses.clone();
    for lit in &group.lits {
        let binding = bindings.iter().find(|binding| binding.pair == lit.pair)?;
        let x = pair_x(binding.pair)? as i32;
        let y = x + 1;
        let target_lit = if lit.positive {
            binding.fresh
        } else {
            -binding.fresh
        };

        let mut buckets: BTreeMap<Vec<i32>, u8> = BTreeMap::new();
        for clause in &current {
            let (rest, pattern) = split_pair_pattern(clause, x, y, lit.positive)?;
            let entry = buckets.entry(rest).or_insert(0);
            if (*entry & pattern) != 0 {
                return None;
            }
            *entry |= pattern;
        }

        let mut next = Vec::with_capacity(buckets.len());
        for (rest, mask) in buckets {
            if mask != 0b11 {
                return None;
            }
            let mut first = rest.clone();
            first.push(target_lit);
            first.push(if lit.positive { y } else { -y });
            proof.push(canonical_clause(first)?);

            let mut second = rest.clone();
            second.push(target_lit);
            second.push(if lit.positive { -y } else { y });
            proof.push(canonical_clause(second)?);

            let mut final_clause = rest;
            final_clause.push(target_lit);
            let final_clause = canonical_clause(final_clause)?;
            proof.push(final_clause.clone());
            next.push(final_clause);
        }
        current = next;
    }

    if current.len() != 1 || current[0] != group.lifted_clause {
        return None;
    }
    Some(())
}

fn split_pair_pattern(
    clause: &[i32],
    x: i32,
    y: i32,
    target_positive: bool,
) -> Option<(Vec<i32>, u8)> {
    let mut rest = Vec::with_capacity(clause.len().saturating_sub(2));
    let mut x_pos = None;
    let mut y_pos = None;
    for &lit in clause {
        if lit.unsigned_abs() == x as u32 {
            if x_pos.replace(lit > 0).is_some() {
                return None;
            }
        } else if lit.unsigned_abs() == y as u32 {
            if y_pos.replace(lit > 0).is_some() {
                return None;
            }
        } else {
            rest.push(lit);
        }
    }
    let x_pos = x_pos?;
    let y_pos = y_pos?;
    let pattern = if target_positive {
        match (x_pos, y_pos) {
            (true, true) => 0b01,
            (false, false) => 0b10,
            _ => return None,
        }
    } else {
        match (x_pos, y_pos) {
            (true, false) => 0b01,
            (false, true) => 0b10,
            _ => return None,
        }
    };
    Some((canonical_clause(rest)?, pattern))
}

fn pair_x(pair: usize) -> Option<usize> {
    pair.checked_mul(2)?.checked_add(1)
}

fn canonical_clause(mut clause: Vec<i32>) -> Option<Vec<i32>> {
    clause.sort_unstable_by(|&lhs, &rhs| {
        lhs.unsigned_abs()
            .cmp(&rhs.unsigned_abs())
            .then_with(|| lhs.cmp(&rhs))
    });
    let mut out = Vec::with_capacity(clause.len());
    let mut prev_var = 0u32;
    let mut prev_lit = 0i32;
    for lit in clause {
        let var = lit.unsigned_abs();
        if var == prev_var {
            if lit == prev_lit {
                continue;
            }
            return None;
        }
        prev_var = var;
        prev_lit = lit;
        out.push(lit);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_lifts_tiny_pair_abs_refutation() {
        let clauses = vec![
            vec![1, 2],
            vec![-1, -2],
            vec![3, 4],
            vec![-3, -4],
            vec![1, -2, 3, -4],
            vec![1, -2, -3, 4],
            vec![-1, 2, 3, -4],
            vec![-1, 2, -3, 4],
        ];
        let formula = extract_pair_abs_formula(&clauses, 4).expect("structured pair formula");
        assert_eq!(formula.abstract_num_vars, 2);
        assert_eq!(formula.abstract_clause_count(), 3);
        assert_eq!(formula.pair_count(), 2);
        assert_eq!(formula.abstract_clauses, vec![vec![-1, -2], vec![1], vec![2]]);

        let proof = formula.build_lift_proof().expect("lift proof");
        assert_eq!(&proof[..8], &[
            vec![1, 2, -5],
            vec![-1, -2, -5],
            vec![1, -2, 5],
            vec![-1, 2, 5],
            vec![3, 4, -6],
            vec![-3, -4, -6],
            vec![3, -4, 6],
            vec![-3, 4, 6],
        ]);
        assert!(proof.contains(&vec![5]));
        assert!(proof.contains(&vec![-5, -6]));
        assert!(proof.contains(&vec![6]));
    }
}
