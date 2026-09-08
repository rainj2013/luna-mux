//! Explicit CLI hints over the rendered cursor row, not process detection.
use crate::terminal_runtime_contract::TerminalScreenSnapshot;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptState {
    Prompt,
    Continuation,
    Pager,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CliPreset {
    Mysql,
    Redis,
    Pager,
    #[default]
    Custom,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptRule {
    pub state: PromptState,
    pub pattern: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CliProfile {
    #[serde(default)]
    pub profile: CliPreset,
    /// When present, replaces the preset's rules in the supplied order.
    pub rules: Option<Vec<PromptRule>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScreenSnapshotArguments {
    #[serde(default = "snapshot_bytes")]
    pub max_bytes: usize,
    pub cli: Option<CliProfile>,
}
fn snapshot_bytes() -> usize {
    65536
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptWait {
    pub cli: CliProfile,
    #[serde(default = "default_states")]
    pub states: Vec<PromptState>,
}
fn default_states() -> Vec<PromptState> {
    vec![PromptState::Prompt]
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliObservation {
    pub state: PromptState,
    pub profile: CliPreset,
    pub source: &'static str,
    pub matched_rule: Option<usize>,
}

pub struct PromptMatcher {
    profile: CliPreset,
    source: &'static str,
    rules: Vec<(PromptState, Regex)>,
}
impl PromptMatcher {
    pub fn new(config: &CliProfile) -> Result<Self, String> {
        let preset: &[(_, &str)] = match config.profile {
            CliPreset::Mysql => &[
                (PromptState::Continuation, r#"\s*(?:->|'>|">|`>|/\*>)\s*"#),
                (
                    PromptState::Pager,
                    r"\s*(?:--More--(?:\(\d+%\))?|\(END\))\s*",
                ),
                (PromptState::Prompt, r"\s*mysql>\s*"),
            ],
            CliPreset::Redis => &[(
                PromptState::Prompt,
                r"(?:[^\s>]+:\d+(?:\[\d+\])?|not connected)>\s*",
            )],
            CliPreset::Pager => &[(
                PromptState::Pager,
                r"\s*(?:--More--(?:\(\d+%\))?|\(END\)|:)\s*",
            )],
            CliPreset::Custom => &[],
        };
        let rules = config.rules.clone().unwrap_or_else(|| {
            preset
                .iter()
                .map(|(state, pattern)| PromptRule {
                    state: state.clone(),
                    pattern: pattern.to_string(),
                })
                .collect()
        });
        if rules.is_empty() || rules.len() > 16 {
            return Err("CLI rules must contain 1..16 entries".into());
        }
        let mut compiled = Vec::new();
        for rule in rules {
            if rule.state == PromptState::Unknown
                || rule.pattern.is_empty()
                || rule.pattern.len() > 512
            {
                return Err(
                    "CLI rule requires prompt/continuation/pager and a 1..512-byte pattern".into(),
                );
            }
            let regex = RegexBuilder::new(&format!(r"\A(?:{})\z", rule.pattern))
                .size_limit(1024 * 1024)
                .dfa_size_limit(1024 * 1024)
                .build()
                .map_err(|_| "Invalid or overly complex CLI rule pattern".to_string())?;
            if regex.is_match("") {
                return Err("CLI rule must not match an empty line".into());
            }
            compiled.push((rule.state, regex));
        }
        Ok(Self {
            profile: config.profile.clone(),
            source: if config.rules.is_some() {
                "customRules"
            } else {
                "preset"
            },
            rules: compiled,
        })
    }

    pub fn observe(&self, screen: &TerminalScreenSnapshot) -> CliObservation {
        let matched = if screen.truncated || screen.size_limited {
            None
        } else {
            self.rules.iter().enumerate().find(|(_, (state, regex))| {
                (screen.cursor_visible || *state == PromptState::Pager)
                    && regex.is_match(&screen.cursor_line)
            })
        };
        CliObservation {
            state: matched
                .map(|(_, (state, _))| state.clone())
                .unwrap_or_default(),
            profile: self.profile.clone(),
            source: self.source,
            matched_rule: matched.map(|(index, _)| index),
        }
    }
}

impl PromptWait {
    pub fn matcher(&self) -> Result<PromptMatcher, String> {
        if self.states.is_empty()
            || self.states.len() > 3
            || self.states.contains(&PromptState::Unknown)
        {
            return Err(
                "wait.prompt.states must contain 1..3 prompt/continuation/pager states".into(),
            );
        }
        PromptMatcher::new(&self.cli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_screen::TerminalScreen;
    fn observe(profile: CliPreset, text: &str) -> PromptState {
        let mut screen = TerminalScreen::new(4, 80);
        screen.process(text.as_bytes());
        PromptMatcher::new(&CliProfile {
            profile,
            rules: None,
        })
        .unwrap()
        .observe(&screen.snapshot("r", 10, 10000))
        .state
    }
    #[test]
    fn presets_classify_rendered_prompts_without_inferring_process_identity() {
        assert_eq!(
            observe(CliPreset::Mysql, "\x1b[32mmysql> \x1b[0m"),
            PromptState::Prompt
        );
        for text in ["    -> ", "    '> ", "    `> ", "    /*> "] {
            assert_eq!(observe(CliPreset::Mysql, text), PromptState::Continuation);
        }
        assert_eq!(
            observe(CliPreset::Mysql, "mysql> SELECT 1;"),
            PromptState::Unknown
        );
        assert_eq!(
            observe(CliPreset::Mysql, "mysql> \r\x1b[Kbusy"),
            PromptState::Unknown
        );
        assert_eq!(
            observe(CliPreset::Redis, "127.0.0.1:6379[2]> "),
            PromptState::Prompt
        );
        assert_eq!(
            observe(CliPreset::Pager, "\x1b[7m(END)\x1b[0m"),
            PromptState::Pager
        );
    }
    #[test]
    fn custom_rules_are_bounded_replace_presets_and_reject_empty_matches() {
        let config: CliProfile = serde_json::from_value(serde_json::json!({"profile":"mysql","rules":[{"state":"continuation","pattern":"my-prompt>\\s*"}]})).unwrap();
        let mut screen = TerminalScreen::new(4, 80);
        screen.process(b"my-prompt> ");
        let matcher = PromptMatcher::new(&config).unwrap();
        assert_eq!(
            matcher.observe(&screen.snapshot("r", 1, 1000)).state,
            PromptState::Continuation
        );
        let mut incomplete = screen.snapshot("r", 1, 1000);
        incomplete.truncated = true;
        assert_eq!(matcher.observe(&incomplete).state, PromptState::Unknown);
        screen.process(b"\r\x1b[Kmysql> ");
        assert_eq!(
            matcher.observe(&screen.snapshot("r", 2, 1000)).state,
            PromptState::Unknown
        );
        let bad: CliProfile = serde_json::from_value(
            serde_json::json!({"rules":[{"state":"prompt","pattern":".*"}]}),
        )
        .unwrap();
        assert!(PromptMatcher::new(&bad).is_err());
    }
}
