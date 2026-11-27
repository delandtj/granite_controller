#!/usr/bin/env bash
# Granite is not a Hero repo. Keep the hero-bundles skills and the lab CLI
# out of it: lab exports a shared CARGO_TARGET_DIR and rewrites Cargo.toml
# profiles, and the hero skills tell the model to do the same by hand.
#
# Usage (from .claude/settings.json):
#   no-hero.sh session   SessionStart: unset target-dir overrides for every
#                        Bash command of the session (via CLAUDE_ENV_FILE)
#   no-hero.sh pretool   PreToolUse on Bash|Skill: deny lab, target-dir
#                        overrides and hero skills
set -u

deny() {
    jq -n --arg r "$1" '{hookSpecificOutput: {hookEventName: "PreToolUse",
        permissionDecision: "deny", permissionDecisionReason: $r}}'
    exit 0
}

case "${1:-}" in
session)
    if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
        echo 'unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR' >> "$CLAUDE_ENV_FILE"
    fi
    exit 0
    ;;
pretool)
    input=$(cat)
    tool=$(jq -r '.tool_name // empty' <<<"$input")
    case "$tool" in
    Bash)
        cmd=$(jq -r '.tool_input.command // empty' <<<"$input")
        # lab in command position (start, after ; & | ( ` $( or a wrapper
        # like env/exec/xargs), with or without a path prefix. Not "lab"
        # as a word inside an argument such as a commit message.
        if grep -qE '(^|[;&|(`])[[:space:]]*((env|exec|sudo|xargs|nohup|time|[A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*)[[:space:]]+)*([^[:space:];&|]*/)?lab([[:space:]]|$)' <<<"$cmd"; then
            deny "Granite is not a Hero repo: do not run lab here (it redirects the cargo target dir and rewrites Cargo.toml profiles). Use plain cargo."
        fi
        if grep -qE 'CARGO(_BUILD)?_TARGET_DIR[[:space:]]*=|--target-dir' <<<"$cmd"; then
            deny "Granite builds into its own target/ dirs (firmware/target, firmware/plane-fd/target). Do not override CARGO_TARGET_DIR or --target-dir here."
        fi
        ;;
    Skill)
        skill=$(jq -r '.tool_input.skill // empty' <<<"$input")
        if grep -qE '^(hero|herolib)[-_a-z]*:' <<<"$skill"; then
            deny "Hero skills ($skill) do not apply to Granite: its toolchains, branches and build dirs are deliberate. Follow the repo's own conventions."
        fi
        ;;
    esac
    exit 0
    ;;
esac
exit 0
