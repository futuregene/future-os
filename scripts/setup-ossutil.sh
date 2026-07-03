#!/usr/bin/env bash
#
# setup-ossutil.sh — install Aliyun ossutil v2 and prepare it for CI use.
#
# Downloads ossutil v2 from Aliyun's public CDN (no third-party GitHub Action),
# installs it to /usr/local/bin, and derives OSS_REGION from OSS_ENDPOINT so the
# v2 SigV4 signer has a region. Authentication itself is handled by ossutil v2's
# automatic reading of the OSS_* environment variables:
#
#   OSS_ENDPOINT            e.g. oss-cn-hangzhou.aliyuncs.com   (required)
#   OSS_ACCESS_KEY_ID       AccessKey ID                        (required)
#   OSS_ACCESS_KEY_SECRET   AccessKey Secret                    (required)
#   OSS_REGION              derived here and exported to GITHUB_ENV
#
# Assumes a Linux x86_64 GitHub-hosted runner. Bump OSSUTIL_VERSION to upgrade;
# verify the CDN URL returns 200 first (some patch versions don't exist).

set -euo pipefail

OSSUTIL_VERSION="${OSSUTIL_VERSION:-2.2.0}"
pkg="ossutil-${OSSUTIL_VERSION}-linux-amd64"
url="https://gosspublic.alicdn.com/ossutil/v2/${OSSUTIL_VERSION}/${pkg}.zip"

echo "==> Downloading ossutil ${OSSUTIL_VERSION}"
curl -fsSL -o /tmp/ossutil.zip "$url"
unzip -q -o /tmp/ossutil.zip -d /tmp
sudo mv "/tmp/${pkg}/ossutil" /usr/local/bin/ossutil
sudo chmod +x /usr/local/bin/ossutil
ossutil version

# ossutil v2 signs requests with SigV4, which needs a region. Standard OSS
# endpoints are oss-<region>.aliyuncs.com — strip the prefix/suffix (and any
# scheme or -internal marker) to recover the region.
region="$(printf '%s' "${OSS_ENDPOINT:?OSS_ENDPOINT is required}" \
  | sed -E 's#^https?://##; s/^oss-//; s/-internal//; s/\.aliyuncs\.com$//')"
echo "==> Derived OSS_REGION=$region"
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "OSS_REGION=$region" >>"$GITHUB_ENV"
fi
