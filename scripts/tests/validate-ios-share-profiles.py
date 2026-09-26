#!/usr/bin/env python3
"""Fail before archiving if the host/extension profiles lack shared entitlements."""
from datetime import datetime, timezone
from pathlib import Path
import plistlib
import sys

GROUP = "group.cn.futureos.mobile.share"


def validate(host: dict, share: dict) -> None:
    if host.get("TeamIdentifier") != share.get("TeamIdentifier") or not host.get("TeamIdentifier"):
        raise ValueError("Host and Share Extension profiles must belong to the same Apple team")
    for profile, bundle in [(host, "cn.futureos.mobile"), (share, "cn.futureos.mobile.share")]:
        entitlements = profile.get("Entitlements", {})
        prefixes = profile.get("ApplicationIdentifierPrefix", [])
        if entitlements.get("application-identifier") not in [f"{prefix}.{bundle}" for prefix in prefixes]:
            raise ValueError(f"Provisioning profile must explicitly match {bundle}")
        if GROUP not in entitlements.get("com.apple.security.application-groups", []):
            raise ValueError(f"Enable App Group {GROUP} for {bundle} and regenerate its profile")
        if entitlements.get("get-task-allow") or profile.get("ProvisionedDevices") or profile.get("ProvisionsAllDevices"):
            raise ValueError(f"{bundle} needs an App Store distribution profile")
        expires = profile.get("ExpirationDate")
        if not isinstance(expires, datetime) or expires.replace(tzinfo=timezone.utc) <= datetime.now(timezone.utc):
            raise ValueError(f"Provisioning profile for {bundle} is expired")


if __name__ == "__main__":
    try:
        validate(*(plistlib.loads(Path(filename).read_bytes()) for filename in sys.argv[1:]))
    except (ValueError, TypeError) as error:
        sys.exit(str(error))
    print("Host and Share Extension profiles have matching App Group entitlements")
