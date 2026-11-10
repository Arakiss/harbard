#!/bin/sh
# harbard installer.
#
#   curl -fsSL https://raw.githubusercontent.com/Arakiss/harbard/main/scripts/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/Arakiss/harbard/main/scripts/install.sh | sh -s -- --help
#
# Installs the newest release of the CLI (checksum verified, and signature
# verified when cosign is present), wires the hooks of the coding agents it
# finds (Claude Code, Codex, Cursor) and checks the result with `harbard
# doctor`. On macOS it also builds the desktop app from a source checkout
# under Application Support, installs the background services the app needs
# and opens it. Running it again updates what is there.
#
# Environment (all optional):
#   HARBARD_VERSION       release tag to install (default: the newest, pre-releases included)
#   HARBARD_INSTALL_DIR   where the CLI goes (default: ~/.local/bin)
#   HARBARD_HOME          harbard's data directory (default: ~/.harbard)
#   HARBARD_SOURCE_DIR    checkout the desktop app is built from
#                         (default: ~/Library/Application Support/harbard/source)
#   HARBARD_INSTALL_FROM  a directory holding harbard-<target>.tar.gz and its
#                         .sha256 (and .sigstore.json): install from there
#   HARBARD_NO_COSIGN=1   skip the signature check even when cosign is present
#   NO_COLOR              no colour
# For tests only:
#   HARBARD_INSTALL_OS, HARBARD_INSTALL_ARCH   stand in for `uname -s` / `uname -m`
#
# The whole script is functions until the last line, so a download cut short
# runs nothing.

set -u

REPO="${HARBARD_REPO:-Arakiss/harbard}"
INSTALL_URL="https://raw.githubusercontent.com/$REPO/main/scripts/install.sh"
PATH_MARK="# Added by the harbard installer"
AGENT_LABELS="com.harbard.serve com.harbard.watchdog"
# The web console's agent, retired when the app started drawing it itself.
RETIRED_LABEL="com.harbard.console"

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

ui_init() {
  TTY=0
  COLOR=0
  if [ -t 1 ] && [ "${TERM:-dumb}" != "dumb" ]; then
    TTY=1
    [ -z "${NO_COLOR:-}" ] && COLOR=1
  fi
  INTERACTIVE=0
  if [ "$TTY" = 1 ] && (: </dev/tty) 2>/dev/null; then
    INTERACTIVE=1
  fi
  ESC="$(printf '\033')"
  if [ "$COLOR" = 1 ]; then
    C_RESET="${ESC}[0m"; C_BOLD="${ESC}[1m"; C_DIM="${ESC}[2m"
    C_RED="${ESC}[31m"; C_GREEN="${ESC}[32m"; C_YELLOW="${ESC}[33m"; C_CYAN="${ESC}[36m"
  else
    C_RESET=""; C_BOLD=""; C_DIM=""; C_RED=""; C_GREEN=""; C_YELLOW=""; C_CYAN=""
  fi
  UTF=0
  case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
    *UTF-8* | *utf-8* | *UTF8* | *utf8*) UTF=1 ;;
  esac
  if [ "$UTF" = 1 ]; then
    G_OK="✓"; G_FAIL="✗"; G_SKIP="–"; G_WARN="!"; G_DRY="○"; G_BAR="│"; G_DOT="·"; G_ARROW="→"
  else
    G_OK="ok"; G_FAIL="x"; G_SKIP="-"; G_WARN="!"; G_DRY="o"; G_BAR="|"; G_DOT="-"; G_ARROW="->"
  fi
  COLS=80
  if [ "$TTY" = 1 ]; then
    c="$(stty size </dev/tty 2>/dev/null | cut -d' ' -f2)"
    case "$c" in '' | *[!0-9]*) ;; *) COLS="$c" ;; esac
  fi
  SPIN_PID=""
}

# All user-facing output goes to fd 3, which is the original stdout: a step
# runs with its own stdout and stderr in the log.
say() { printf '%s\n' "$*" >&3; }
blank() { printf '\n' >&3; }
err() { printf '%s\n' "$*" >&2; }

fmt_secs() {
  if [ "$1" -lt 1 ]; then
    printf '<1s'
  elif [ "$1" -lt 60 ]; then
    printf '%ss' "$1"
  else
    printf '%sm %02ds' "$(($1 / 60))" "$(($1 % 60))"
  fi
}

# A path as the user reads it: ~ for their home.
tilde() {
  case "$1" in
    "$HOME") printf '~' ;;
    "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;;
    *) printf '%s' "$1" ;;
  esac
}

now() { date +%s; }

# The last meaningful line a step wrote to the log, for the spinner.
activity() {
  tail -n 30 "$LOG" 2>/dev/null | tr '\r' '\n' | sed "s/${ESC}\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//" |
    grep -v '^$' | grep -v '^== ' | tail -n 1 | cut -c "1-$1"
}

spin_loop() {
  sl_i=0
  sl_act=""
  sl_room=$((COLS - ${#STEP_TITLE} - 24))
  [ "$sl_room" -lt 10 ] && sl_room=10
  while :; do
    if [ "$UTF" = 1 ]; then
      case $((sl_i % 10)) in
        0) sl_f="⠋" ;; 1) sl_f="⠙" ;; 2) sl_f="⠹" ;; 3) sl_f="⠸" ;; 4) sl_f="⠼" ;;
        5) sl_f="⠴" ;; 6) sl_f="⠦" ;; 7) sl_f="⠧" ;; 8) sl_f="⠇" ;; *) sl_f="⠏" ;;
      esac
    else
      case $((sl_i % 4)) in 0) sl_f="|" ;; 1) sl_f="/" ;; 2) sl_f="-" ;; *) sl_f="\\" ;; esac
    fi
    [ $((sl_i % 5)) -eq 0 ] && sl_act="$(activity "$sl_room")"
    sl_el="$(fmt_secs $(($(now) - STEP_T0)))"
    printf '\r%s[2K  %s%s%s %s  %s%s%s%s' "$ESC" "$C_CYAN" "$sl_f" "$C_RESET" "$SPIN_LABEL" \
      "$C_DIM" "$sl_el" "${sl_act:+ $G_DOT $sl_act}" "$C_RESET"
    sl_i=$((sl_i + 1))
    sleep 0.1
  done
}

spin_start() {
  [ "$TTY" = 1 ] || return 0
  printf '%s[?25l' "$ESC" >&3
  spin_loop >&3 2>/dev/null &
  SPIN_PID=$!
}

spin_stop() {
  if [ -n "$SPIN_PID" ]; then
    kill "$SPIN_PID" 2>/dev/null
    wait "$SPIN_PID" 2>/dev/null
    SPIN_PID=""
    printf '\r%s[2K%s[?25h' "$ESC" "$ESC" >&3
  fi
}

# Detail lines under a step: `detail`, and `note` for a warning.
detail() { STEP_DETAILS="${STEP_DETAILS}  ${1}
"; }
note() { STEP_DETAILS="${STEP_DETAILS}${G_WARN} ${1}
"; }

print_details() {
  [ -n "$STEP_DETAILS" ] || return 0
  printf '%s' "$STEP_DETAILS" | while IFS= read -r pd_line; do
    case "$pd_line" in
      "$G_WARN "*) say "        ${C_YELLOW}${pd_line}${C_RESET}" ;;
      *) say "        ${C_DIM}${pd_line}${C_RESET}" ;;
    esac
  done
}

