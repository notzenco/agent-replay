//! Removes secrets from text before it leaves the machine.
//!
//! Redaction is applied when a session is exported, not when it is read, so
//! the terminal viewer still shows what really happened.
//!
//! Every rule reports the same shape: a span of the input to replace and the
//! label to replace it with. Rules are tried in priority order and a rule
//! never touches a span another rule already claimed.

use regex::Regex;

use crate::honeytoken;

/// The shortest run of characters that can be a secret.
const MIN_ENTROPY_LENGTH: usize = 32;

/// The Shannon entropy a run of characters needs before it is treated as a
/// secret, in bits per character.
const MIN_ENTROPY: f64 = 3.5;

/// One piece of the redacted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// Whether this piece is a honeytoken and should be highlighted.
    pub honeytoken: bool,
}

/// The result of redacting some text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Redaction {
    /// The text as it should be shown.
    pub text: String,
    /// The text split into pieces, so a renderer can highlight the tokens.
    pub segments: Vec<Segment>,
    /// The identifiers of the honeytokens that were found.
    pub honeytokens: Vec<String>,
    /// How many secrets were replaced.
    pub secrets: usize,
}

/// One rule that finds a kind of secret.
struct Rule {
    /// The text that replaces the secret.
    replacement: &'static str,
    /// Lower runs first. A rule that ran earlier keeps the span it claimed.
    priority: u8,
    /// The pattern that finds the secret.
    pattern: Regex,
    /// Which capture group holds the secret. Group zero is the whole match.
    group: usize,
}

/// Removes secrets from text.
pub struct Redactor {
    /// Whether secrets, home paths and emails are replaced.
    enabled: bool,
    /// Whether honeytokens are found and marked.
    highlight_honeytokens: bool,
    /// The home directory, replaced with `~`.
    home: Option<String>,
    rules: Vec<Rule>,
}

impl Redactor {
    /// A redactor that replaces secrets.
    pub fn new(enabled: bool) -> Self {
        Redactor {
            enabled,
            highlight_honeytokens: true,
            home: home_directory(),
            rules: rules(),
        }
    }

    /// Whether honeytokens are found and marked.
    pub fn highlight_honeytokens(mut self, highlight: bool) -> Self {
        self.highlight_honeytokens = highlight;
        self
    }

    /// Redact some text.
    pub fn redact(&self, text: &str) -> Redaction {
        let tokens = if self.highlight_honeytokens {
            honeytoken::find(text)
        } else {
            Vec::new()
        };

        let mut edits: Vec<Edit> = Vec::new();
        for token in &tokens {
            edits.push(Edit {
                start: token.start,
                end: token.end,
                replacement: text[token.start..token.end].to_string(),
                honeytoken: true,
                priority: 0,
            });
        }

        if self.enabled {
            for rule in &self.rules {
                for capture in rule.pattern.captures_iter(text) {
                    let Some(found) = capture.get(rule.group) else {
                        continue;
                    };
                    edits.push(Edit {
                        start: found.start(),
                        end: found.end(),
                        replacement: rule.replacement.to_string(),
                        honeytoken: false,
                        priority: rule.priority,
                    });
                }
            }
            if let Some(home) = &self.home {
                for (start, _) in text.match_indices(home.as_str()) {
                    edits.push(Edit {
                        start,
                        end: start + home.len(),
                        replacement: "~".to_string(),
                        honeytoken: false,
                        priority: 7,
                    });
                }
            }
            for (start, end) in high_entropy_spans(text) {
                edits.push(Edit {
                    start,
                    end,
                    replacement: "[redacted:high-entropy]".to_string(),
                    honeytoken: false,
                    priority: 8,
                });
            }
        }

        let accepted = resolve(text, edits);
        build(text, accepted, tokens)
    }
}

/// A change to make to the text.
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
    honeytoken: bool,
    priority: u8,
}

/// Choose the edits that do not fight each other.
///
/// Rules are tried in priority order. The first rule to claim a span keeps
/// it, so a password inside a JSON string is reported as a password rather
/// than as a run of random characters.
fn resolve(text: &str, mut edits: Vec<Edit>) -> Vec<Edit> {
    edits.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.start.cmp(&b.start)));

    let mut claimed = vec![false; text.len()];
    let mut accepted = Vec::new();
    for edit in edits {
        if claimed[edit.start..edit.end].iter().any(|c| *c) {
            continue;
        }
        for slot in &mut claimed[edit.start..edit.end] {
            *slot = true;
        }
        accepted.push(edit);
    }

    accepted.sort_by_key(|e| e.start);
    accepted
}

