#!/usr/bin/env bash
# Install CI-only Ubuntu packages without letting the Azure runner mirror hold
# a job hostage.
#
# GitHub's Ubuntu images keep the mirror hostname out of the apt sources they
# ship. The 24.04 image writes `URIs: mirror+file:/etc/apt/apt-mirrors.txt` into
# /etc/apt/sources.list.d/ubuntu.sources, and the hostnames live in that mirror
# list in priority order (azure.archive.ubuntu.com first, Ubuntu's public
# archive second). So editing a hostname in the sources changes nothing about
# where packages actually come from - the mirror list is the file that has to
# move. That was the defect behind run 39822317986: the retry announced a switch
# to the public archive, then fetched from azure.archive.ubuntu.com again at
# ~50 kB/s and burned a second 180 s budget on the same endpoint. The old
# `sed` found no matching hostname, changed no file, and did not say so.
#
# A transfer that crawls is also not something apt can fail over from: only a
# broken one is. So downloads get a budget sized to the bytes on the wire, and
# every attempt prints the archive host it is about to use.
set -Eeuo pipefail

# Where apt's sources are read from. Only the self-test changes this; it points
# the rewrite logic at a fixture tree so it can run without root or a real apt.
apt_etc_dir=${APT_ETC_DIR:-/etc/apt}

if (( $# == 0 )); then
  echo "usage: $0 <apt-package> [...] | --self-test" >&2
  exit 2
fi

# Metadata fetches are bounded tightly: an index that will not arrive is worth
# abandoning in seconds, not minutes.
update_budget=45

# Unpacking a few hundred MB of GUI libraries takes well under a minute on a
# healthy runner, so this only bounds a dpkg that has stopped moving.
unpack_budget=240

# Do not let one unavailable mirror consume the job's entire setup budget. APT
# otherwise retries individual index downloads for minutes before surfacing an
# error. The retry count is deliberately zero for indexes: the second attempt is
# made against the official archive below, rather than the same unhealthy
# endpoint. Transfers keep a few retries, since an aborted connection is cheap
# to resume while a stalled one is not (http::Timeout bounds a read, not the
# whole transfer).
apt_opts=(
  -o Acquire::Retries=0
  -o Acquire::http::Timeout=15
  -o Acquire::https::Timeout=15
  -o Dpkg::Use-Pty=0
)

as_root() {
  # The self-test decodes and rewrites its own fixture tree, so it never needs
  # (or may assume) root.
  if [[ $apt_etc_dir == /etc/apt ]]; then
    sudo "$@"
  else
    "$@"
  fi
}

# Every file that could name the archive endpoint: the classic lists, the deb822
# files in sources.list.d, and the file a `mirror+file:` source defers to. That
# last one is the point - on 24.04 it holds the only hostname on the system.
source_files() {
  local file
  echo "$apt_etc_dir/sources.list"
  for file in "$apt_etc_dir"/sources.list.d/*.list "$apt_etc_dir"/sources.list.d/*.sources; do
    if [[ -e $file ]]; then echo "$file"; fi
  done
  grep -hoE '(mirror\+)?file:[^[:space:]]+' \
    "$apt_etc_dir/sources.list" "$apt_etc_dir"/sources.list.d/* 2>/dev/null |
    sed -E 's/^(mirror\+)?file://' || true
  [[ -e $apt_etc_dir/apt-mirrors.txt ]] && echo "$apt_etc_dir/apt-mirrors.txt"
  return 0
}

# Repoint the runner's Azure mirror at Ubuntu's public archive. Ubuntu 24.04
# runners use deb822 (*.sources) pointing at a mirror list; older images may
# write the hostname into classic *.list files. Suites, components, priorities
# and any third-party source configuration are preserved - only the hostname
# changes.
switch_to_public_archive() {
  local file changed=0
  while IFS= read -r file; do
    if [[ ! -f $file ]]; then continue; fi
    if ! grep -qE 'azure\.(archive|ports)\.ubuntu\.com' "$file"; then continue; fi
    as_root sed -i \
      -e 's|azure\.archive\.ubuntu\.com|archive.ubuntu.com|g' \
      -e 's|azure\.ports\.ubuntu\.com|ports\.ubuntu.com|g' \
      "$file"
    echo "APT mirror: rewrote ${file} (Azure mirror -> Ubuntu public archive)" >&2
    changed=1
  done < <(source_files)
  if (( changed == 0 )); then
    # Never announce a switch that did not happen: a silent no-op here is what
    # made the previous version of this script useless.
    echo "APT mirror: no apt source or mirror list names the Azure mirror; nothing was rewritten" >&2
    return 1
  fi
  return 0
}

# The mirror list a `mirror+file:` source defers to, as configured.
configured_mirror_lists() {
  grep -hoE 'mirror\+file:[^[:space:]]+' \
    "$apt_etc_dir/sources.list" "$apt_etc_dir"/sources.list.d/* 2>/dev/null |
    sed -E 's/^mirror\+file://' | sort -u || true
}

# Entries apt can use, for the log line that says how much failover is left.
mirror_list_entries() {
  awk 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "$1"
}

# The mirror apt will actually use: lowest priority number wins and the rest
# are failover. Entries without a priority keep their file order.
mirror_list_pick() {
  awk -F'\t' '
    {
      url = $1
      sub(/[[:space:]]+$/, "", url)
      if (url == "" || url ~ /^#/) { next }
      prio = 1000000
      if ($2 ~ /priority:/) { p = $2; sub(/.*priority:[[:space:]]*/, "", p); prio = p + 0 }
      if (pick == "" || prio < best) { best = prio; pick = url }
    }
    END { print pick }' "$1"
}

