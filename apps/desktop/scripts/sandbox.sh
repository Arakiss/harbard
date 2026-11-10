#!/usr/bin/env bash
# A throwaway copy of the bus for trying the app without touching the real one.
#
#   scripts/sandbox.sh start <dir> <port>   copy ~/.harbard to <dir>, run serve on <port>
#   scripts/sandbox.sh stop <dir>           stop that serve
#   scripts/sandbox.sh env <dir> <port>     print the env the app needs
#
# Writes the app makes (replies, reads, renames) land in the copy, so no live
# agent session ever sees them. Append test events with
# `HARBARD_HOME=<dir> harbard note ...` to watch a live update arrive.
set -euo pipefail
cmd=${1:?start|stop|env}
dir=${2:?sandbox dir}
port=${3:-7519}
src=${HARBARD_SANDBOX_SOURCE:-$HOME/.harbard}
pidfile="${dir:?}/sandbox-serve.pid"
case "$cmd" in
  start)
    if [[ "$port" == 7419 ]]; then echo "refusing the live port 7419" >&2; exit 2; fi
    mkdir -p "$dir"
    rsync -a --delete --exclude 'logs/' --exclude '*.lock' --exclude 'hub.token' "$src/" "$dir/"
    HARBARD_HOME="$dir" nohup harbard serve --port "$port" >"$dir/sandbox-serve.log" 2>&1 &
    echo $! >"$pidfile"
    for _ in $(seq 1 50); do
      [[ -f "$dir/hub.token" ]] && curl -sf -o /dev/null -H "Authorization: Bearer $(cat "$dir/hub.token")" "http://127.0.0.1:$port/api/v1/meta" && break
      sleep 0.2
    done
    echo "sandbox serve pid $(cat "$pidfile") on :$port, home $dir"
    ;;
  stop)
    if [[ -f "$pidfile" ]]; then
      kill "$(cat "$pidfile")" 2>/dev/null || true
      : >"$pidfile"
    fi
    ;;
  env)
    echo "HARBARD_HOME=$dir HARBARD_API_PORT=$port"
    ;;
esac