# step <title> <function>: run a function as a numbered step. The function
# sets STEP_STATUS (ok | skip | warn) and STEP_FAIL / STEP_HINT on failure.
step() {
  STEP_N=$((STEP_N + 1))
  STEP_TITLE="$1"
  STEP_DETAILS=""
  STEP_STATUS="ok"
  STEP_FAIL=""
  STEP_HINT=""
  st_num="${STEP_N}/${STEP_TOTAL}"
  SPIN_LABEL="${C_DIM}${st_num}${C_RESET}  ${STEP_TITLE}"
  printf '\n== %s %s\n' "$st_num" "$STEP_TITLE" >>"$LOG"
  if [ "$TTY" = 0 ]; then
    printf '[%s] %s ... ' "$st_num" "$STEP_TITLE" >&3
  fi
  STEP_T0="$(now)"
  spin_start
  "$2" >>"$LOG" 2>&1
  st_rc=$?
  spin_stop
  st_took="$(fmt_secs $(($(now) - STEP_T0)))"
  [ "$st_rc" -ne 0 ] && STEP_STATUS="fail"
  [ "$DRY_RUN" = 1 ] && [ "$STEP_STATUS" = "ok" ] && STEP_STATUS="dry"
  if [ "$TTY" = 1 ]; then
    case "$STEP_STATUS" in
      ok) st_g="${C_GREEN}${G_OK}${C_RESET}"; st_t="$st_took" ;;
      warn) st_g="${C_YELLOW}${G_WARN}${C_RESET}"; st_t="$st_took" ;;
      skip) st_g="${C_DIM}${G_SKIP}${C_RESET}"; st_t="skipped" ;;
      dry) st_g="${C_CYAN}${G_DRY}${C_RESET}"; st_t="dry run" ;;
      *) st_g="${C_RED}${G_FAIL}${C_RESET}"; st_t="failed after $st_took" ;;
    esac
    printf '  %s %s%-5s%s %-44s %s%s%s\n' "$st_g" "$C_DIM" "$st_num" "$C_RESET" "$STEP_TITLE" \
      "$C_DIM" "$st_t" "$C_RESET" >&3
  else
    case "$STEP_STATUS" in
      ok) printf 'ok (%s)\n' "$st_took" >&3 ;;
      warn) printf 'ok, with warnings (%s)\n' "$st_took" >&3 ;;
      skip) printf 'skipped\n' >&3 ;;
      dry) printf 'dry run\n' >&3 ;;
      *) printf 'FAILED (%s)\n' "$st_took" >&3 ;;
    esac
  fi
  print_details
  [ "$STEP_STATUS" = "fail" ] && fail_panel
  return 0
}

fail_panel() {
  blank
  say "  ${C_RED}${C_BOLD}Step ${STEP_N} of ${STEP_TOTAL} failed:${C_RESET} ${STEP_TITLE}"
  [ -n "$STEP_FAIL" ] && say "  ${STEP_FAIL}"
  fp_tail="$(sed -n '/^== '"${STEP_N}"'\//,$p' "$LOG" | sed '1d' | grep -v '^[[:space:]]*$' | tail -n 15 |
    sed "s/${ESC}\[[0-9;?]*[A-Za-z]//g")"
  if [ -n "$fp_tail" ]; then
    blank
    say "  ${C_DIM}Last lines of the log:${C_RESET}"
    printf '%s\n' "$fp_tail" | while IFS= read -r fp_line; do say "  ${C_DIM}${G_BAR}${C_RESET} $fp_line"; done
  fi
  blank
  [ "$LOG_KEEP" = 1 ] && say "  Full log   $(tilde "$LOG")"
  say "  Next       ${STEP_HINT:-fix the cause above, then run the installer again; finished steps are kept}"
  blank
  exit 1
}

die() {
  spin_stop
  blank
  say "  ${C_RED}${C_BOLD}${G_FAIL} $1${C_RESET}"
  shift
  for d_line in "$@"; do say "    $d_line"; done
  blank
  exit 1
}

on_exit() {
  spin_stop
  [ -n "${WORK:-}" ] && rm -rf "${WORK:?}"
  [ -n "${STAGE:-}" ] && rm -rf "${STAGE:?}"
}

on_signal() {
  spin_stop
  blank
  say "  ${C_RED}${G_FAIL} Interrupted.${C_RESET} Run the installer again to finish; finished steps are kept."
  exit 130
}

# ---------------------------------------------------------------------------
# Arguments and platform
# ---------------------------------------------------------------------------

usage() {
  cat <<EOF
harbard installer

Usage: curl -fsSL $INSTALL_URL | sh -s -- [options]

Options:
  --dry-run        print every action and change nothing
  -y, --yes        never ask (the installer asks at most once, before installing build tools)
  --no-app         macOS: install the CLI and hooks only, without the desktop app
  --version <tag>  install this release, such as v0.3.0-alpha.10 (default: the newest)
  --no-modify-path do not add the CLI's directory to your shell profile
  --uninstall      remove what the installer added; your data stays
  --purge          with --uninstall, also delete harbard's data (~/.harbard)
  -h, --help       show this help

Environment: HARBARD_INSTALL_DIR (default ~/.local/bin), HARBARD_HOME,
HARBARD_SOURCE_DIR, HARBARD_INSTALL_FROM, NO_COLOR. See the top of the script.
EOF
}

parse_args() {
  DRY_RUN=0
  YES=0
  APP=1
  UNINSTALL=0
  PURGE=0
  MODIFY_PATH=1
  PIN="${HARBARD_VERSION:-}"
  while [ $# -gt 0 ]; do
    case "$1" in
      --dry-run) DRY_RUN=1 ;;
      -y | --yes) YES=1 ;;
      --no-app) APP=0 ;;
      --no-modify-path) MODIFY_PATH=0 ;;
      --uninstall) UNINSTALL=1 ;;
      --purge) PURGE=1 ;;
      --version)
        [ $# -ge 2 ] || { err "harbard installer: --version needs a tag, such as v0.3.0-alpha.10"; exit 2; }
        PIN="$2"
        shift
        ;;
      --version=*) PIN="${1#--version=}" ;;
      -h | --help) usage; exit 0 ;;
      *) err "harbard installer: unknown option $1 (see --help)"; exit 2 ;;
    esac
    shift
  done
  if [ "$PURGE" = 1 ] && [ "$UNINSTALL" = 0 ]; then
    err "harbard installer: --purge goes with --uninstall"
    exit 2
  fi
  if [ -n "$PIN" ]; then
    case "$PIN" in v*) ;; *) PIN="v$PIN" ;; esac
  fi
}

detect_platform() {
  dp_os="${HARBARD_INSTALL_OS:-$(uname -s)}"
  dp_arch="${HARBARD_INSTALL_ARCH:-$(uname -m)}"
  case "$dp_os" in
    Darwin) OS="macos" ;;
    Linux) OS="linux" ;;
    MINGW* | MSYS* | CYGWIN* | Windows*)
      die "harbard does not run on Windows yet." "Use it inside WSL 2, where this installer works as on Linux."
      ;;
    *)
      die "harbard does not support $dp_os." "It runs on macOS and Linux. To try another system, build from source:" \
        "https://github.com/$REPO#install"
      ;;
  esac
  case "$dp_arch" in
    x86_64 | amd64) ARCH="x86_64" ;;
    arm64 | aarch64) ARCH="aarch64" ;;
    *) die "harbard has no build for the $dp_arch processor." "Prebuilt binaries exist for x86_64 and arm64." ;;
  esac
  # A shell under Rosetta reports x86_64 on Apple silicon: install the native build.
  if [ "$OS" = "macos" ] && [ "$ARCH" = "x86_64" ] && [ -z "${HARBARD_INSTALL_ARCH:-}" ] &&
    [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = "1" ]; then
    ARCH="aarch64"
  fi
  if [ "$OS" = "macos" ]; then
    TARGET="$ARCH-apple-darwin"
    dp_ver="$(sw_vers -productVersion 2>/dev/null || true)"
    if [ "$ARCH" = "aarch64" ]; then dp_chip="Apple silicon"; else dp_chip="Intel"; fi
    OS_LABEL="macOS${dp_ver:+ $dp_ver} $G_DOT $dp_chip"
  else
    TARGET="$ARCH-unknown-linux-gnu"
    if ldd --version 2>&1 | grep -qi musl; then
      die "This Linux uses musl; the prebuilt harbard needs glibc." \
        "Build from source instead: https://github.com/$REPO#install"
    fi
    dp_name="$(sed -n 's/^PRETTY_NAME="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' /etc/os-release 2>/dev/null | head -n 1)"
    OS_LABEL="${dp_name:-Linux} $G_DOT $ARCH"
  fi
  if [ "$OS" != "macos" ]; then APP=0; fi
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "This installer needs \`$1\`, which is not installed." "$2"
}

