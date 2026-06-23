//! SAT Competition output helpers and the solver 12 run-result contract.
//!
//! This module owns stdout formatting details that are independent from the
//! search algorithm. Proof file creation is still in `main.rs` until the proof
//! boundary is extracted by a later task.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::SolverConfig;
use crate::{FALSE, UNASSIGNED};

pub(crate) const RESULT_JSON: &str = "result.json";
pub(crate) const STATUS_TXT: &str = "status.txt";
pub(crate) const MODEL_TXT: &str = "model.txt";
pub(crate) const PROOF_OUT: &str = "proof.out";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SolveStatus {
    Sat,
    Unsat,
    Unknown,
    ParseError,
}

impl SolveStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Sat => "SAT",
            Self::Unsat => "UNSAT",
            Self::Unknown => "UNKNOWN",
            Self::ParseError => "PARSE_ERROR",
        }
    }

    pub(crate) fn s_line(self) -> &'static str {
        match self {
            Self::Sat => "s SATISFIABLE",
            Self::Unsat => "s UNSATISFIABLE",
            Self::Unknown | Self::ParseError => "s UNKNOWN",
        }
    }

    pub(crate) fn exit_code(self) -> i32 {
        match self {
            Self::Sat | Self::Unsat | Self::Unknown => 0,
            Self::ParseError => 2,
        }
    }

    pub(crate) fn termination_reason(self) -> &'static str {
        match self {
            Self::Sat => "sat",
            Self::Unsat => "unsat",
            Self::Unknown => "unknown",
            Self::ParseError => "parse-error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProofCompleteness {
    Complete,
    NotRequested,
    Incomplete,
    None,
}

impl ProofCompleteness {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::NotRequested => "not_requested",
            Self::Incomplete => "incomplete",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputContractState {
    Complete,
    #[allow(dead_code)]
    Rejected,
}

impl OutputContractState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputContract {
    pub(crate) status: SolveStatus,
    pub(crate) proof_completeness: ProofCompleteness,
    pub(crate) model_written: bool,
    pub(crate) proof_written: bool,
    pub(crate) stats_written: bool,
    pub(crate) result_json_written: bool,
    pub(crate) output_contract_state: OutputContractState,
}

impl OutputContract {
    pub(crate) fn validate(self) -> Result<(), &'static str> {
        let _stats_written = self.stats_written;
        if !self.result_json_written {
            return Err("result_json_missing");
        }
        if self.output_contract_state != OutputContractState::Complete {
            return Err("output_contract_incomplete");
        }
        match self.status {
            SolveStatus::Sat => {
                if !self.model_written {
                    return Err("sat_model_missing");
                }
                if self.proof_written {
                    return Err("sat_proof_forbidden");
                }
                if matches!(self.proof_completeness, ProofCompleteness::Complete) {
                    return Err("sat_proof_complete_forbidden");
                }
            }
            SolveStatus::Unsat => {
                if self.model_written {
                    return Err("unsat_model_forbidden");
                }
                match self.proof_completeness {
                    ProofCompleteness::Complete => {
                        if !self.proof_written {
                            return Err("unsat_complete_proof_missing");
                        }
                    }
                    ProofCompleteness::NotRequested => {
                        if self.proof_written {
                            return Err("unsat_unrequested_proof_present");
                        }
                    }
                    _ => return Err("unsat_proof_incomplete"),
                }
            }
            SolveStatus::Unknown => {
                if self.model_written {
                    return Err("unknown_model_forbidden");
                }
                if self.proof_written {
                    return Err("unknown_proof_forbidden");
                }
                if self.proof_completeness == ProofCompleteness::Complete {
                    return Err("unknown_complete_proof_forbidden");
                }
            }
            SolveStatus::ParseError => {
                if self.model_written {
                    return Err("parse_error_model_forbidden");
                }
                if self.proof_written {
                    return Err("parse_error_proof_forbidden");
                }
            }
        }
        Ok(())
    }
}

pub(crate) struct ResultContractFields<'a> {
    pub(crate) unknown_reason: Option<&'a str>,
    pub(crate) termination_reason: Option<&'a str>,
    pub(crate) input_sha256: Option<&'a str>,
    pub(crate) model_check_result: &'a str,
    pub(crate) proof_check_result: &'a str,
    pub(crate) output_contract_state: &'a str,
}

impl<'a> ResultContractFields<'a> {
    pub(crate) fn new(
        unknown_reason: Option<&'a str>,
        input_sha256: Option<&'a str>,
        model_check_result: &'a str,
        proof_check_result: &'a str,
        output_contract_state: &'a str,
    ) -> Self {
        Self {
            unknown_reason,
            termination_reason: None,
            input_sha256,
            model_check_result,
            proof_check_result,
            output_contract_state,
        }
    }