/// Apply the edits and split the result into segments.
fn build(text: &str, edits: Vec<Edit>, tokens: Vec<honeytoken::Honeytoken>) -> Redaction {
    let mut out = String::new();
    let mut segments: Vec<Segment> = Vec::new();
    let mut secrets = 0usize;
    let mut pos = 0usize;

    for edit in edits {
        if edit.start > pos {
            push_segment(&mut segments, &mut out, &text[pos..edit.start], false);
        }
        push_segment(&mut segments, &mut out, &edit.replacement, edit.honeytoken);
        if !edit.honeytoken {
            secrets += 1;
        }
        pos = edit.end;
    }
    if pos < text.len() {
        push_segment(&mut segments, &mut out, &text[pos..], false);
    }

    Redaction {
        text: out,
        segments,
        honeytokens: tokens.into_iter().map(|t| t.id).collect(),
        secrets,
    }
}

/// Add a piece of text, merging it with the piece before it when they are
/// the same kind.
fn push_segment(segments: &mut Vec<Segment>, out: &mut String, text: &str, honeytoken: bool) {
    if text.is_empty() {
        return;
    }
    out.push_str(text);
    match segments.last_mut() {
        Some(last) if last.honeytoken == honeytoken => last.text.push_str(text),
        _ => segments.push(Segment {
            text: text.to_string(),
            honeytoken,
        }),
    }
}

/// The rules that find secrets, in priority order.
fn rules() -> Vec<Rule> {
    // replacement, priority, pattern, capture group
    let patterns: Vec<(&'static str, u8, &str, usize)> = vec![
        (
            "[redacted:private-key]",
            1,
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
            0,
        ),
        ("[redacted:aws-key-id]", 2, r"\bAKIA[0-9A-Z]{16}\b", 0),
        (
            "[redacted:aws-secret-key]",
            2,
            r#"(?i)aws_secret_access_key\s*[=:]\s*["']?([A-Za-z0-9/+=]{40})["']?"#,
            1,
        ),
        (
            "[redacted:github-token]",
            2,
            r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b",
            0,
        ),
        (
            "[redacted:gitlab-token]",
            2,
            r"\bglpat-[A-Za-z0-9_-]{20,}\b",
            0,
        ),
        (
            "[redacted:slack-token]",
            2,
            r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b",
            0,
        ),
        (
            "[redacted:stripe-key]",
            2,
            r"\b[sr]k_live_[A-Za-z0-9]{16,}\b",
            0,
        ),
        (
            "[redacted:anthropic-key]",
            2,
            r"\bsk-ant-[A-Za-z0-9_-]{20,}\b",
            0,
        ),
        (
            "[redacted:openai-key]",
            2,
            r"\bsk-(?:proj-)?[A-Za-z0-9]{20,}\b",
            0,
        ),
        (
            "[redacted:google-api-key]",
            2,
            r"\bAIza[0-9A-Za-z_-]{35}\b",
            0,
        ),
        ("[redacted:npm-token]", 2, r"\bnpm_[A-Za-z0-9]{36}\b", 0),
        (
            "[redacted:jwt]",
            2,
            r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
            0,
        ),
        (
            "[redacted:bearer-token]",
            3,
            r"(?i)\bBearer\s+([A-Za-z0-9._~+/=-]{16,})",
            1,
        ),
        (
            "[redacted:url-password]",
            3,
            // The password in `scheme://user:password@host`. A placeholder
            // such as `${DB_PASSWORD}` is not a secret, so the password has
            // to start with a letter or a digit.
            r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s/@:"'<]+:([A-Za-z0-9][^\s/@:"'<]{3,})@"#,
            1,
        ),
        (
            "[redacted:password]",
            4,
            r#"(?i)\b(?:pass(?:word|wd)?|secret|api[_-]?key|access[_-]?token|auth[_-]?token)\b["']?\s*[=:]\s*["']?([^\s"',;]{6,})"#,
            1,
        ),
        (
            "[email]",
            5,
            r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b",
            0,
        ),
        (
            "~",
            6,
            // Any user's home directory, not just this machine's: a session
            // recorded elsewhere (or a CI log it quotes) names other users.
            // The boundary character keeps a web route such as
            // `example.com/home/index` from looking like a home directory.
            r#"(?:^|[\s"'`(=:,<\[/])((?:/Users/|/home/|/mnt/[A-Za-z]/Users/)[A-Za-z0-9._-]+)"#,
            1,
        ),
        (
            "~",
            6,
            r#"(?:^|[\s"'`(=:,<\[/])([A-Za-z]:(?:\\{1,2}|/)Users(?:\\{1,2}|/)[A-Za-z0-9._-]+)"#,
            1,
        ),
    ];

    patterns
        .into_iter()
        .map(|(replacement, priority, pattern, group)| Rule {
            replacement,
            priority,
            pattern: Regex::new(pattern).expect("redaction pattern should compile"),
            group,
        })
        .collect()
}