# ---------------------------------------------------------------------------
# Small tools
# ---------------------------------------------------------------------------

fetch() { # url dest
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https' --tlsv1.2 --retry 3 --connect-timeout 15 -o "$2" "$1"
  else
    wget -q --https-only -O "$2" "$1"
  fi
}

fetch_text() { # url
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https' --tlsv1.2 --retry 2 --connect-timeout 15 "$1"
  else
    wget -q --https-only -O - "$1"
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    openssl dgst -sha256 "$1" | sed 's/.*= *//'
  fi
}

lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }

# The receipt: what this installer put where, one `key=value` per line. The
# uninstaller removes exactly what it lists.
receipt_get() { sed -n "s/^$1=//p" "$RECEIPT" 2>/dev/null | head -n 1; }
receipt_all() { sed -n "s/^$1=//p" "$RECEIPT" 2>/dev/null; }

receipt_set() { # key value
  [ "$DRY_RUN" = 1 ] && return 0
  mkdir -p "$DATA_DIR"
  { grep -v "^$1=" "$RECEIPT" 2>/dev/null; printf '%s=%s\n' "$1" "$2"; } >"$RECEIPT.tmp" && mv -f "$RECEIPT.tmp" "$RECEIPT"
}

receipt_add() { # key value (one of many)
  [ "$DRY_RUN" = 1 ] && return 0
  receipt_all "$1" | grep -Fqx -- "$2" && return 0
  mkdir -p "$DATA_DIR"
  printf '%s=%s\n' "$1" "$2" >>"$RECEIPT"
}

receipt_drop() { # key [value]
  [ "$DRY_RUN" = 1 ] && return 0
  [ -f "$RECEIPT" ] || return 0
  if [ $# -ge 2 ]; then
    grep -Fvx -- "$1=$2" "$RECEIPT" >"$RECEIPT.tmp"
  else
    grep -v "^$1=" "$RECEIPT" >"$RECEIPT.tmp"
  fi
  mv -f "$RECEIPT.tmp" "$RECEIPT"
}

# The harness a setup target names, as people know it.
harness_name() {
  case "$1" in claude) printf 'Claude Code' ;; codex) printf 'Codex' ;; *) printf 'Cursor' ;; esac
}

# Why harbard will consider a harness present (the same rule as `harbard
# setup`: its command on PATH, or its configuration folder).
harness_seen() {
  case "$1" in
    claude)
      if command -v claude >/dev/null 2>&1; then printf 'claude on PATH'
      elif [ -d "$HOME/.claude" ]; then tilde "$HOME/.claude"; fi
      ;;
    codex)
      if command -v codex >/dev/null 2>&1; then printf 'codex on PATH'
      elif [ -d "${CODEX_HOME:-$HOME/.codex}" ]; then tilde "${CODEX_HOME:-$HOME/.codex}"; fi
      ;;
    cursor)
      if command -v cursor >/dev/null 2>&1; then printf 'cursor on PATH'
      elif command -v cursor-agent >/dev/null 2>&1; then printf 'cursor-agent on PATH'
      elif [ -d "$HOME/.cursor" ]; then tilde "$HOME/.cursor"; fi
      ;;
  esac
}

harness_config() {
  case "$1" in
    claude) printf '%s' "$HOME/.claude/settings.json" ;;
    codex) printf '%s' "${CODEX_HOME:-$HOME/.codex}/hooks.json" ;;
    *) printf '%s' "$HOME/.cursor/hooks.json" ;;
  esac
}

# The WorkingDirectory of a LaunchAgent plist, or nothing.
plist_workdir() {
  pw_v="$(plutil -extract WorkingDirectory raw -o - "$1" 2>/dev/null)" ||
    pw_v="$(tr -d '\n' <"$1" | sed -n 's#.*<key>WorkingDirectory</key>[[:space:]]*<string>\([^<]*\)</string>.*#\1#p')"
  printf '%s' "$pw_v"
}

# ---------------------------------------------------------------------------
# Plan
# ---------------------------------------------------------------------------

