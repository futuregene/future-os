#!/usr/bin/env bash
# Install CI-only Ubuntu packages without letting the Azure runner mirror hold
# a job hostage. GitHub's Ubuntu images normally point x86_64 runners at
# azure.archive.ubuntu.com and ARM runners at azure.ports.ubuntu.com; Ubuntu's
# corresponding public archive endpoints are the fallback.
set -Eeuo pipefail

if (( $# == 0 )); then
  echo "usage: $0 <apt-package> [...]" >&2
  exit 2
fi

# Do not let one unavailable mirror consume the job's entire setup budget. APT
# otherwise retries individual index downloads for minutes before surfacing an
# error. The retry count is deliberately zero here: the second attempt is made
# against the official archive below, rather than the same unhealthy endpoint.
apt_update_args=(
  -o Acquire::Retries=0
  -o Acquire::http::Timeout=15
  -o Acquire::https::Timeout=15
  -o Dpkg::Use-Pty=0
)

# Point the runner's Azure mirror at Ubuntu's public archive. Ubuntu 24.04
# runners use deb822 (*.sources); older images may use classic *.list files.
# Only the archive hostname is rewritten, preserving suites, components and any
# third-party source configuration.
switch_to_public_archive() {
  while IFS= read -r -d '' source_file; do
    sudo sed -i \
      -e 's|azure\.archive\.ubuntu\.com|archive.ubuntu.com|g' \
      -e 's|azure\.ports\.ubuntu\.com|ports.ubuntu.com|g' \
      "$source_file"
  done < <(
    sudo grep -rElZ --include='*.list' --include='*.sources' \
      'azure\.(archive|ports)\.ubuntu\.com' /etc/apt/sources.list /etc/apt/sources.list.d 2>/dev/null || true
  )
}

# CI never needs recommended GUI extras; dependencies declared by the selected
# packages remain installed. Keep limited download retries for transient package
# fetches after the index source has been established.
install_packages() {
  sudo timeout 180s apt-get \
    -o Acquire::Retries=2 \
    -o Acquire::http::Timeout=15 \
    -o Acquire::https::Timeout=15 \
    -o Dpkg::Use-Pty=0 \
    --no-install-recommends \
    install -y "$@"
}

update_index() {
  sudo timeout 45s apt-get "${apt_update_args[@]}" update
}

if ! update_index; then
  echo "Azure Ubuntu mirror did not update within 45 seconds; switching to Ubuntu's public archive." >&2
  switch_to_public_archive
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

# A failing fetch keeps its own retries, but a *stalled* one has none:
# `Acquire::Retries` retries an error, not a connection that opens and then
# crawls, and `http::Timeout` bounds the socket rather than the transfer. That
# is how a run sat on ~2.5 MB from the Azure mirror until the 180 s budget ran
# out and the job died with 124 (run 39802801818, three jobs at once). The
# endpoint is the problem, so it is rewritten and the installation tried again.
echo "Package installation did not finish (status ${status}); switching to Ubuntu's public archive and retrying." >&2
switch_to_public_archive
if ! update_index; then
  echo "APT update failed after switching to Ubuntu's public archive." >&2
  exit 1
fi
install_packages "$@"
