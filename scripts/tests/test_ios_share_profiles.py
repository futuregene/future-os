import copy
from datetime import datetime, timedelta, timezone
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("profiles", Path(__file__).resolve().parents[0] / "validate-ios-share-profiles.py")
profiles = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profiles)


def profile(bundle):
    return {
        "TeamIdentifier": ["TEAM"],
        "ApplicationIdentifierPrefix": ["PREFIX"],
        "ExpirationDate": datetime.now(timezone.utc) + timedelta(days=2),
        "Entitlements": {
            "application-identifier": f"PREFIX.{bundle}",
            "com.apple.security.application-groups": [profiles.GROUP],
            "get-task-allow": False,
        },
    }


class ProfileTests(unittest.TestCase):
    def test_matching_store_profiles(self):
        profiles.validate(profile("cn.futureos.mobile"), profile("cn.futureos.mobile.share"))

    def test_bad_entitlements_or_distribution_are_rejected(self):
        base = profile("cn.futureos.mobile.share")
        changes = [
            {"TeamIdentifier": ["OTHER"]},
            {"ExpirationDate": datetime(2000, 1, 1)},
            {"ProvisionedDevices": ["device"]},
            {"ProvisionsAllDevices": True},
            {"Entitlements": {**base["Entitlements"], "get-task-allow": True}},
            {"Entitlements": {**base["Entitlements"], "application-identifier": "PREFIX.*"}},
            {"Entitlements": {**base["Entitlements"], "com.apple.security.application-groups": []}},
        ]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ValueError):
                profiles.validate(profile("cn.futureos.mobile"), {**copy.deepcopy(base), **change})

    def test_old_host_profile_without_app_group_is_rejected(self):
        host = profile("cn.futureos.mobile")
        del host["Entitlements"]["com.apple.security.application-groups"]
        with self.assertRaises(ValueError):
            profiles.validate(host, profile("cn.futureos.mobile.share"))