resolve_paths() {
  BIN_DIR="${HARBARD_INSTALL_DIR:-$HOME/.local/bin}"
  case "$BIN_DIR" in /*) ;; *) BIN_DIR="$(pwd)/$BIN_DIR" ;; esac
  BIN="$BIN_DIR/harbard"
  DATA_DIR="${HARBARD_HOME:-$HOME/.harbard}"
  RECEIPT="$DATA_DIR/install-receipt"
  SUPPORT_DIR="$HOME/Library/Application Support/harbard"
  SOURCE_DIR="${HARBARD_SOURCE_DIR:-$SUPPORT_DIR/source}"
  AGENT_DIR="$HOME/Library/LaunchAgents"
  GIT_URL="https://github.com/$REPO.git"
  ORIG_PATH="$PATH"
  PATH="$BIN_DIR:$PATH"
  export PATH
}

resolve_version() {
  if [ -n "${HARBARD_INSTALL_FROM:-}" ]; then
    [ -d "$HARBARD_INSTALL_FROM" ] || die "HARBARD_INSTALL_FROM is not a directory: $HARBARD_INSTALL_FROM"
    VERSION="${PIN:-local}"
    SOURCE_LABEL="$(tilde "$HARBARD_INSTALL_FROM")"
    return 0
  fi
  SOURCE_LABEL="github.com/$REPO"
  if [ -n "$PIN" ]; then
    VERSION="$PIN"
    return 0
  fi
  # Releases are pre-releases for now, which `releases/latest` skips.
  rv_tag="$(fetch_text "https://api.github.com/repos/$REPO/releases?per_page=1" 2>/dev/null |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  if [ -z "$rv_tag" ]; then
    # Rate-limited API (a shared IP): the Atom feed lists the same releases.
    rv_tag="$(fetch_text "https://github.com/$REPO/releases.atom" 2>/dev/null |
      sed -n 's#.*/releases/tag/\([^"<]*\)".*#\1#p' | head -n 1)"
  fi
  [ -n "$rv_tag" ] || die "Could not find a published harbard release." \
    "Check your connection, or pin one: sh -s -- --version v0.3.0-alpha.10" \
    "Releases: https://github.com/$REPO/releases"
  VERSION="$rv_tag"
}

# The desktop app is built on this Mac: what it needs and what is missing.
check_app_prereqs() {
  MISSING=""
  CLT_OK=1
  xcode-select -p >/dev/null 2>&1 || CLT_OK=0
  [ -x "$HOME/.cargo/bin/cargo" ] && PATH="$HOME/.cargo/bin:$PATH"
  export PATH
  command -v cargo >/dev/null 2>&1 || MISSING="$MISSING rust"
  MISSING="${MISSING# }"
}

# Services that already run from another checkout (a developer's own) are
# not this installer's to replace.
check_service_owner() {
  for co_label in serve; do
    co_plist="$AGENT_DIR/com.harbard.$co_label.plist"
    [ -f "$co_plist" ] || continue
    co_dir="$(plist_workdir "$co_plist")"
    [ -n "$co_dir" ] || continue
    case "$co_dir" in
      "$SOURCE_DIR" | "$SOURCE_DIR"/*) ;;
      *) FOREIGN_SERVICES="$co_dir"; return 0 ;;
    esac
  done
  FOREIGN_SERVICES=""
}

header() {
  blank
  if [ "$UNINSTALL" = 1 ]; then
    say "  ${C_BOLD}harbard${C_RESET}  ${C_DIM}uninstall${C_RESET}"
  else
    say "  ${C_BOLD}harbard${C_RESET}  ${C_CYAN}${VERSION}${C_RESET}  ${C_DIM}${G_DOT} coordination for parallel coding agents${C_RESET}"
  fi
  blank
  say "  ${C_DIM}System ${C_RESET}   $OS_LABEL"
  if [ "$UNINSTALL" = 0 ]; then
    say "  ${C_DIM}From   ${C_RESET}   $SOURCE_LABEL"
    if [ "$APP" = 1 ]; then
      say "  ${C_DIM}Install${C_RESET}   CLI $G_DOT hooks $G_DOT background services $G_DOT desktop app"
    elif [ "$OS" = "macos" ]; then
      say "  ${C_DIM}Install${C_RESET}   CLI $G_DOT hooks  ${C_DIM}(no desktop app: --no-app)${C_RESET}"
    else
      say "  ${C_DIM}Install${C_RESET}   CLI $G_DOT hooks  ${C_DIM}(the desktop app is macOS-only for now)${C_RESET}"
    fi
  fi
  [ "$DRY_RUN" = 1 ] && say "  ${C_DIM}Mode   ${C_RESET}   ${C_YELLOW}dry run: nothing is changed${C_RESET}"
}

confirm_prereqs() {
  [ -n "$MISSING" ] || return 0
  [ "$YES" = 1 ] || [ "$INTERACTIVE" = 0 ] || [ "$DRY_RUN" = 1 ] && return 0
  blank
  say "  The desktop app is built on this Mac and needs:"
  for cp_tool in $MISSING; do
    case "$cp_tool" in
      rust) say "    $G_DOT Rust, with rustup (https://rustup.rs), into ~/.cargo and ~/.rustup" ;;
    esac
  done
  printf '  Install them now? [Y/n] ' >&3
  cp_answer=""
  read -r cp_answer </dev/tty || cp_answer="n"
  case "$cp_answer" in
    "" | y | Y | yes | YES | Yes) ;;
    *)
      APP=0
      say "  ${C_DIM}Continuing without the desktop app. Run the installer again to add it.${C_RESET}"
      ;;
  esac
}

# ---------------------------------------------------------------------------
# Install steps
# ---------------------------------------------------------------------------

step_download() {
  ASSET="harbard-$TARGET.tar.gz"
  TARBALL="$WORK/$ASSET"
  if [ -n "${HARBARD_INSTALL_FROM:-}" ]; then
    sd_from="$HARBARD_INSTALL_FROM/$ASSET"
  else
    sd_from="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
  fi
  STEP_HINT="check your connection and that $VERSION has a build for $TARGET (https://github.com/$REPO/releases), then run the installer again"
  if [ -n "${HARBARD_INSTALL_FROM:-}" ]; then
    [ -f "$sd_from" ] && [ -f "$sd_from.sha256" ] || {
      STEP_FAIL="$(tilde "$HARBARD_INSTALL_FROM") has no $ASSET with its .sha256"
      STEP_HINT="package one, or unset HARBARD_INSTALL_FROM to download the release"
      return 1
    }
    cp "$sd_from.sha256" "$TARBALL.sha256" || return 1
  else
    fetch "$sd_from.sha256" "$TARBALL.sha256" || {
      STEP_FAIL="could not download the checksum of $ASSET for $VERSION"
      return 1
    }
  fi
  EXPECTED="$(lower "$(cut -d' ' -f1 <"$TARBALL.sha256" | head -n 1)")"
  case "$EXPECTED" in
    ????????????????????????????????????????????????????????????????) ;;
    *) STEP_FAIL="the published checksum file is not a sha256"; return 1 ;;
  esac
  if [ "$(receipt_get cli_sha256)" = "$EXPECTED" ] && [ -x "$BIN" ] && "$BIN" --version >/dev/null 2>&1; then
    CLI_CURRENT=1
    STEP_STATUS="skip"
    detail "$($BIN --version) is already installed, from the same archive"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then
    detail "would download $sd_from"
    detail "would check it against the published sha256 (${EXPECTED%"${EXPECTED#????????????}"}...)"
    if [ -n "${HARBARD_NO_COSIGN:-}" ] || ! command -v cosign >/dev/null 2>&1; then
      detail "would skip the signature check (cosign is not installed)"
    else
      detail "would verify its signature with cosign"
    fi
    return 0
  fi
  if [ -n "${HARBARD_INSTALL_FROM:-}" ]; then
    cp "$sd_from" "$TARBALL" || return 1
    [ -f "$sd_from.sigstore.json" ] && cp "$sd_from.sigstore.json" "$TARBALL.sigstore.json"
  else
    fetch "$sd_from" "$TARBALL" || { STEP_FAIL="could not download $sd_from"; return 1; }
  fi
  sd_got="$(sha256_of "$TARBALL")"
  if [ "$sd_got" != "$EXPECTED" ]; then
    STEP_FAIL="checksum mismatch: the archive is not the one published, so it is refused.
  expected $EXPECTED
  got      $sd_got"
    STEP_HINT="run the installer again; if it repeats, report it at https://github.com/$REPO/issues"
    rm -f "$TARBALL"
    return 1
  fi
  detail "sha256 matches the published checksum (${EXPECTED%"${EXPECTED#????????????}"}...)"
  if [ -n "${HARBARD_NO_COSIGN:-}" ]; then
    detail "signature not checked (HARBARD_NO_COSIGN is set)"
  elif ! command -v cosign >/dev/null 2>&1; then
    detail "signature not checked: cosign is not installed (the checksum is)"
  else
    if [ -z "${HARBARD_INSTALL_FROM:-}" ]; then
      fetch "$sd_from.sigstore.json" "$TARBALL.sigstore.json" || {
        STEP_FAIL="the release has no signature bundle ($ASSET.sigstore.json), so it is refused"
        STEP_HINT="set HARBARD_NO_COSIGN=1 to rely on the checksum alone"
        return 1
      }
    fi
    if [ ! -f "$TARBALL.sigstore.json" ]; then
      detail "signature not checked: no .sigstore.json beside the local archive"
    else
      sd_id="https://github.com/$REPO/.github/workflows/release.yml@refs/tags/"
      if [ "$VERSION" = "local" ]; then
        set -- --certificate-identity-regexp "^$(printf '%s' "$sd_id" | sed 's/[.]/\\./g')v"
      else
        set -- --certificate-identity "$sd_id$VERSION"
      fi
      if cosign verify-blob "$TARBALL" --bundle "$TARBALL.sigstore.json" "$@" \
        --certificate-oidc-issuer https://token.actions.githubusercontent.com; then
        detail "signature verified with cosign: built by $REPO's release workflow"
      else
        STEP_FAIL="cosign could not verify the signature, so the archive is refused"
        STEP_HINT="update cosign and run the installer again, or set HARBARD_NO_COSIGN=1 to rely on the checksum alone"
        return 1
      fi
    fi
  fi
}

# Sign a binary the way install-cli.sh does: macOS kills an unsigned or
# stale-signed binary that launchd or a hook runner starts.
sign_binary() {
  if codesign --verify --strict "$1" 2>/dev/null &&
    ! codesign -dv "$1" 2>&1 | grep -q 'Signature=adhoc'; then
    detail "kept its signature ($(codesign -dv "$1" 2>&1 | sed -n 's/^Authority=//p' | head -n 1))"
    return 0
  fi
  sb_id="${HARBARD_CODESIGN_IDENTITY:-}"
  if [ -z "$sb_id" ] && [ -r "$DATA_DIR/codesign-identity" ]; then
    sb_id="$(head -n 1 "$DATA_DIR/codesign-identity" | tr -d '\r')"
  fi
  if [ -n "$sb_id" ] && codesign --force --identifier com.harbard.cli --sign "$sb_id" "$1"; then
    detail "signed with \"$sb_id\""
    return 0
  fi
  codesign --force --sign - "$1" || return 1
  detail "signed ad hoc (macOS runs only signed binaries from launchd and hooks)"
}

step_cli() {
  STEP_HINT="check that $(tilde "$BIN_DIR") is writable, then run the installer again"
  if [ "${CLI_CURRENT:-0}" = 1 ]; then
    detail "$(tilde "$BIN") unchanged"
  elif [ "$DRY_RUN" = 1 ]; then
    detail "would install $(tilde "$BIN")"
    [ "$OS" = "macos" ] && detail "would sign it ad hoc unless it carries a valid signature"
  else
    mkdir -p "$BIN_DIR" || return 1
    # Beside the destination, so the final mv is a rename: a running harbard
    # keeps its old file and a hook never sees half a binary.
    STAGE="$(mktemp -d "$BIN_DIR/.harbard-install.XXXXXX")" || return 1
    tar -xzf "$TARBALL" -C "$STAGE" || { STEP_FAIL="the archive could not be unpacked"; return 1; }
    [ -f "$STAGE/harbard" ] || { STEP_FAIL="the archive holds no harbard binary"; return 1; }
    chmod 0755 "$STAGE/harbard"
    if [ "$OS" = "macos" ] && [ -z "${HARBARD_INSTALL_OS:-}" ]; then
      sign_binary "$STAGE/harbard" || { STEP_FAIL="codesign could not sign the binary"; return 1; }
    fi
    sc_ver="$("$STAGE/harbard" --version 2>/dev/null)" || {
      STEP_FAIL="the downloaded binary does not run on this machine ($TARGET)"
      return 1
    }
    mv -f "$STAGE/harbard" "$BIN" || return 1
    rm -rf "${STAGE:?}"
    STAGE=""
    receipt_set bin "$BIN"
    receipt_set version "$VERSION"
    receipt_set cli_sha256 "$EXPECTED"
    CLI_CHANGED=1
    detail "$sc_ver $G_ARROW $(tilde "$BIN")"
  fi
  path_setup
  sc_first="$(PATH="$ORIG_PATH" command -v harbard 2>/dev/null || true)"
  if [ -n "$sc_first" ] && [ "$sc_first" != "$BIN" ]; then
    note "another harbard comes first on your PATH: $(tilde "$sc_first")"
  fi
}

# Put the CLI's directory on PATH for the user's shell, once.
path_setup() {
  case ":$ORIG_PATH:" in
    *":$BIN_DIR:"*) detail "$(tilde "$BIN_DIR") is already on your PATH"; return 0 ;;
  esac
  case "$BIN_DIR" in
    "$HOME"/*) ps_dir="\$HOME${BIN_DIR#"$HOME"}" ;;
    *) ps_dir="$BIN_DIR" ;;
  esac
  ps_shell="$(basename "${SHELL:-sh}")"
  case "$ps_shell" in
    zsh) RC_FILE="${ZDOTDIR:-$HOME}/.zshrc"; PATH_LINE="export PATH=\"$ps_dir:\$PATH\"" ;;
    bash)
      if [ "$OS" = "macos" ]; then RC_FILE="$HOME/.bash_profile"; else RC_FILE="$HOME/.bashrc"; fi
      PATH_LINE="export PATH=\"$ps_dir:\$PATH\""
      ;;
    fish)
      RC_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/harbard.fish"
      PATH_LINE="fish_add_path \"$BIN_DIR\""
      ;;
    *) RC_FILE="$HOME/.profile"; PATH_LINE="export PATH=\"$ps_dir:\$PATH\"" ;;
  esac
  if [ "$MODIFY_PATH" = 0 ]; then
    PATH_TODO="$PATH_LINE"
    note "$(tilde "$BIN_DIR") is not on your PATH; add this line to $(tilde "$RC_FILE"):"
    note "  $PATH_LINE"
    return 0
  fi
  if grep -Fqx "$PATH_MARK" "$RC_FILE" 2>/dev/null; then
    detail "PATH already set in $(tilde "$RC_FILE")"
    PATH_TODO="$PATH_LINE"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then
    detail "would add $(tilde "$BIN_DIR") to PATH in $(tilde "$RC_FILE")"
    return 0
  fi
  mkdir -p "$(dirname "$RC_FILE")"
  printf '\n%s\n%s\n' "$PATH_MARK" "$PATH_LINE" >>"$RC_FILE" || {
    note "could not write $(tilde "$RC_FILE"); add this line yourself: $PATH_LINE"
    PATH_TODO="$PATH_LINE"
    return 0
  }
  receipt_set rc_file "$RC_FILE"
  PATH_TODO="$PATH_LINE"
  detail "added $(tilde "$BIN_DIR") to PATH in $(tilde "$RC_FILE")"
}

step_hooks() {
  HOOKS_WIRED=""
  HOOKS_FOUND=""
  HOOKS_ABSENT=""
  for sh_t in claude codex cursor; do
    sh_seen="$(harness_seen "$sh_t")"
    if [ -n "$sh_seen" ]; then
      HOOKS_FOUND="$HOOKS_FOUND $sh_t"
    else
      HOOKS_ABSENT="$HOOKS_ABSENT $sh_t"
    fi
  done
  if [ -z "$HOOKS_FOUND" ]; then
    STEP_STATUS="warn"
    note "no Claude Code, Codex or Cursor found on this machine"
    note "install one, then run: harbard setup"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then
    for sh_t in $HOOKS_FOUND; do
      detail "$(printf '%-12s' "$(harness_name "$sh_t")") found ($(harness_seen "$sh_t")); would wire $(tilde "$(harness_config "$sh_t")")"
    done
    for sh_t in $HOOKS_ABSENT; do
      detail "$(printf '%-12s' "$(harness_name "$sh_t")") not found"
    done
    return 0
  fi
  STEP_HINT="run \`harbard setup\` to see what it could not wire, then \`harbard doctor\`"
  printf '$ harbard setup\n'
  sh_out="$("$BIN" setup 2>&1)"
  sh_rc=$?
  printf '%s\n' "$sh_out"
  [ "$sh_rc" -eq 0 ] || { STEP_FAIL="harbard setup failed: $(printf '%s' "$sh_out" | tail -n 1)"; return 1; }
  for sh_t in claude codex cursor; do
    sh_act="$(printf '%s\n' "$sh_out" | sed -n "s/^  $sh_t  *\([a-z_]*\).*/\1/p" | head -n 1)"
    sh_name="$(printf '%-12s' "$(harness_name "$sh_t")")"
    case "$sh_act" in
      installed)
        HOOKS_WIRED="$HOOKS_WIRED $sh_t"
        detail "$sh_name wired $G_ARROW $(tilde "$(harness_config "$sh_t")")"
        ;;
      unchanged)
        HOOKS_WIRED="$HOOKS_WIRED $sh_t"
        detail "$sh_name already wired ($(tilde "$(harness_config "$sh_t")"))"
        ;;
      absent) detail "$sh_name not found" ;;
      *) note "$sh_name: $sh_act" ;;
    esac
  done
  receipt_set hooks "${HOOKS_WIRED# }"
}

