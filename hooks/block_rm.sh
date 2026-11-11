#!/bin/bash
# PreToolUse hook: blocks destructive rm invocations.
# Straight from Claude Code's own documented example (code.claude.com/docs/en/hooks).
COMMAND=$(jq -r '.tool_input.command')

if echo "$COMMAND" | grep -qE 'rm -rf|rm -r|rm -f'; then
  jq -n '{
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: "Destructive command blocked by hook"
    }
  }'
else
  exit 0
fi
