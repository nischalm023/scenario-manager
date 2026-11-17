#!/bin/bash
# Inject skill reminders based on prompt analysis
# This hook runs before Claude processes user prompts

# Read the prompt from stdin (Claude passes it via stdin)
PROMPT=$(cat)

REMINDERS=""

# Java file patterns - Desmo-J, simulation, metrics
if echo "$PROMPT" | grep -qiE '\.java|Desmo-?J|SimProcess|@MetricHandler|lifeCycle|process.*schedule'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/java-backend.md for Desmo-J and metrics patterns"
fi

# Java style - commit, review, create class/record
if echo "$PROMPT" | grep -qiE 'commit|review.*java|create.*(class|record|service|controller)|before.*push'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/java-style-checklist.md before committing"
fi

# Frontend patterns
if echo "$PROMPT" | grep -qiE '\.tsx?|frontend|react|component|shadcn|tanstack'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/frontend.md - NEVER modify apps/frontend/src/components/ui/"
fi

# Architecture/gRPC
if echo "$PROMPT" | grep -qiE 'grpc|microservice|service.*boundar|proto|channel.*factory'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/architecture.md for service boundaries"
fi

# Relay system
if echo "$PROMPT" | grep -qiE 'vtm|htm|relay|tote|zone.*capacity|backpressure'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/resources/relay-system.md for VTM/HTM patterns"
fi

# Commands/running services
if echo "$PROMPT" | grep -qiE 'start.*service|run.*server|build|test|docker|nx serve|mvn spring-boot'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/commands.md for development commands"
fi

# Debugging
if echo "$PROMPT" | grep -qiE 'error|not working|fail|debug|issue|connection refused|health check'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/debugging.md for troubleshooting"
fi

# Set cover, animation, comparison features
if echo "$PROMPT" | grep -qiE 'set.?cover|animation|snapshot|comparison|algorithm.*compare'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/resources/feature-guides.md for feature documentation"
fi

# Adding gRPC client
if echo "$PROMPT" | grep -qiE 'add.*grpc|new.*client|grpc.*client|stub'; then
  REMINDERS="$REMINDERS
SKILL_REMINDER: Consult .claude/skills/resources/grpc-patterns.md for adding gRPC clients"
fi

# Output reminders if any were found
if [ -n "$REMINDERS" ]; then
  echo "$REMINDERS"
fi