step_prereqs() {
  STEP_HINT="install the missing tool by hand, then run the installer again"
  if [ -z "$MISSING" ]; then
    detail "Rust $(cargo --version 2>/dev/null | cut -d' ' -f2) already installed"
    return 0
  fi
  for spq_tool in $MISSING; do
    case "$spq_tool" in
      rust)
        if [ "$DRY_RUN" = 1 ]; then
          detail "would install Rust with rustup (https://sh.rustup.rs), without editing your shell profile"
          continue
        fi
        fetch https://sh.rustup.rs "$WORK/rustup-init.sh" || { STEP_FAIL="could not download rustup"; return 1; }
        sh "$WORK/rustup-init.sh" -y --no-modify-path --profile minimal || { STEP_FAIL="rustup failed"; return 1; }
        PATH="$HOME/.cargo/bin:$PATH"
        export PATH
        receipt_set installed_rust 1
        detail "Rust $(cargo --version 2>/dev/null | cut -d' ' -f2) $G_ARROW ~/.cargo (rustup)"
        ;;
    esac
  done
}

step_source() {
  SOURCE_CHANGED=0
  case "$VERSION" in v*) ss_ref="$VERSION" ;; *) ss_ref="main" ;; esac
  STEP_HINT="check $(tilde "$SOURCE_DIR"), then run the installer again"
  if [ -d "$SOURCE_DIR/.git" ]; then
    if [ "$DRY_RUN" = 1 ]; then
      detail "would fetch $GIT_URL and move $(tilde "$SOURCE_DIR") to $ss_ref"
    else
      git -C "$SOURCE_DIR" fetch --quiet --tags origin main || { STEP_FAIL="git could not fetch $GIT_URL"; return 1; }
      if [ "$(git -C "$SOURCE_DIR" symbolic-ref --short -q HEAD)" != "main" ]; then
        STEP_FAIL="$(tilde "$SOURCE_DIR") is not on main"
        STEP_HINT="run: git -C \"$SOURCE_DIR\" checkout main, then the installer again"
        return 1
      fi
      if [ -n "$(git -C "$SOURCE_DIR" status --porcelain --untracked-files=no)" ]; then
        STEP_FAIL="$(tilde "$SOURCE_DIR") has local changes"
        STEP_HINT="discard them (git -C \"$SOURCE_DIR\" checkout -- .), then run the installer again"
        return 1
      fi
      ss_old="$(git -C "$SOURCE_DIR" rev-parse HEAD)"
      ss_new="$(git -C "$SOURCE_DIR" rev-parse --verify --quiet "$ss_ref^{commit}")" ||
        { STEP_FAIL="$ss_ref is not in $GIT_URL"; return 1; }
      if [ "$ss_old" = "$ss_new" ]; then
        detail "$(tilde "$SOURCE_DIR") already at $ss_ref"
      elif git -C "$SOURCE_DIR" merge-base --is-ancestor "$ss_old" "$ss_new"; then
        git -C "$SOURCE_DIR" merge --ff-only --quiet "$ss_new" || return 1
        SOURCE_CHANGED=1
        detail "updated $(tilde "$SOURCE_DIR") to $ss_ref"
      else
        detail "$(tilde "$SOURCE_DIR") is newer than $ss_ref; left as it is"
      fi
    fi
  else
    if [ "$DRY_RUN" = 1 ]; then
      detail "would clone $GIT_URL into $(tilde "$SOURCE_DIR") at $ss_ref"
    else
      mkdir -p "$(dirname "$SOURCE_DIR")" || return 1
      git clone --quiet "$GIT_URL" "$SOURCE_DIR" || { STEP_FAIL="git could not clone $GIT_URL"; return 1; }
      # `main` at the release: `harbard update` fast-forwards main to newer tags.
      git -C "$SOURCE_DIR" checkout --quiet -B main "$ss_ref" || { STEP_FAIL="$ss_ref is not in $GIT_URL"; return 1; }
      git -C "$SOURCE_DIR" branch --quiet --set-upstream-to=origin/main main || true
      SOURCE_CHANGED=1
      detail "cloned $GIT_URL $G_ARROW $(tilde "$SOURCE_DIR") ($ss_ref)"
    fi
  fi
  receipt_set source_dir "$SOURCE_DIR"
  if [ "$DRY_RUN" = 1 ]; then
    detail "would point \`harbard update\` at it (releases channel, CLI in $(tilde "$BIN_DIR"))"
    return 0
  fi
  SOURCE_COMMIT="$(git -C "$SOURCE_DIR" rev-parse HEAD)"
  printf '$ harbard update --source ... --install-dir ... --channel releases --check\n'
  "$BIN" update --source "$SOURCE_DIR" --install-dir "$BIN_DIR" --channel releases --check --quiet
  if [ $? -eq 2 ]; then
    STEP_FAIL="harbard update did not accept $(tilde "$SOURCE_DIR") as its source"
    return 1
  fi
  detail "\`harbard update\` now updates from it (releases channel)"
}

