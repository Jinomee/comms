//! Canonical environment-based AI tool detection and child-env clearing.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::tool::Tool;

#[derive(Debug, Clone, Copy)]
pub enum EnvMatch {
    Set,
    NonEmpty,
    Equals(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct EnvPredicate {
    pub var: &'static str,
    pub condition: EnvMatch,
}

#[derive(Debug)]
pub struct ToolDetectionRule {
    pub tool: Tool,
    pub predicates: &'static [EnvPredicate],
    pub clear_for_child: &'static [&'static str],
}

const CLAUDE_NATIVE: &[EnvPredicate] = &[
    EnvPredicate {
        var: "CLAUDECODE",
        condition: EnvMatch::Equals("1"),
    },
    EnvPredicate {
        var: "CLAUDE_ENV_FILE",
        condition: EnvMatch::NonEmpty,
    },
];
const ANTIGRAVITY_NATIVE: &[EnvPredicate] = &[EnvPredicate {
    var: "ANTIGRAVITY_AGENT",
    condition: EnvMatch::Set,
}];
const GEMINI_NATIVE: &[EnvPredicate] = &[EnvPredicate {
    var: "GEMINI_CLI",
    condition: EnvMatch::Equals("1"),
}];
const CODEX_NATIVE: &[EnvPredicate] = &[
    EnvPredicate {
        var: "CODEX_SANDBOX",
        condition: EnvMatch::Set,
    },
    EnvPredicate {
        var: "CODEX_SANDBOX_NETWORK_DISABLED",
        condition: EnvMatch::Set,
    },
    EnvPredicate {
        var: "CODEX_MANAGED_BY_NPM",
        condition: EnvMatch::Set,
    },
    EnvPredicate {
        var: "CODEX_MANAGED_BY_BUN",
        condition: EnvMatch::Set,
    },
    EnvPredicate {
        var: "CODEX_THREAD_ID",
        condition: EnvMatch::Set,
    },
    EnvPredicate {
        var: "CODEX_SESSION_ID",
        condition: EnvMatch::Set,
    },
];
const OPENCODE_NATIVE: &[EnvPredicate] = &[EnvPredicate {
    var: "OPENCODE",
    condition: EnvMatch::Equals("1"),
}];
const KILO_NATIVE: &[EnvPredicate] = &[EnvPredicate {
    var: "KILO",
    condition: EnvMatch::Equals("1"),
}];
const CURSOR_NATIVE: &[EnvPredicate] = &[
    EnvPredicate {
        var: "CURSOR_AGENT",
        condition: EnvMatch::Set,
    },
    EnvPredicate {
        var: "CURSOR_PROJECT_DIR",
        condition: EnvMatch::Set,
    },
];
const KIMI_NATIVE: &[EnvPredicate] = &[
    EnvPredicate {
        var: "KIMI_CODE_CLI",
        condition: EnvMatch::Equals("1"),
    },
    EnvPredicate {
        var: "KIMI_SESSION_ID",
        condition: EnvMatch::Set,
    },
];
const GROK_NATIVE: &[EnvPredicate] = &[
    EnvPredicate {
        var: "GROK_SESSION_ID",
        condition: EnvMatch::Set,
    },
    EnvPredicate {
        var: "GROK_HOOK_EVENT",
        condition: EnvMatch::Set,
    },
];
const PI_NATIVE: &[EnvPredicate] = &[EnvPredicate {
    var: "COMMS_PI",
    condition: EnvMatch::Equals("1"),
}];
const OMP_NATIVE: &[EnvPredicate] = &[EnvPredicate {
    var: "COMMS_OMP",
    condition: EnvMatch::Equals("1"),
}];

macro_rules! comms_tool_predicate {
    ($name:literal, $ident:ident) => {
        const $ident: &[EnvPredicate] = &[EnvPredicate {
            var: "COMMS_TOOL",
            condition: EnvMatch::Equals($name),
        }];
    };
}

comms_tool_predicate!("claude", COMMS_TOOL_CLAUDE);
comms_tool_predicate!("antigravity", COMMS_TOOL_ANTIGRAVITY);
comms_tool_predicate!("gemini", COMMS_TOOL_GEMINI);
comms_tool_predicate!("codex", COMMS_TOOL_CODEX);
comms_tool_predicate!("opencode", COMMS_TOOL_OPENCODE);
comms_tool_predicate!("kilo", COMMS_TOOL_KILO);
comms_tool_predicate!("cursor", COMMS_TOOL_CURSOR);
comms_tool_predicate!("kimi", COMMS_TOOL_KIMI);
comms_tool_predicate!("copilot", COMMS_TOOL_COPILOT);
comms_tool_predicate!("grok", COMMS_TOOL_GROK);
comms_tool_predicate!("pi", COMMS_TOOL_PI);
comms_tool_predicate!("omp", COMMS_TOOL_OMP);

/// Detection precedence: native markers first, then comms's explicit fallback.
pub static TOOL_DETECTION_RULES: &[ToolDetectionRule] = &[
    ToolDetectionRule {
        tool: Tool::Claude,
        predicates: CLAUDE_NATIVE,
        clear_for_child: &["CLAUDECODE", "CLAUDE_ENV_FILE"],
    },
    ToolDetectionRule {
        tool: Tool::Antigravity,
        predicates: ANTIGRAVITY_NATIVE,
        clear_for_child: &["ANTIGRAVITY_AGENT"],
    },
    ToolDetectionRule {
        tool: Tool::Gemini,
        predicates: GEMINI_NATIVE,
        clear_for_child: &["GEMINI_CLI", "GEMINI_SYSTEM_MD", "GEMINI_CLI_NO_RELAUNCH"],
    },
    ToolDetectionRule {
        tool: Tool::Codex,
        predicates: CODEX_NATIVE,
        clear_for_child: &[
            "CODEX_SANDBOX",
            "CODEX_SANDBOX_NETWORK_DISABLED",
            "CODEX_MANAGED_BY_NPM",
            "CODEX_MANAGED_BY_BUN",
            "CODEX_THREAD_ID",
            "CODEX_SESSION_ID",
        ],
    },
    ToolDetectionRule {
        tool: Tool::OpenCode,
        predicates: OPENCODE_NATIVE,
        clear_for_child: &["OPENCODE", "OPENCODE_PID"],
    },
    ToolDetectionRule {
        tool: Tool::Kilo,
        predicates: KILO_NATIVE,
        clear_for_child: &["KILO"],
    },
    ToolDetectionRule {
        tool: Tool::Cursor,
        predicates: CURSOR_NATIVE,
        clear_for_child: &["CURSOR_AGENT", "CURSOR_PROJECT_DIR"],
    },
    ToolDetectionRule {
        tool: Tool::Kimi,
        predicates: KIMI_NATIVE,
        clear_for_child: &["KIMI_CODE_CLI", "KIMI_SESSION_ID"],
    },
    ToolDetectionRule {
        tool: Tool::Grok,
        predicates: GROK_NATIVE,
        clear_for_child: &[
            "GROK_SESSION_ID",
            "GROK_HOOK_EVENT",
            "GROK_HOOK_NAME",
            "GROK_AGENT",
            "GROK_LEADER_SOCKET",
        ],
    },
    ToolDetectionRule {
        tool: Tool::Pi,
        predicates: PI_NATIVE,
        clear_for_child: &["COMMS_PI", "PI_CODING_AGENT", "PI_CODING_AGENT_SESSION_DIR"],
    },
    ToolDetectionRule {
        tool: Tool::Omp,
        predicates: OMP_NATIVE,
        clear_for_child: &[
            "COMMS_OMP",
            "PI_CODING_AGENT",
            "PI_CODING_AGENT_SESSION_DIR",
        ],
    },
    ToolDetectionRule {
        tool: Tool::Claude,
        predicates: COMMS_TOOL_CLAUDE,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Antigravity,
        predicates: COMMS_TOOL_ANTIGRAVITY,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Gemini,
        predicates: COMMS_TOOL_GEMINI,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Codex,
        predicates: COMMS_TOOL_CODEX,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::OpenCode,
        predicates: COMMS_TOOL_OPENCODE,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Kilo,
        predicates: COMMS_TOOL_KILO,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Cursor,
        predicates: COMMS_TOOL_CURSOR,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Kimi,
        predicates: COMMS_TOOL_KIMI,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Copilot,
        predicates: COMMS_TOOL_COPILOT,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Grok,
        predicates: COMMS_TOOL_GROK,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Pi,
        predicates: COMMS_TOOL_PI,
        clear_for_child: &["COMMS_TOOL"],
    },
    ToolDetectionRule {
        tool: Tool::Omp,
        predicates: COMMS_TOOL_OMP,
        clear_for_child: &["COMMS_TOOL"],
    },
];

fn predicate_matches(env: &HashMap<String, String>, predicate: &EnvPredicate) -> bool {
    match predicate.condition {
        EnvMatch::Set => env.contains_key(predicate.var),
        EnvMatch::NonEmpty => env
            .get(predicate.var)
            .is_some_and(|value| !value.is_empty()),
        EnvMatch::Equals(expected) => env
            .get(predicate.var)
            .is_some_and(|value| value == expected),
    }
}

pub fn detect_tool(env: &HashMap<String, String>) -> Tool {
    TOOL_DETECTION_RULES
        .iter()
        .find(|rule| {
            rule.predicates
                .iter()
                .any(|predicate| predicate_matches(env, predicate))
        })
        .map(|rule| rule.tool)
        .unwrap_or(Tool::Adhoc)
}

pub fn tool_marker_vars() -> &'static [&'static str] {
    static VARS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
        let mut seen = HashSet::new();
        let mut vars = Vec::new();
        for rule in TOOL_DETECTION_RULES {
            for var in rule
                .predicates
                .iter()
                .map(|predicate| predicate.var)
                .chain(rule.clear_for_child.iter().copied())
            {
                if seen.insert(var) {
                    vars.push(var);
                }
            }
        }
        vars
    });
    VARS.as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn native_markers_beat_comms_tool_fallback() {
        assert_eq!(
            detect_tool(&env(&[("GEMINI_CLI", "1"), ("COMMS_TOOL", "claude")])),
            Tool::Gemini
        );
    }

    #[test]
    fn antigravity_precedes_overlapping_gemini_marker() {
        assert_eq!(
            detect_tool(&env(&[("ANTIGRAVITY_AGENT", "1"), ("GEMINI_CLI", "1")])),
            Tool::Antigravity
        );
    }

    #[test]
    fn every_detection_var_is_cleared_for_children() {
        let clear: HashSet<&str> = tool_marker_vars().iter().copied().collect();
        for rule in TOOL_DETECTION_RULES {
            for predicate in rule.predicates {
                assert!(
                    clear.contains(predicate.var),
                    "{} detection marker must be cleared for child processes",
                    predicate.var
                );
            }
        }
    }

    #[test]
    fn previously_missing_markers_are_in_child_clear_set() {
        assert!(tool_marker_vars().contains(&"CLAUDE_ENV_FILE"));
        assert!(tool_marker_vars().contains(&"COMMS_TOOL"));
    }
}
