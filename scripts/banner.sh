#!/usr/bin/env bash
# FutureOS ASCII Banner — preview all styles with colors
# Usage: bash scripts/banner.sh [style-name]
#        bash scripts/banner.sh          # show all
#        bash scripts/banner.sh b        # show only style B

set -e

L1='  ███████╗██╗   ██╗████████╗██╗   ██╗██████╗ ███████╗     ██████╗ ███████╗'
L2='  ██╔════╝██║   ██║╚══██╔══╝██║   ██║██╔══██╗██╔════╝    ██╔═══██╗██╔════╝'
L3='  █████╗  ██║   ██║   ██║   ██║   ██║██████╔╝█████╗      ██║   ██║███████╗'
L4='  ██╔══╝  ██║   ██║   ██║   ██║   ██║██╔══██╗██╔══╝      ██║   ██║╚════██║'
L5='  ██║     ╚██████╔╝   ██║   ╚██████╔╝██║  ██║███████╗    ╚██████╔╝███████║'
L6='  ╚═╝      ╚═════╝    ╚═╝    ╚═════╝ ╚═╝  ╚═╝╚══════╝     ╚═════╝ ╚══════╝'

cyan()  { printf '\033[38;5;%dm' "$1"; }
bold()  { printf '\033[1m'; }
dim()   { printf '\033[2m'; }
reset() { printf '\033[0m'; }
hr()    { reset; echo; echo "──────"; echo; }
header(){ echo; echo "  $1"; echo; }

# ─── Style A: Cyan gradient ────────────────────────────────────────────────

style_a() {
  header "Style A — Cyan gradient"
  cyan 51; echo "$L1"; cyan 45; echo "$L2"; cyan 39; echo "$L3"
  cyan 33; echo "$L4"; cyan 27; echo "$L5"; cyan 21; echo "$L6"
  reset
}

# ─── Style B: Orange-Yellow warm gradient ───────────────────────────────────

style_b() {
  header "Style B — Orange-Yellow warm gradient"
  printf '\033[38;5;208m'; echo "$L1"
  printf '\033[38;5;214m'; echo "$L2"
  printf '\033[38;5;220m'; echo "$L3"
  printf '\033[38;5;226m'; echo "$L4"
  printf '\033[38;5;190m'; echo "$L5"
  printf '\033[38;5;154m'; echo "$L6"
  reset
}

# ─── Style C: Cyan bold ─────────────────────────────────────────────────────

style_c() {
  header "Style C — Cyan bold"
  bold; cyan 51
  echo "$L1"; echo "$L2"; echo "$L3"; echo "$L4"; echo "$L5"; echo "$L6"
  reset
}

# ─── Style D: Magenta bold ──────────────────────────────────────────────────

style_d() {
  header "Style D — Magenta bold"
  bold; printf '\033[1;35m'
  echo "$L1"; echo "$L2"; echo "$L3"
  bold; printf '\033[1;36m'
  echo "$L4"; echo "$L5"; echo "$L6"
  reset
}

# ─── Style E: Matrix green ──────────────────────────────────────────────────

style_e() {
  header "Style E — Matrix Green"
  printf '\033[38;5;46m'; echo "$L1"
  printf '\033[38;5;40m'; echo "$L2"
  printf '\033[38;5;34m'; echo "$L3"
  printf '\033[38;5;28m'; echo "$L4"
  printf '\033[38;5;22m'; echo "$L5"
  dim; echo "$L6"
  reset
}

# ─── Style F: Red gradient ──────────────────────────────────────────────────

style_f() {
  header "Style F — Red gradient"
  printf '\033[38;5;208m'; echo "$L1"
  printf '\033[38;5;202m'; echo "$L2"
  printf '\033[38;5;196m'; echo "$L3"
  printf '\033[38;5;160m'; echo "$L4"
  printf '\033[38;5;124m'; echo "$L5"
  printf '\033[38;5;88m';  echo "$L6"
  reset
}

# ─── Main ───────────────────────────────────────────────────────────────────

STYLES=(a b c d e f)

show_all() {
  for s in "${STYLES[@]}"; do
    "style_$s"; hr
  done
}

[[ $# -eq 0 ]] && show_all || "style_$1"
