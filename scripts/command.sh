#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PID_FILE="$ROOT_DIR/.agent.pid"
LOG_FILE="$ROOT_DIR/.agent.log"

ACTION="${1:-}"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/command.sh <action>

Actions:
  session   Start the Claude-style interactive session (`cargo run`)
  run       Run a one-shot prompt through the agent
  list      Show loaded memory, subagents, commands, and tools
  init      Scaffold Claude-style project files
  check     Run cargo check
  build     Run cargo build
  test      Run cargo test

Environment:
  AGENT_SYSTEM_PROMPT   Override the system prompt
EOF
}

ensure_action() {
  if [[ -z "$ACTION" ]]; then
    usage
    exit 1
  fi
}

default_env() {
  export AGENT_SYSTEM_PROMPT="${AGENT_SYSTEM_PROMPT:-You are a Claude-style Rust coding agent.}"
}

run_session() {
  default_env
  cd "$ROOT_DIR"
  cargo run
}

run_agent() {
  default_env
  cd "$ROOT_DIR"
  cargo run -- run --input "${AGENT_USER_INPUT:-Plan the next refactor step.}"
}

list_agent() {
  default_env
  cd "$ROOT_DIR"
  cargo run -- list
}

init_agent() {
  default_env
  cd "$ROOT_DIR"
  cargo run -- init
}

check_agent() {
  cd "$ROOT_DIR"
  cargo check
}

build_agent() {
  cd "$ROOT_DIR"
  cargo build
}

test_agent() {
  cd "$ROOT_DIR"
  cargo test
}

ensure_action

case "$ACTION" in
  session)
    run_session
    ;;
  run)
    run_agent
    ;;
  list)
    list_agent
    ;;
  init)
    init_agent
    ;;
  check)
    check_agent
    ;;
  build)
    build_agent
    ;;
  test)
    test_agent
    ;;
  help|-h|--help)
    usage
    ;;
  *)
    echo "Unknown action: $ACTION"
    usage
    exit 1
    ;;
esac