step_services() {
  STEP_HINT="read the log above; the services' own logs are in ~/.harbard/logs"
  ss_retired="$AGENT_DIR/$RETIRED_LABEL.plist"
  if [ "$DRY_RUN" = 1 ]; then
    detail "would write and start 2 LaunchAgents in ~/Library/LaunchAgents:"
    detail "  com.harbard.serve     the bus API on 127.0.0.1:7419"
    detail "  com.harbard.watchdog  restarts serve if it stops answering"
    [ -f "$ss_retired" ] && detail "would stop and delete $RETIRED_LABEL: the app no longer needs the web console"
    return 0
  fi
  ss_all=1
  for ss_l in $AGENT_LABELS; do [ -f "$AGENT_DIR/$ss_l.plist" ] || ss_all=0; done
  [ -f "$ss_retired" ] && ss_all=0
  if [ "$ss_all" = 1 ] && [ "${CLI_CHANGED:-0}" = 0 ] && [ "${SOURCE_CHANGED:-0}" = 0 ]; then
    STEP_STATUS="skip"
    detail "running from $(tilde "$SOURCE_DIR"); nothing changed"
    return 0
  fi
  bash "$SOURCE_DIR/scripts/install-launchagents.sh" || { STEP_FAIL="the services did not start"; return 1; }
  for ss_l in $AGENT_LABELS; do receipt_add agent "$AGENT_DIR/$ss_l.plist"; done
  receipt_drop agent "$ss_retired"
  detail "serve 127.0.0.1:7419 $G_DOT watchdog"
  detail "they start at login; logs in ~/.harbard/logs"
}

# Where apps/desktop/scripts/install.sh puts the app.
app_paths() {
  if [ -w /Applications ]; then
    printf '%s\n%s\n' "/Applications/harbard.app" "$HOME/Applications/harbard.app"
  else
    printf '%s\n' "$HOME/Applications/harbard.app"
  fi
}

step_app() {
  APP_PATH="$(app_paths | head -n 1)"
  STEP_HINT="read the log above, then run the installer again; the previous app, if any, was kept"
  if [ "$DRY_RUN" = 1 ]; then
    detail "would build the desktop app (the first build takes several minutes)"
    detail "would install $(tilde "$APP_PATH")"
    return 0
  fi
  if [ -d "$APP_PATH" ] && [ "$(receipt_get app_commit)" = "$SOURCE_COMMIT" ]; then
    STEP_STATUS="skip"
    detail "$(tilde "$APP_PATH") already built from $VERSION"
    return 0
  fi
  HARBARD_QUIT_APP=1 HARBARD_OPEN_APP=0 bash "$SOURCE_DIR/scripts/update-desktop.sh" ||
    { STEP_FAIL="the desktop app did not build or install"; return 1; }
  app_paths | while IFS= read -r sa_p; do
    [ -d "$sa_p" ] && receipt_add app "$sa_p"
  done
  receipt_set app_commit "$SOURCE_COMMIT"
  detail "$(tilde "$APP_PATH")"
}

step_doctor() {
  if [ "$DRY_RUN" = 1 ]; then
    detail "would run: harbard doctor"
    return 0
  fi
  printf '$ harbard doctor\n'
  sd_out="$("$BIN" doctor 2>&1)"
  sd_rc=$?
  printf '%s\n' "$sd_out"
  sd_serve="$(printf '%s\n' "$sd_out" | sed -n 's/^serve  *//p' | head -n 1)"
  if [ "$sd_rc" -eq 0 ]; then
    detail "harbard doctor: all checks pass"
  else
    STEP_STATUS="warn"
    note "harbard doctor found something to look at; run it to read the report"
  fi
  [ -n "$sd_serve" ] && detail "serve: $(printf '%s' "$sd_serve" | cut -c 1-90)"
  return 0
}

