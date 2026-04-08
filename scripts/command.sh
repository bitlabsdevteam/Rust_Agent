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
  run       Run the agent in the foreground
  start     Run the agent in the background
  stop      Stop the background agent
  restart   Restart the background agent
  status    Show background agent status
  check     Run cargo check
  build     Run cargo build
  test      Run cargo test

Environment:
  AGENT_SYSTEM_PROMPT   Override the system prompt
  AGENT_USER_INPUT      Override the user input
EOF
}

ensure_action() {
  if [[ -z "$ACTION" ]]; then
    usage
    exit 1
  fi
}

default_inputs() {
  export AGENT_SYSTEM_PROMPT="${AGENT_SYSTEM_PROMPT:-You are a simple Rust agent.}"
  export AGENT_USER_INPUT="${AGENT_USER_INPUT:-Use tool echo on this message.}"
}

is_running() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
}

start_agent() {
  if [[ -f "$PID_FILE" ]]; then
    local existing_pid
    existing_pid="$(cat "$PID_FILE")"
    if is_running "$existing_pid"; then
      echo "Agent is already running with PID $existing_pid"
      return 0
    fi
    rm -f "$PID_FILE"
  fi

  default_inputs
  cd "$ROOT_DIR"
  nohup cargo run >"$LOG_FILE" 2>&1 &

  local agent_pid=$!
  echo "$agent_pid" >"$PID_FILE"
  echo "Agent started with PID $agent_pid"
  echo "Log file: $LOG_FILE"
}

stop_agent() {
  if [[ ! -f "$PID_FILE" ]]; then
    echo "No PID file found. Agent does not appear to be running."
    return 0
  fi

  local agent_pid
  agent_pid="$(cat "$PID_FILE")"

  if is_running "$agent_pid"; then
    kill "$agent_pid"
    echo "Stopped agent PID $agent_pid"
  else
    echo "PID $agent_pid is not running."
  fi

  rm -f "$PID_FILE"
}

status_agent() {
  if [[ ! -f "$PID_FILE" ]]; then
    echo "Agent is not running."
    return 0
  fi

  local agent_pid
  agent_pid="$(cat "$PID_FILE")"

  if is_running "$agent_pid"; then
    echo "Agent is running with PID $agent_pid"
  else
    echo "PID file exists but process $agent_pid is not running."
    return 1
  fi
}

run_agent() {
  default_inputs
  cd "$ROOT_DIR"
  cargo run
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

restart_agent() {
  stop_agent
  start_agent
}

ensure_action

case "$ACTION" in
  run)
    run_agent
    ;;
  start)
    start_agent
    ;;
  stop)
    stop_agent
    ;;
  restart)
    restart_agent
    ;;
  status)
    status_agent
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
