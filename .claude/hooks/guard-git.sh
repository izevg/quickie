#!/bin/sh
# Claude Code PreToolUse hook for Bash: blocks commands that bypass the quality gates or rewrite
# shared history (docs/quality.md). Exit code 2 blocks the call and shows stderr to the agent.
set -eu

command=$(jq -r '.tool_input.command // empty')

block() {
  printf 'Blocked by .claude/hooks/guard-git.sh: %s. See docs/quality.md.\n' "$1" >&2
  exit 2
}

matches() {
  printf '%s' "$command" | grep -Eq -- "$1"
}

if matches '--no-verify'; then
  block "--no-verify skips the git hooks"
fi
if matches 'git[[:space:]](.*[[:space:]])?commit([[:space:]].*)?[[:space:]]-[a-zA-Z]*n'; then
  block "git commit -n skips the git hooks"
fi
if matches 'core\.hooksPath' && ! matches 'core\.hooksPath[= ]+\.githooks([[:space:];&|]|$)'; then
  block "the git hooks path must stay .githooks"
fi
if matches 'git[[:space:]](.*[[:space:]])?push([[:space:]].*)?[[:space:]](--force([[:space:]]|$)|-[a-zA-Z]*f)'; then
  block "force push (use --force-with-lease on your own branch only, after asking)"
fi
exit 0