summary() {
  blank
  if [ "$DRY_RUN" = 1 ]; then
    say "  ${C_BOLD}Dry run complete.${C_RESET} Nothing was changed. Run without --dry-run to install."
    blank
    return 0
  fi
  say "  ${C_GREEN}${C_BOLD}harbard ${VERSION} is installed.${C_RESET}"
  blank
  row() { say "  ${C_DIM}${G_BAR}${C_RESET}  $(printf '%-11s' "$1") $2"; }
  row "CLI" "$(tilde "$BIN")"
  su_wired=""
  for su_t in ${HOOKS_WIRED:-}; do su_wired="${su_wired:+$su_wired, }$(harness_name "$su_t")"; done
  su_absent=""
  for su_t in ${HOOKS_ABSENT:-}; do su_absent="${su_absent:+$su_absent, }$(harness_name "$su_t")"; done
  row "Hooks" "${su_wired:-none wired}${su_absent:+  ${C_DIM}(not found: $su_absent)${C_RESET}}"
  if [ "$APP" = 1 ]; then
    row "Services" "serve, watchdog  ${C_DIM}(LaunchAgents)${C_RESET}"
    row "App" "$(tilde "$APP_PATH")"
    row "Source" "$(tilde "$SOURCE_DIR")"
  elif [ "$OS" = "macos" ]; then
    row "App" "${C_DIM}not installed; run the installer again without --no-app to add it${C_RESET}"
  else
    row "App" "${C_DIM}the desktop app is macOS-only for now${C_RESET}"
  fi
  row "Data" "$(tilde "$DATA_DIR")"
  row "Log" "$(tilde "$LOG")"
  blank
  if [ "$APP" = 1 ]; then
    row "Open" "harbard from Applications or Spotlight  ${C_DIM}(open -a harbard)${C_RESET}"
    row "Update" "harbard update"
  else
    row "Update" "run this installer again"
  fi
  row "Check" "harbard doctor"
  row "Uninstall" "curl -fsSL $INSTALL_URL | sh -s -- --uninstall"
  if [ -n "${PATH_TODO:-}" ] && ! printf '%s' ":$ORIG_PATH:" | grep -Fq ":$BIN_DIR:"; then
    blank
    say "  ${C_YELLOW}${G_WARN}${C_RESET} Open a new terminal to use \`harbard\`, or run now:"
    say "      $PATH_TODO"
    say "    Agents started before this install hear the bus once restarted from a new terminal:"
    say "    their hooks find \`harbard\` on PATH."
  fi
  blank
}

install_main() {
  CLI_CURRENT=0
  CLI_CHANGED=0
  SOURCE_CHANGED=0
  SOURCE_COMMIT=""
  APP_PATH=""
  PATH_TODO=""
  HOOKS_WIRED=""
  HOOKS_ABSENT=""
  resolve_version
  MISSING=""
  FOREIGN_SERVICES=""
  if [ "$APP" = 1 ]; then
    check_app_prereqs
    check_service_owner
  fi
  header
  if [ "$APP" = 1 ] && [ -n "$FOREIGN_SERVICES" ]; then
    die "harbard's services on this Mac already run from $(tilde "$FOREIGN_SERVICES")." \
      "That setup is updated from its own checkout (harbard update), so the installer leaves it alone." \
      "To install only the CLI and hooks here: sh -s -- --no-app"
  fi
  if [ "$APP" = 1 ] && [ "$CLT_OK" = 0 ]; then
    if [ "$DRY_RUN" = 0 ] && [ "$INTERACTIVE" = 1 ]; then xcode-select --install >/dev/null 2>&1 || true; fi
    die "The desktop app needs Apple's Command Line Tools, which are not installed." \
      "Install them with: xcode-select --install   (a dialog opens; follow it)" \
      "Then run this installer again. Or install without the app: sh -s -- --no-app"
  fi
  confirm_prereqs
  STEP_TOTAL=4
  [ "$APP" = 1 ] && STEP_TOTAL=8
  blank
  step "Download and verify harbard $VERSION" step_download
  step "Install the CLI" step_cli
  step "Connect your coding agents" step_hooks
  if [ "$APP" = 1 ]; then
    step "Check the build tools" step_prereqs
    step "Fetch the source the app is built from" step_source
    step "Start the background services" step_services
    step "Build and install the desktop app" step_app
  fi
  step "Check the installation" step_doctor
  if [ "$APP" = 1 ] && [ "$DRY_RUN" = 0 ] && [ -n "$APP_PATH" ] && [ -d "$APP_PATH" ]; then
    open "$APP_PATH" 2>/dev/null || true
  fi
  summary
}

# ---------------------------------------------------------------------------
# Uninstall
# ---------------------------------------------------------------------------

step_unhook() {
  uh_bin="$(receipt_get bin)"
  [ -x "${uh_bin:-}" ] || uh_bin="$BIN"
  [ -x "$uh_bin" ] || uh_bin="$(command -v harbard 2>/dev/null || true)"
  if [ -z "$uh_bin" ]; then
    STEP_STATUS="warn"
    UN_KEPT=1
    note "no harbard binary to remove the hooks with; they were left in place"
    return 0
  fi
  uh_any=0
  for uh_t in claude codex cursor; do
    uh_cfg="$(harness_config "$uh_t")"
    [ -f "$uh_cfg" ] || continue
    grep -q harbard "$uh_cfg" 2>/dev/null || continue
    uh_any=1
    if [ "$DRY_RUN" = 1 ]; then
      detail "would remove harbard's hooks from $(tilde "$uh_cfg"); other hooks stay"
      continue
    fi
    printf '$ harbard install %s --uninstall\n' "$uh_t"
    "$uh_bin" install "$uh_t" --uninstall || { STEP_FAIL="could not remove the $(harness_name "$uh_t") hooks"; return 1; }
    detail "$(printf '%-12s' "$(harness_name "$uh_t")") hooks removed from $(tilde "$uh_cfg")"
  done
  receipt_drop hooks
  if [ "$uh_any" = 0 ]; then
    STEP_STATUS="skip"
    detail "no harbard hooks found"
  fi
}

step_unservice() {
  receipt_all agent >"$WORK/agents"
  if [ ! -s "$WORK/agents" ]; then
    STEP_STATUS="skip"
    detail "the installer started none"
    return 0
  fi
  us_uid="$(id -u)"
  while IFS= read -r us_plist; do
    us_label="$(basename "$us_plist" .plist)"
    case "$us_label" in com.harbard.*) ;; *) continue ;; esac
    if [ "$DRY_RUN" = 1 ]; then
      [ -f "$us_plist" ] && detail "would stop $us_label and delete $(tilde "$us_plist")"
      continue
    fi
    if [ -f "$us_plist" ]; then
      # Only the job this very plist defines, never another of the same name.
      if launchctl print "gui/$us_uid/$us_label" 2>/dev/null | grep -Fq "path = $us_plist"; then
        launchctl bootout "gui/$us_uid" "$us_plist" 2>/dev/null || true
      fi
      rm -f "$us_plist"
      detail "stopped $us_label"
    fi
    receipt_drop agent "$us_plist"
  done <"$WORK/agents"
}

step_unapp() {
  receipt_all app >"$WORK/apps"
  if [ ! -s "$WORK/apps" ]; then
    STEP_STATUS="skip"
    detail "the installer installed none"
    return 0
  fi
  ua_ls="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
  while IFS= read -r ua_p; do
    case "$ua_p" in */harbard.app) ;; *) continue ;; esac
    if [ "$DRY_RUN" = 1 ]; then
      [ -d "$ua_p" ] && detail "would quit and delete $(tilde "$ua_p")"
      continue
    fi
    [ -d "$ua_p" ] || { receipt_drop app "$ua_p"; continue; }
    # Quit only the copy being deleted, the way the user would (Apple Events, ⌘Q).
    # shellcheck disable=SC2009 # by its full path, as update-desktop.sh matches it
    if ps -axo comm= 2>/dev/null | sed 's/^[[:space:]]*//' | grep -Fqx "$ua_p/Contents/MacOS/harbard-desktop"; then
      osascript -e 'tell application id "com.harbard.desktop" to quit' >/dev/null 2>&1 || true
      sleep 2
    fi
    [ -x "$ua_ls" ] && "$ua_ls" -u "$ua_p" >/dev/null 2>&1
    rm -rf "${ua_p:?}"
    for ua_b in "$(dirname "$ua_p")"/.harbard-backup.*; do
      [ -d "$ua_b" ] || continue
      [ -x "$ua_ls" ] && "$ua_ls" -u "$ua_b/harbard.app" >/dev/null 2>&1
      rm -rf "${ua_b:?}"
    done
    detail "deleted $(tilde "$ua_p")"
    receipt_drop app "$ua_p"
  done <"$WORK/apps"
  receipt_drop app_commit
}

