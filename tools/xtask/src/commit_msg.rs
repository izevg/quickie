//! Commit messages: Conventional Commits, a short header, and no agent attribution.

const TYPES: &[&str] = &[
    "feat", "fix", "perf", "docs", "chore", "ci", "test", "refactor", "style", "build",
];
const MAX_HEADER: usize = 72;
const PASS_THROUGH: &[&str] = &["Merge ", "Revert \"", "fixup! ", "squash! ", "amend! "];
const SCISSORS: &str = "# ------------------------ >8 ------------------------";
const AGENTS: &[&str] = &[
    "claude",
    "anthropic",
    "openai",
    "chatgpt",
    "codex",
    "copilot",
    "cursor",
    "gemini",
    "kimi",
    "qoder",
    "devin",
    "aider",
];

/// Checks a commit message as git hands it to the `commit-msg` hook.
pub fn check(raw: &str) -> Vec<String> {
    let lines: Vec<&str> = raw
        .lines()
        .take_while(|line| *line != SCISSORS)
        .filter(|line| !line.starts_with('#'))
        .collect();
    let Some(header) = lines.iter().find(|line| !line.trim().is_empty()) else {
        return vec!["commit message is empty".to_owned()];
    };
    let mut findings = Vec::new();
    if !PASS_THROUGH.iter().any(|prefix| header.starts_with(prefix)) {
        findings.extend(check_header(header));
    }
    if lines.get(1).is_some_and(|line| !line.trim().is_empty()) {
        findings.push("line 2 must be blank: separate the header from the body".to_owned());
    }
    for line in &lines {
        if let Some(reason) = attribution(line) {
            findings.push(format!(
                "no agent attribution in commits ({reason}): {line}"
            ));
        }
    }
    findings
}

fn check_header(header: &str) -> Vec<String> {
    let mut findings = Vec::new();
    let length = header.chars().count();
    if length > MAX_HEADER {
        findings.push(format!("header is {length} characters (max {MAX_HEADER})"));
    }
    let Some((prefix, description)) = header.split_once(": ") else {
        findings.push(format!(
            "header must be `type(scope)!: description`, got: {header}"
        ));
        return findings;
    };
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let (kind, scope) = match prefix.split_once('(') {
        Some((kind, rest)) => (kind, rest.strip_suffix(')')),
        None => (prefix, Some("x")),
    };
    if !TYPES.contains(&kind) {
        findings.push(format!(
            "unknown type `{kind}`, use one of: {}",
            TYPES.join(" ")
        ));
    }
    if !scope.is_some_and(valid_scope) {
        findings.push(format!("scope must be lowercase `a-z0-9-/`, got: {prefix}"));
    }
    if description.trim().is_empty() || description.starts_with(' ') {
        findings.push("description after `: ` must not be empty".to_owned());
    }
    findings
}

fn valid_scope(scope: &str) -> bool {
    !scope.is_empty()
        && scope
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/')
}

fn attribution(line: &str) -> Option<&'static str> {
    let lower = line.to_lowercase();
    if lower.contains("generated with") || line.contains('\u{1f916}') {
        return Some("generated-with footer");
    }
    let trailer = lower.starts_with("co-authored-by:") || lower.starts_with("signed-off-by:");
    if trailer && AGENTS.iter().any(|agent| lower.contains(agent)) {
        return Some("agent trailer");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn accepts_conventional_headers() {
        for header in [
            "feat: add the command palette",
            "fix(palette): keep focus after escape",
            "feat(engine/bridge)!: change the tab api",
            "docs: translate the architecture",
        ] {
            assert_eq!(check(header), Vec::<String>::new(), "{header}");
        }
    }

    #[test]
    fn accepts_a_body_and_human_co_authors() {
        let message = "feat: x\n\nWhy it matters.\n\nCo-Authored-By: A Person <a@example.org>\n";
        assert_eq!(check(message), Vec::<String>::new());
    }

    #[test]
    fn ignores_git_comments_and_the_scissors_part() {
        let message = "fix: y\n# Please enter the commit message\n# ------------------------ >8 ------------------------\nCo-Authored-By: Claude <noreply@anthropic.com>\n";
        assert_eq!(check(message), Vec::<String>::new());
    }

    #[test]
    fn passes_merge_and_fixup_headers() {
        assert_eq!(check("Merge branch 'x'"), Vec::<String>::new());
        assert_eq!(check("fixup! feat: x"), Vec::<String>::new());
    }

    #[test]
    fn rejects_an_empty_message() {
        assert_eq!(check("# only a comment\n"), ["commit message is empty"]);
    }

    #[test]
    fn rejects_unknown_types_and_missing_separators() {
        assert_eq!(check("feature: x").len(), 1);
        assert_eq!(check("feat x").len(), 1);
        assert_eq!(check("feat:x").len(), 1);
    }

    #[test]
    fn rejects_bad_scopes_and_empty_descriptions() {
        assert_eq!(check("feat(Palette): x").len(), 1);
        assert_eq!(check("feat(): x").len(), 1);
        assert_eq!(check("feat(x: y").len(), 1);
        assert_eq!(check("feat:  ").len(), 1);
    }

    #[test]
    fn rejects_long_headers() {
        let header = format!("feat: {}", "a".repeat(67));
        assert_eq!(check(&header), ["header is 73 characters (max 72)"]);
        assert_eq!(
            check(&format!("feat: {}", "a".repeat(66))),
            Vec::<String>::new()
        );
    }

    #[test]
    fn rejects_a_body_glued_to_the_header() {
        assert_eq!(check("feat: x\nbody").len(), 1);
    }

    #[test]
    fn rejects_agent_attribution() {
        for footer in [
            "Co-Authored-By: Claude Opus <noreply@anthropic.com>",
            "co-authored-by: Codex <codex@openai.com>",
            "Generated with [Claude Code](https://claude.com/claude-code)",
            "\u{1f916} via an assistant",
        ] {
            let message = format!("feat: x\n\n{footer}\n");
            assert_eq!(check(&message).len(), 1, "{footer}");
        }
    }
}