    pub(crate) fn with_termination_reason(mut self, termination_reason: &'a str) -> Self {
        self.termination_reason = Some(termination_reason);
        self
    }
}

pub(crate) fn print_assignment(assignment: &[u8]) {
    for line in assignment_lines(assignment) {
        println!("{line}");
    }
}

pub(crate) fn assignment_lines(assignment: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::from("v");
    for (var, &value) in assignment.iter().enumerate().skip(1) {
        assert_ne!(
            value, UNASSIGNED,
            "SAT model snapshot left variable {var} unassigned"
        );
        let lit = if value == FALSE {
            -(var as i32)
        } else {
            var as i32
        };
        let token = format!(" {}", lit);
        if line.len() + token.len() > 4090 {
            lines.push(line);
            line = String::from("v");
        }
        line.push_str(&token);
    }
    line.push_str(" 0");
    lines.push(line);
    lines
}

pub(crate) fn prepare_output_contract_dir(output_dir: &Path) {
    fs::create_dir_all(output_dir).unwrap_or_else(|err| {
        eprintln!(
            "Error creating output directory {}: {err}",
            output_dir.display()
        );
        std::process::exit(2);
    });
    for file_name in [
        RESULT_JSON,
        STATUS_TXT,
        MODEL_TXT,
        PROOF_OUT,
        "proof.out.tmp",
    ] {
        let path = output_dir.join(file_name);
        if path.exists() {
            fs::remove_file(&path).unwrap_or_else(|err| {
                eprintln!(
                    "Error removing stale output artifact {}: {err}",
                    path.display()
                );
                std::process::exit(2);
            });
        }
    }
}

pub(crate) fn write_model_file(output_dir: &Path, assignment: &[u8]) -> PathBuf {
    let path = output_dir.join(MODEL_TXT);
    fs::write(&path, assignment_lines(assignment).join("\n") + "\n").unwrap_or_else(|err| {
        eprintln!("Error writing model file {}: {err}", path.display());
        std::process::exit(2);
    });
    path
}

pub(crate) fn write_result_contract(
    output_dir: &Path,
    status: SolveStatus,
    config: &SolverConfig,
    fields: &ResultContractFields<'_>,
    model_file: Option<&Path>,
    proof_completeness: ProofCompleteness,
) {
    fs::create_dir_all(output_dir).unwrap_or_else(|err| {
        eprintln!(
            "Error creating output directory {}: {err}",
            output_dir.display()
        );
        std::process::exit(2);
    });
    let status_file = output_dir.join(STATUS_TXT);
    fs::write(&status_file, format!("{}\n", status.as_str())).unwrap_or_else(|err| {
        eprintln!("Error writing status file {}: {err}", status_file.display());
        std::process::exit(2);
    });

    let proof_file = output_dir.join(PROOF_OUT);
    let proof_file_json = if proof_file.exists() {
        json_string(&proof_file.display().to_string())
    } else {
        "null".to_string()
    };
    let model_file_json = model_file
        .map(|path| json_string(&path.display().to_string()))
        .unwrap_or_else(|| "null".to_string());

    let result = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"status\": \"{}\",\n",
            "  \"exit_code\": {},\n",
            "  \"termination_reason\": \"{}\",\n",
            "  \"unknown_reason\": {},\n",
            "  \"status_file\": {},\n",
            "  \"model_file\": {},\n",
            "  \"proof_file\": {},\n",
            "  \"proof_completeness\": \"{}\",\n",
            "  \"model_check_result\": \"{}\",\n",
            "  \"proof_check_result\": \"{}\",\n",
            "  \"config_hash\": \"{}\",\n",
            "  \"input_sha256\": {},\n",
            "  \"profile\": \"{}\",\n",
            "  \"proof_policy\": \"{}\",\n",
            "  \"output_contract_state\": \"{}\",\n",
            "  \"stats_json_seen\": {}\n",
            "}}\n"
        ),
        status.as_str(),
        status.exit_code(),
        fields
            .termination_reason
            .unwrap_or_else(|| status.termination_reason()),
        fields
            .unknown_reason
            .map(json_string)
            .unwrap_or_else(|| "null".to_string()),
        json_string(&status_file.display().to_string()),
        model_file_json,
        proof_file_json,
        proof_completeness.as_str(),
        fields.model_check_result,
        fields.proof_check_result,
        config.config_hash(),
        fields
            .input_sha256
            .map(json_string)
            .unwrap_or_else(|| "null".to_string()),
        config.profile_name(),
        config.proof_policy_name(),
        fields.output_contract_state,
        if config.stats_json { "true" } else { "false" },
    );
    let result_path = output_dir.join(RESULT_JSON);
    fs::write(&result_path, result).unwrap_or_else(|err| {
        eprintln!("Error writing result file {}: {err}", result_path.display());
        std::process::exit(2);
    });
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}