/// The home directory, as the current user sees it.
fn home_directory() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| home.to_string_lossy().to_string())
        .filter(|home| !home.is_empty())
}

/// Find the runs of characters that look like a secret nobody has a rule for.
fn high_entropy_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if is_candidate_byte(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_candidate_byte(bytes[i]) {
                i += 1;
            }
            let candidate = &text[start..i];
            if is_high_entropy(candidate) {
                spans.push((start, i));
            }
        } else {
            i += 1;
        }
    }
    spans
}

/// Whether a byte could be part of a secret.
fn is_candidate_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
}

/// Whether a run of characters looks like a secret.
///
/// The test is deliberately strict: a run has to be long, has to mix upper
/// case, lower case and digits, and has to be close to random. A git commit
/// hash is lower case hex and never qualifies, and neither does an ordinary
/// long word.
fn is_high_entropy(candidate: &str) -> bool {
    if candidate.len() < MIN_ENTROPY_LENGTH {
        return false;
    }
    let has_upper = candidate.bytes().any(|b| b.is_ascii_uppercase());
    let has_lower = candidate.bytes().any(|b| b.is_ascii_lowercase());
    let has_digit = candidate.bytes().any(|b| b.is_ascii_digit());
    if !(has_upper && has_lower && has_digit) {
        return false;
    }
    shannon_entropy(candidate) >= MIN_ENTROPY
}

