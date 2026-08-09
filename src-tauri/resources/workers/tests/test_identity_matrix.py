import importlib.util
import unittest
from pathlib import Path


WORKER = Path(__file__).resolve().parents[1] / "7_frontend_recon.py"
SPEC = importlib.util.spec_from_file_location("frontend_recon_identity", WORKER)
frontend_recon = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(frontend_recon)


class IdentityMatrixTests(unittest.TestCase):
    def run_for(self, identity, status, response_keys):
        return frontend_recon.tag_identity_run(
            {
                "url": "https://example.test/app",
                "finalUrl": "https://example.test/app",
                "apis": [
                    {
                        "method": "GET",
                        "url": "https://example.test/api/profile",
                        "parameters": ["id"],
                        "statusCode": status,
                        "responseKeys": response_keys,
                    }
                ],
                "opportunities": [],
                "runtimeExploration": {
                    "states": [{"id": "state-1", "url": "https://example.test/app"}],
                    "actions": [{"id": "action-1", "stateId": "state-1"}],
                    "requests": [
                        {
                            "method": "GET",
                            "url": "https://example.test/api/profile",
                            "stateId": "state-1",
                            "actionId": "action-1",
                        }
                    ],
                    "blockedRequests": [],
                    "coverage": {"stateCount": 1, "actionCount": 1},
                },
                "authSessionValidation": {"valid": True, "wafDetected": False},
                "analysisSummary": {},
            },
            identity,
            0 if identity.endswith("admin") else 1,
        )

    def test_merges_identity_observations_and_schema_difference(self):
        merged = frontend_recon.merge_identity_runs(
            [
                self.run_for("session:a:admin", 200, ["id", "email", "role"]),
                self.run_for("session:b:user", 200, ["id", "email"]),
            ]
        )
        self.assertEqual(len(merged["identityRuns"]), 2)
        self.assertEqual(len(merged["apis"]), 1)
        self.assertEqual(
            merged["apis"][0]["identityKeys"],
            ["session:a:admin", "session:b:user"],
        )
        self.assertTrue(
            any(
                item["differenceType"] == "response_schema"
                for item in merged["identityComparisons"]
            )
        )
        self.assertNotEqual(
            merged["runtimeExploration"]["states"][0]["id"],
            merged["runtimeExploration"]["states"][1]["id"],
        )

    def test_waf_stops_matrix(self):
        first = self.run_for("session:a:admin", 403, [])
        first["authSessionValidation"]["wafDetected"] = True
        merged = frontend_recon.merge_identity_runs([first])
        self.assertTrue(merged["authSessionValidation"]["wafDetected"])
        self.assertEqual(
            merged["runtimeExploration"]["stopReason"],
            "confirmed_waf_or_challenge",
        )


if __name__ == "__main__":
    unittest.main()