# `--print-uris` does not resolve the mirror list: for a source that defers to
# one it prints `mirror+file:/etc/apt/apt-mirrors.txt/pool/...`, so the endpoint
# has to be substituted here or the log names no archive. This is also the only
# honest before/after check of a switch, since that pseudo-URI looks identical
# on both sides of one.
resolve_mirror_uris() {
  local list base line uri rest
  list=$(configured_mirror_lists | sed -n '1p')
  base=""
  if [[ -n $list && -f $list ]]; then base=$(mirror_list_pick "$list"); fi
  while IFS= read -r line; do
    uri=$(example_uri "$line")
    case $uri in
      mirror+file:*)
        if [[ -n $base ]]; then
          rest=${uri#mirror+file:}
          rest=${rest#"$list"}
          uri="${base%/}${rest}"
        fi
        ;;
    esac
    printf "'%s' %s\n" "$uri" "${line#* }"
  done
}

# The archive URIs apt would fetch, asked of apt rather than of the source
# files, so every layer of indirection is accounted for.
planned_uris() {
  as_root apt-get "${apt_opts[@]}" --print-uris --yes --no-install-recommends \
    install "$@" 2>/dev/null | grep -E "^'" || true
}

# Bytes apt would fetch, straight from --print-uris, so the download budget
# follows the payload instead of a fixed guess.
bytes_of_uris() {
  awk -v quote="'" '
    substr($0, 1, 1) != quote { next }
    {
      size = $3
      if (size ~ /k$/)            { sub(/k$/, "", size); total += size * 1024 }
      else if (size ~ /M$/)       { sub(/M$/, "", size); total += size * 1024 * 1024 }
      else if (size ~ /^[0-9]+$/) { total += size }
    }
    END { print total + 0 }'
}

# Distinct archive hostnames across the planned URIs.
hosts_of_uris() {
  awk -v quote="'" '
    substr($0, 1, 1) == quote {
      split($1, parts, "/")
      if (parts[3] != "" && !seen[parts[3]]++) { joined = joined (joined == "" ? "" : ",") parts[3] }
    }
    END { print joined }'
}

# One example URI, unquoted, so a log reader sees the endpoint itself. Pure
# bash on the already-captured plan: a pipeline that stops after the first line
# would let SIGPIPE fail the caller under `set -o pipefail`.
example_uri() {
  local line=${1%%$'\n'*}
  line=${line%% *}
  line=${line#\'}
  echo "${line%\'}"
}

# Archives still missing from the cache - that is, what a killed download was
# still fetching. Printed on failure so the log names the packages involved
# instead of leaving the reader to guess from a truncated "Get:" line.
pending_packages() {
  planned_uris "$@" | resolve_mirror_uris |
    awk -v quote="'" 'substr($0, 1, 1) == quote { printf "%s ", $2 }'
}

as_mb() { awk -v bytes="$1" 'BEGIN { printf "%.1f", bytes / 1048576 }'; }

# Sized to the payload: the runner mirror is normally good for megabytes a
# second (a sibling job fetched 59 MB in 19 s), so ~4 s/MB is slow enough not to
# cut a healthy transfer short and short enough to fail a crawl within minutes.
download_budget_for() {
  local mb budget
  mb=$(( ($1 + 1048575) / 1048576 ))
  budget=$(( 90 + mb * 4 ))
  if (( budget < 180 )); then budget=180; fi
  if (( budget > 480 )); then budget=480; fi
  echo "$budget"
}

update_index() {
  as_root timeout "${update_budget}s" apt-get "${apt_opts[@]}" update
}

# Fetch first, unpack second. Two budgets instead of one means a failure says
# which half ran out: a download that hits its budget is an endpoint problem
# (-o Acquire::http::Timeout already bounds a transfer that stops moving),
# while a stuck unpack is a dpkg problem and retrying it elsewhere is pointless.
install_packages() {
  local plan hosts sample bytes budget list
  plan=$(planned_uris "$@" | resolve_mirror_uris)
  hosts=$(printf '%s\n' "$plan" | hosts_of_uris)
  sample=$(example_uri "$plan")
  bytes=$(printf '%s\n' "$plan" | bytes_of_uris)
  budget=$(download_budget_for "$bytes")
  last_hosts=${hosts:-no archive host}
  list=$(configured_mirror_lists | sed -n '1p')
  if [[ -n $list && -f $list ]]; then
    echo "APT mirror list ${list}: $(mirror_list_entries "$list") entries, using $(mirror_list_pick "$list")" >&2
  fi
  echo "APT download: $(as_mb "$bytes") MB from ${last_hosts} (budget ${budget}s)" >&2
  if [[ -n $sample ]]; then
    echo "APT sample URI: ${sample}" >&2
  fi
  last_phase=download
  as_root timeout "${budget}s" apt-get "${apt_opts[@]}" -o Acquire::Retries=2 \
    --no-install-recommends install -y --download-only "$@" || return $?
  last_phase=unpack
  as_root timeout "${unpack_budget}s" apt-get "${apt_opts[@]}" \
    --no-install-recommends install -y --no-download "$@"
}

self_test() {
  local status=0 file
  # Deliberately not `local`: the EXIT trap below still has to see it.
  root=$(mktemp -d)
  trap 'rm -rf "$root"' EXIT
  mkdir -p "$root/sources.list.d"
  apt_etc_dir=$root

  # The Ubuntu 24.04 runner image: sources defer to a three-line mirror list
  # (144 bytes, priorities descending) and name no hostname themselves.
  cat > "$root/apt-mirrors.txt" <<'EOF'
http://azure.archive.ubuntu.com/ubuntu/	priority:1
https://archive.ubuntu.com/ubuntu/	priority:2
https://security.ubuntu.com/ubuntu/	priority:3
EOF
  cat > "$root/sources.list.d/ubuntu.sources" <<EOF
Types: deb
URIs: mirror+file:${root}/apt-mirrors.txt
Suites: noble noble-updates
Components: main universe
EOF

  # apt leaves the indirection unresolved in --print-uris, so the reporting
  # path has to substitute the mirror itself. Asserted on both sides of the
  # switch: the plan line is identical, only the resolved endpoint may change.
  plan="'mirror+file:${root}/apt-mirrors.txt/pool/main/f/foo/foo_1.0_amd64.deb' foo_1.0_amd64.deb 1048576 SHA256:abc"
  expect_resolved() {
    local want=$1 got
    got=$(printf '%s\n' "$plan" | resolve_mirror_uris)
    case $got in
      "'${want}"*) ;;
      *) echo "self-test: resolved '${got}', expected '${want}...'" >&2; status=1 ;;
    esac
  }

  if (( $(mirror_list_entries "$root/apt-mirrors.txt") != 3 )); then
    echo "self-test: did not count the three mirror-list entries" >&2
    status=1
  fi
  expect_resolved 'http://azure.archive.ubuntu.com/ubuntu/pool/main/f/foo/foo_1.0_amd64.deb'

  if ! switch_to_public_archive 2>/dev/null; then
    echo "self-test: the mirror list was not rewritten" >&2
    status=1
  fi
  if grep -q 'azure' "$root/apt-mirrors.txt"; then
    echo "self-test: the Azure hostname survived in the mirror list" >&2
    status=1
  fi
  if ! grep -q '^http://archive\.ubuntu\.com/ubuntu/.*priority:1' "$root/apt-mirrors.txt"; then
    echo "self-test: the first mirror is not the public archive at its priority" >&2
    status=1
  fi
  expect_resolved 'http://archive.ubuntu.com/ubuntu/pool/main/f/foo/foo_1.0_amd64.deb'
  # Second call has nothing left to move and must say so rather than claim a switch.
  if switch_to_public_archive > /dev/null 2>&1; then
    echo "self-test: an already-public mirror list reported a rewrite" >&2
    status=1
  fi

  # Classic 22.04 layout: the hostname is in the sources and in a *.list file.
  rm -f "$root/apt-mirrors.txt" "$root/sources.list.d/ubuntu.sources"
  printf 'deb http://azure.archive.ubuntu.com/ubuntu noble main\n' > "$root/sources.list"
  if ! switch_to_public_archive 2>/dev/null; then
    echo "self-test: a classic sources.list was not rewritten" >&2
    status=1
  fi
  if ! grep -q 'http://archive\.ubuntu\.com/ubuntu noble main' "$root/sources.list"; then
    echo "self-test: the classic sources.list kept the Azure hostname" >&2
    status=1
  fi

  # ARM runners use ports.ubuntu.com and must move to the matching port archive.
  printf 'URIs: http://azure.ports.ubuntu.com/ubuntu-ports\n' > "$root/sources.list.d/ubuntu.sources"
  if ! switch_to_public_archive 2>/dev/null; then
    echo "self-test: an arm64 sources file was not rewritten" >&2
    status=1
  fi
  if grep -q 'azure' "$root/sources.list.d/ubuntu.sources"; then
    echo "self-test: the arm64 sources file kept the Azure hostname" >&2
    status=1
  fi

  # A tree that never used the Azure mirror must not be touched, and must report
  # it: the old script's silent success is the defect this covers.
  printf 'URIs: http://archive.ubuntu.com/ubuntu\n' > "$root/sources.list.d/ubuntu.sources"
  printf 'deb http://archive.ubuntu.com/ubuntu noble main\n' > "$root/sources.list"
  if switch_to_public_archive > /dev/null 2>&1; then
    echo "self-test: a public-only tree reported a rewrite" >&2
    status=1
  fi
  for file in "$root/sources.list" "$root/sources.list.d/ubuntu.sources"; do
    if ! grep -q 'http://archive\.ubuntu\.com/ubuntu' "$file"; then
      echo "self-test: ${file} was damaged by a no-op switch" >&2
      status=1
    fi
  done

  if (( status != 0 )); then
    echo "self-test: FAILED" >&2
    return 1
  fi
  echo "self-test: ok (mirror list, classic list, arm64 list, no-op)"
  return 0
}

if [[ ${1:-} == --self-test ]]; then
  self_test
  exit 0
fi

last_hosts=""
last_phase=""

if ! update_index; then
  echo "APT update did not finish within ${update_budget}s; switching to Ubuntu's public archive." >&2
  switch_to_public_archive || true
  if ! update_index; then
    echo "APT update also failed after switching to Ubuntu's public archive." >&2
    exit 1
  fi
fi

status=0
install_packages "$@" || status=$?
if (( status == 0 )); then
  exit 0
fi

echo "Package installation did not finish (status ${status}) during ${last_phase} from ${last_hosts}." >&2
still_pending=$(pending_packages "$@")
if [[ -n $still_pending ]]; then
  echo "Archives still outstanding: ${still_pending}" >&2
fi
switch_to_public_archive || true
echo "Retrying installation from Ubuntu's public archive." >&2
if ! update_index; then
  echo "APT update failed after switching to Ubuntu's public archive." >&2
  exit 1
fi
install_packages "$@"