/// The Shannon entropy of a run of characters, in bits per character.
fn shannon_entropy(text: &str) -> f64 {
    let mut counts = [0usize; 256];
    for byte in text.bytes() {
        counts[usize::from(byte)] += 1;
    }
    let total = text.len() as f64;
    let mut entropy = 0.0;
    for count in counts.iter().filter(|c| **c > 0) {
        let p = *count as f64 / total;
        entropy -= p * p.log2();
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(text: &str) -> Redaction {
        Redactor::new(true).redact(text)
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let text = "just a normal sentence about the weather";
        assert_eq!(redact(text).text, text);
        assert_eq!(redact(text).secrets, 0);
    }

    #[test]
    fn removes_an_aws_key_id() {
        let out = redact("key is AKIAIOSFODNN7EXAMPLE here").text;
        assert!(out.contains("[redacted:aws-key-id]"));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn removes_a_github_token() {
        let out = redact("ghp_abcdefghijklmnopqrstuvwxyz0123456789").text;
        assert!(out.contains("[redacted:github-token]"));
    }

    #[test]
    fn removes_a_private_key_block() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----";
        let out = redact(text).text;
        assert_eq!(out, "[redacted:private-key]");
    }

    #[test]
    fn removes_a_password_but_keeps_the_key() {
        let out = redact("password = hunter2000").text;
        assert_eq!(out, "password = [redacted:password]");
    }

    #[test]
    fn removes_an_email_address() {
        let out = redact("write to someone@example.com please").text;
        assert!(out.contains("[email]"));
        assert!(!out.contains("someone@example.com"));
    }

    #[test]
    fn removes_the_home_directory() {
        let redactor = Redactor::new(true);
        let Some(home) = redactor.home.clone() else {
            return;
        };
        let text = format!("{home}/Developer/project");
        let out = redactor.redact(&text).text;
        assert_eq!(out, "~/Developer/project");
    }

    #[test]
    fn removes_a_high_entropy_string() {
        let secret = "aB3xY9zQ1mN7pR4tK6wJ2hF5dG8sV0cE";
        let out = redact(&format!("the value is {secret} ok")).text;
        assert!(out.contains("[redacted:high-entropy]"));
        assert!(!out.contains(secret));
    }

    #[test]
    fn keeps_a_git_commit_hash() {
        let hash = "a3f5c9d1e7b4a2f8c6d0e3b5a7c9f1d2e4b6a8c0";
        let out = redact(&format!("commit {hash}")).text;
        assert!(out.contains(hash));
    }

    #[test]
    fn keeps_a_long_identifier() {
        let name = "ThisIsAVeryLongCamelCaseIdentifierNameWithoutDigits";
        let out = redact(name).text;
        assert_eq!(out, name);
    }

    #[test]
    fn can_be_turned_off() {
        let redactor = Redactor::new(false);
        let text = "key AKIAIOSFODNN7EXAMPLE mail someone@example.com";
        let out = redactor.redact(text);
        assert_eq!(out.text, text);
        assert_eq!(out.secrets, 0);
    }

    #[test]
    fn finds_and_marks_a_honeytoken() {
        let id = honeytoken::id_with_check_chars("ABCDEF");
        let token = format!("{}{id}", honeytoken::MARKER);
        let out = redact(&format!("leaked {token} here"));
        assert_eq!(out.honeytokens.len(), 1);
        assert!(
            out.segments
                .iter()
                .any(|s| s.honeytoken && s.text.contains(&token))
        );
    }

    #[test]
    fn honeytoken_highlighting_can_be_turned_off() {
        let redactor = Redactor::new(true).highlight_honeytokens(false);
        let out = redactor.redact("TRIPWIREABCDEF");
        assert!(out.honeytokens.is_empty());
        assert!(!out.segments.iter().any(|s| s.honeytoken));
    }

    #[test]
    fn a_honeytoken_is_not_removed_as_high_entropy() {
        let id = honeytoken::id_with_check_chars("ABCD");
        let token = format!("{}{id}", honeytoken::MARKER);
        let out = redact(&format!("{token}Zq7Xk2Lm9pRt4"));
        assert!(out.text.contains(&token));
    }

    #[test]
    fn segments_join_back_to_the_text() {
        let out = redact("hello AKIAIOSFODNN7EXAMPLE world");
        let joined: String = out.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, out.text);
    }

    #[test]
    fn a_bearer_header_loses_only_the_token() {
        let out = redact("Authorization: Bearer abcdef0123456789abcdef").text;
        assert!(out.contains("Bearer [redacted:bearer-token]"));
    }

    #[test]
    fn removes_another_users_home_directory() {
        let out = redact("built in /Users/runner/work/app and /home/deploy/app").text;
        assert_eq!(out, "built in ~/work/app and ~/app");
    }

    #[test]
    fn removes_a_windows_home_directory() {
        let out = redact(r"opened C:\Users\someone\dev\app and D:/Users/someone/app").text;
        assert_eq!(out, r"opened ~\dev\app and ~/app");
        let wsl = redact("synced from /mnt/c/Users/someone/dev/app").text;
        assert_eq!(wsl, "synced from ~/dev/app");
        // A path quoted inside JSON arrives with its backslashes doubled.
        let escaped = redact(r#"path "C:\\Users\\someone\\dev\\app" recorded"#).text;
        assert_eq!(escaped, r#"path "~\\dev\\app" recorded"#);
    }

    #[test]
    fn keeps_a_home_shaped_web_route() {
        let text = "see https://example.com/home/index for the docs";
        assert_eq!(redact(text).text, text);
    }

    #[test]
    fn removes_the_password_from_a_url() {
        let out = redact("postgres://app:knm-locally-42@127.0.0.1:5432/app").text;
        assert_eq!(
            out,
            "postgres://app:[redacted:url-password]@127.0.0.1:5432/app"
        );
    }

    #[test]
    fn keeps_a_placeholder_password_in_a_url() {
        let text = "postgres://app:${DB_PASSWORD}@127.0.0.1/app";
        assert_eq!(redact(text).text, text);
    }

    #[test]
    fn keeps_an_ordinary_url() {
        let text = "clone https://github.com/notzenco/agent-replay at 12:30";
        assert_eq!(redact(text).text, text);
    }
}
