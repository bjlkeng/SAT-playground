use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum FormulaEdit {
    Add { clause: Vec<i32> },
    Delete { clause: Vec<i32> },
    Strengthen { before: Vec<i32>, after: Vec<i32> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FormulaEditEnvelope {
    schema_version: u32,
    config_hash: String,
    seed: u64,
    edits: Vec<FormulaEdit>,
}

fn replay_formula_edits(mut formula: Cnf, edits: &[FormulaEdit]) -> Result<Cnf, String> {
    for (edit_idx, edit) in edits.iter().enumerate() {
        match edit {
            FormulaEdit::Add { clause } => formula.push(clause.clone()),
            FormulaEdit::Delete { clause } => {
                let pos = formula
                    .iter()
                    .position(|candidate| candidate == clause)
                    .ok_or_else(|| {
                        format!("edit#{edit_idx}: delete target not found: {clause:?}")
                    })?;
                formula.remove(pos);
            }
            FormulaEdit::Strengthen { before, after } => {
                if !is_clause_strengthening(before, after) {
                    return Err(format!(
                        "edit#{edit_idx}: after clause {after:?} is not a strengthening of {before:?}"
                    ));
                }
                let pos = formula
                    .iter()
                    .position(|candidate| candidate == before)
                    .ok_or_else(|| {
                        format!("edit#{edit_idx}: strengthen target not found: {before:?}")
                    })?;
                formula[pos] = after.clone();
            }
        }
    }
    Ok(formula)
}

fn is_clause_strengthening(before: &[i32], after: &[i32]) -> bool {
    let before_set: BTreeSet<i32> = before.iter().copied().collect();
    let after_set: BTreeSet<i32> = after.iter().copied().collect();
    after_set.is_subset(&before_set) && after_set.len() < before_set.len()
}

fn serialize_envelope(envelope: &FormulaEditEnvelope) -> String {
    let mut out = format!(
        "formula_edit_log_v{}\nconfig_hash={}\nseed={}\n",
        envelope.schema_version, envelope.config_hash, envelope.seed
    );
    for edit in &envelope.edits {
        match edit {
            FormulaEdit::Add { clause } => {
                out.push_str("add ");
                push_clause(&mut out, clause);
            }
            FormulaEdit::Delete { clause } => {
                out.push_str("delete ");
                push_clause(&mut out, clause);
            }
            FormulaEdit::Strengthen { before, after } => {
                out.push_str("strengthen ");
                push_clause(&mut out, before);
                out.push_str(" => ");
                push_clause(&mut out, after);
            }
        }
        out.push('\n');
    }
    out
}

fn push_clause(out: &mut String, clause: &[i32]) {
    out.push('[');
    for (idx, lit) in clause.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&lit.to_string());
    }
    out.push(']');
}

#[test]
fn formula_edit_replay_self_tests_synthetic_add_delete_strengthen_events() {
    let initial = vec![vec![1, 2], vec![-1, 3]];
    let edits = vec![
        FormulaEdit::Add {
            clause: vec![-2, 3],
        },
        FormulaEdit::Strengthen {
            before: vec![1, 2],
            after: vec![1],
        },
        FormulaEdit::Delete {
            clause: vec![-1, 3],
        },
    ];

    let replayed = replay_formula_edits(initial, &edits).expect("synthetic replay");
    assert_eq!(replayed, vec![vec![1], vec![-2, 3]]);
}

#[test]
fn formula_edit_replay_rejects_invalid_synthetic_events() {
    let initial = vec![vec![1, 2]];
    let missing_delete =
        replay_formula_edits(initial.clone(), &[FormulaEdit::Delete { clause: vec![3] }]);
    assert!(missing_delete
        .expect_err("delete target should be rejected")
        .contains("delete target not found"));

    let invalid_strengthen = replay_formula_edits(
        initial,
        &[FormulaEdit::Strengthen {
            before: vec![1, 2],
            after: vec![1, 3],
        }],
    );
    assert!(invalid_strengthen
        .expect_err("invalid strengthening should be rejected")
        .contains("not a strengthening"));
}

#[test]
fn formula_edit_debug_log_envelope_records_replay_metadata() {
    let config = SolverConfig::default();
    let envelope = FormulaEditEnvelope {
        schema_version: 1,
        config_hash: config.config_hash(),
        seed: seed_from_env(0x51_10_06),
        edits: vec![
            FormulaEdit::Add {
                clause: vec![1, -2],
            },
            FormulaEdit::Delete { clause: vec![3] },
        ],
    };

    let serialized = serialize_envelope(&envelope);
    assert!(serialized.starts_with("formula_edit_log_v1\n"));
    assert!(serialized.contains(&format!("config_hash={}\n", config.config_hash())));
    assert!(serialized.contains("seed="));
    assert!(serialized.contains("add [1,-2]\n"));
    assert!(serialized.contains("delete [3]\n"));
}