step_unsource() {
  uz_dir="$(receipt_get source_dir)"
  if [ -z "$uz_dir" ] || [ ! -d "$uz_dir" ]; then
    STEP_STATUS="skip"
    detail "the installer cloned none"
    return 0
  fi
  if [ ! -d "$uz_dir/.git" ] || [ ! -f "$uz_dir/Cargo.toml" ]; then
    STEP_STATUS="warn"
    UN_KEPT=1
    note "$(tilde "$uz_dir") does not look like the checkout the installer made; left in place"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then
    detail "would delete $(tilde "$uz_dir")"
    return 0
  fi
  rm -rf "${uz_dir:?}"
  rmdir "$SUPPORT_DIR" 2>/dev/null || true
  receipt_drop source_dir
  detail "deleted $(tilde "$uz_dir")"
}

step_uncli() {
  uc_bin="$(receipt_get bin)"
  [ -n "$uc_bin" ] || uc_bin="$BIN"
  if [ -f "$uc_bin" ]; then
    if [ "$DRY_RUN" = 1 ]; then
      detail "would delete $(tilde "$uc_bin")"
    else
      rm -f "$uc_bin"
      receipt_drop bin
      receipt_drop cli_sha256
      receipt_drop version
      detail "deleted $(tilde "$uc_bin")"
    fi
  else
    detail "no CLI at $(tilde "$uc_bin")"
  fi
  uc_rc="$(receipt_get rc_file)"
  for uc_f in "$uc_rc" "${ZDOTDIR:-$HOME}/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.profile"; do
    [ -n "$uc_f" ] && [ -f "$uc_f" ] || continue
    case "$uc_f" in */fish/conf.d/harbard.fish) continue ;; esac # the installer's own file: deleted below
    grep -Fqx "$PATH_MARK" "$uc_f" || continue
    if [ "$DRY_RUN" = 1 ]; then
      detail "would remove the PATH line from $(tilde "$uc_f")"
      continue
    fi
    awk -v m="$PATH_MARK" '
      skip { skip = 0; next }
      $0 == m { skip = 1; held = 0; next }
      /^$/ { if (held) print ""; held = 1; next }
      { if (held) print ""; held = 0; print }
      END { if (held) print "" }' "$uc_f" >"$uc_f.harbard-tmp" && cat "$uc_f.harbard-tmp" >"$uc_f"
    rm -f "$uc_f.harbard-tmp"
    detail "removed the PATH line from $(tilde "$uc_f")"
  done
  uc_fish="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/harbard.fish"
  if [ -f "$uc_fish" ] && grep -Fqx "$PATH_MARK" "$uc_fish"; then
    if [ "$DRY_RUN" = 1 ]; then detail "would delete $(tilde "$uc_fish")"; else rm -f "$uc_fish"; detail "deleted $(tilde "$uc_fish")"; fi
  fi
  receipt_drop rc_file
}

step_purge() {
  if [ "$PURGE" = 0 ]; then
    STEP_STATUS="skip"
    detail "kept $(tilde "$DATA_DIR") (add --purge to delete it)"
    return 0
  fi
  case "$DATA_DIR" in
    "" | / | "$HOME" | "$HOME/") STEP_FAIL="refusing to delete $DATA_DIR"; return 1 ;;
  esac
  if [ ! -d "$DATA_DIR" ]; then
    detail "no data at $(tilde "$DATA_DIR")"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then
    detail "would delete $(tilde "$DATA_DIR") and everything in it"
    return 0
  fi
  rm -rf "${DATA_DIR:?}"
  detail "deleted $(tilde "$DATA_DIR")"
}

uninstall_main() {
  UN_KEPT=0
  header
  if [ "$PURGE" = 1 ] && [ "$DRY_RUN" = 0 ] && [ "$YES" = 0 ] && [ "$INTERACTIVE" = 1 ]; then
    blank
    printf '  --purge deletes %s: every note, presence and setting. Continue? [y/N] ' "$(tilde "$DATA_DIR")" >&3
    um_answer=""
    read -r um_answer </dev/tty || um_answer=""
    case "$um_answer" in y | Y | yes | YES) ;; *) say "  Nothing was removed."; exit 1 ;; esac
  fi
  if [ "$OS" = "macos" ]; then STEP_TOTAL=6; else STEP_TOTAL=3; fi
  blank
  step "Remove the hooks" step_unhook
  if [ "$OS" = "macos" ]; then
    step "Stop and remove the background services" step_unservice
    step "Remove the desktop app" step_unapp
    step "Remove the source checkout" step_unsource
  fi
  step "Remove the CLI" step_uncli
  step "Remove harbard's data" step_purge
  # Older installs also added bun, for the web console the app used to show.
  UN_RUST="$(receipt_get installed_rust)"
  UN_BUN="$(receipt_get installed_bun)"
  # Everything it listed is gone; a step that left something keeps the receipt.
  if [ "$DRY_RUN" = 0 ] && [ "$UN_KEPT" = 0 ]; then rm -f "$RECEIPT"; fi
  blank
  if [ "$DRY_RUN" = 1 ]; then
    say "  ${C_BOLD}Dry run complete.${C_RESET} Nothing was changed."
  else
    say "  ${C_GREEN}${C_BOLD}harbard is uninstalled.${C_RESET}"
    if [ "$UN_RUST" = 1 ] && [ "$UN_BUN" = 1 ]; then
      say "  ${C_DIM}Rust and bun, which the installer added for the app, stay; remove them with${C_RESET}"
      say "  ${C_DIM}\`rustup self uninstall\` and by deleting ~/.bun.${C_RESET}"
    elif [ "$UN_RUST" = 1 ]; then
      say "  ${C_DIM}Rust, which the installer added for the app, stays; remove it with \`rustup self uninstall\`.${C_RESET}"
    elif [ "$UN_BUN" = 1 ]; then
      say "  ${C_DIM}bun, which an older installer added, stays; remove it by deleting ~/.bun.${C_RESET}"
    fi
  fi
  blank
}

# ---------------------------------------------------------------------------

main() {
  exec 3>&1
  parse_args "$@"
  ui_init
  trap on_exit EXIT
  trap on_signal INT TERM
  detect_platform
  if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
    die "This installer needs curl or wget." "Install one with your package manager, then run it again."
  fi
  need tar "Install it with your package manager."
  resolve_paths
  WORK="$(mktemp -d "${TMPDIR:-/tmp}/harbard-install.XXXXXX")" || die "Could not create a temporary directory."
  STAGE=""
  STEP_N=0
  STEP_TOTAL=0
  LOG_KEEP=0
  if [ "$DRY_RUN" = 1 ] || [ "$UNINSTALL" = 1 ]; then
    LOG="$WORK/run.log"
  else
    mkdir -p "$DATA_DIR/logs" || die "Could not create $(tilde "$DATA_DIR")/logs."
    LOG="$DATA_DIR/logs/install-$(date +%Y%m%d-%H%M%S).log"
    LOG_KEEP=1
  fi
  : >"$LOG"
  if [ "$UNINSTALL" = 1 ]; then
    uninstall_main
  else
    install_main
  fi
}

# scripts/test-install.sh loads the functions without running them.
[ -n "${HARBARD_INSTALL_SOURCE_ONLY:-}" ] || main "$@"
