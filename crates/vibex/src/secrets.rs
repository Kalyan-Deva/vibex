//! Secret guard: spots API keys, tokens and private keys so Vibex can mask
//! them on screen, warn before pasting one, and keep them out of history,
//! recordings, GIFs and the audit log.
//!
//! Only formats with a recognisable shape are matched (a provider prefix,
//! a JWT's three parts, a PEM header, or `password=` style assignments),
//! so ordinary output stays readable.

use regex::Regex;

/// Built-in kinds: (name, pattern).
pub const DEFAULT: &[(&str, &str)] = &[
    ("AWS access key", r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b"),
    ("GitHub token", r"\bgh[pousr]_[A-Za-z0-9]{36,255}\b"),
    ("GitHub token", r"\bgithub_pat_[A-Za-z0-9_]{50,255}\b"),
    ("GitLab token", r"\bglpat-[A-Za-z0-9_-]{20,}\b"),
    ("Slack token", r"\bxox[abposr]-[A-Za-z0-9-]{10,}\b"),
    ("Stripe key", r"\b[sr]k_(?:live|test)_[A-Za-z0-9]{16,}\b"),
    ("Google API key", r"\bAIza[0-9A-Za-z_-]{35}\b"),
    ("API key (sk-)", r"\bsk-(?:ant-|proj-|svcacct-)?[A-Za-z0-9_-]{24,}\b"),
    ("Hugging Face token", r"\bhf_[A-Za-z0-9]{30,}\b"),
    ("npm token", r"\bnpm_[A-Za-z0-9]{36}\b"),
    ("JSON web token", r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"),
    ("private key", r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----"),
    ("password / secret", r#"(?i)\b(?:pass(?:word|wd)?|secret|api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret)\s*[=:]\s*["']?[^\s"'<>]{8,}"#),
];

/// Replaces a secret in saved text.
const REDACTED: &str = "‹redacted›";

#[derive(Clone, Debug)]
pub struct Guard {
    pub rules: Vec<(String, Regex)>,
    /// Every rule in one, for scanning the screen.
    pub combined: Regex,
    /// A whole PEM private key, for saved text.
    pem: Regex,
}

impl Default for Guard {
    fn default() -> Self {
        Guard::new(&[]).expect("built-in secret patterns compile")
    }
}

impl Guard {
    /// The built-in kinds plus `extra` (name, regex).
    pub fn new(extra: &[(String, String)]) -> Result<Guard, String> {
        let mut rules = Vec::new();
        for (name, pat) in DEFAULT.iter().map(|(n, p)| (n.to_string(), p.to_string())).chain(extra.iter().cloned()) {
            let re = regex::RegexBuilder::new(&pat).size_limit(1 << 21).build().map_err(|e| format!("secret pattern '{name}': {e}"))?;
            rules.push((name, re));
        }
        let all = rules.iter().map(|(_, r)| format!("(?:{})", r.as_str())).collect::<Vec<_>>().join("|");
        let combined = regex::RegexBuilder::new(&all).size_limit(1 << 23).build().map_err(|e| e.to_string())?;
        let pem = Regex::new(r"(?s)-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----.*?(?:-----END (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----|\z)")
            .expect("pem pattern");
        Ok(Guard { rules, combined, pem })
    }

    /// The kinds of secret in `text` (each once).
    pub fn kinds(&self, text: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (name, re) in &self.rules {
            if re.is_match(text) && !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }

    pub fn has_secret(&self, text: &str) -> bool {
        self.combined.is_match(text)
    }

    /// `text` with every secret replaced (a whole private key block too).
    pub fn redact(&self, text: &str) -> String {
        if !self.combined.is_match(text) {
            return text.to_string();
        }
        let t = self.pem.replace_all(text, REDACTED);
        self.combined.replace_all(&t, REDACTED).into_owned()
    }

    /// A short look at a secret for the paste question: `ghp_…k3Fq`.
    pub fn preview(&self, text: &str) -> Vec<String> {
        self.combined
            .find_iter(text)
            .take(4)
            .map(|m| {
                let s = m.as_str();
                let chars: Vec<char> = s.chars().collect();
                if chars.len() <= 10 {
                    "•".repeat(chars.len())
                } else {
                    let head: String = chars[..4].iter().collect();
                    let tail: String = chars[chars.len() - 4..].iter().collect();
                    format!("{head}{}{tail}", "•".repeat((chars.len() - 8).min(16)))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake keys for the tests, put together at run time so the source never
    /// holds anything a secret scanner would take for a real key.
    fn fake(parts: &[&str]) -> String {
        parts.concat()
    }

    pub(crate) fn fake_github_token() -> String {
        fake(&["gh", "p_", "abcdefghijklmnopqrstuvwxyz", "ABCDEF0123"])
    }

    #[test]
    fn spots_common_secrets() {
        let g = Guard::default();
        for (text, kind) in [
            (fake(&["export AWS_ACCESS_KEY_ID=", "AKIA", "IOSFODNN7EXAMPLE"]), "AWS access key"),
            (fake(&["token ", &fake_github_token()]), "GitHub token"),
            (fake(&["xox", "b-", "123456789012-abcdefghij"]), "Slack token"),
            (fake(&["sk", "_live_", "0000aaaa1111bbbb2222cccc"]), "Stripe key"),
            (fake(&["key=", "AIza", "SyA-1234567890abcdefghijklmnopqrstu"]), "Google API key"),
            (fake(&["API_KEY=", "sk-", "ant-api03-", "abcdefghijklmnopqrstuvwxyz012345"]), "API key (sk-)"),
            (fake(&["eyJhbGciOiJIUzI1NiJ9", ".", "eyJzdWIiOiIxMjM0NTY3ODkwIn0", ".", "dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"]), "JSON web token"),
            (fake(&["-----BEGIN OPENSSH ", "PRIVATE KEY-----"]), "private key"),
            (fake(&["password = ", "hunter2hunter2"]), "password / secret"),
        ] {
            assert!(g.kinds(&text).contains(&kind.to_string()), "{text} -> {:?}", g.kinds(&text));
        }
    }

    #[test]
    fn leaves_ordinary_output_alone() {
        let g = Guard::default();
        for text in [
            "Compiling vibex v0.1.0",
            "commit 7b189ea0c3a1f9e2d4b5a6c7d8e9f0a1b2c3d4e5",
            "password: ",
            "ls -la ~/.ssh",
            "sk-short",
            "https://github.com/Kalyan-Deva/vibex",
        ] {
            assert!(!g.has_secret(text), "{text}");
        }
    }

    #[test]
    fn redacts_saved_text() {
        let g = Guard::default();
        let curl = format!("curl -H 'Authorization: token {}' x", fake_github_token());
        assert_eq!(g.redact(&curl), "curl -H 'Authorization: token ‹redacted›' x");
        let pem = fake(&["cat id\n-----BEGIN OPENSSH ", "PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA\n-----END OPENSSH ", "PRIVATE KEY-----\ndone"]);
        assert_eq!(g.redact(&pem), "cat id\n‹redacted›\ndone");
        assert_eq!(g.redact("nothing here"), "nothing here");
    }

    #[test]
    fn extra_patterns_and_previews() {
        let g = Guard::new(&[("company token".into(), r"\bMYCO-[0-9a-f]{16}\b".into())]).unwrap();
        assert_eq!(g.kinds("t=MYCO-0123456789abcdef"), vec!["company token".to_string()]);
        let p = g.preview(&fake_github_token());
        assert!(p[0].starts_with("ghp_") && p[0].ends_with("0123") && p[0].contains('•'), "{p:?}");
        assert!(Guard::new(&[("bad".into(), "(".into())]).is_err());
    }
}
