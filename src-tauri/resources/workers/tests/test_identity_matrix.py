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
                        "requestHeaders": {"accept": "application/json", "x-account": identity},
                        "responseHeaders": {"content-type": "application/json"},
                        "responsePreview": '{"id":1,"email":"test@example.test"}',
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
        matrix = next(
            item["matrix"]
            for item in merged["identityComparisons"]
            if item["differenceType"] == "response_schema"
        )
        self.assertEqual(matrix["session:a:admin"]["requestHeaders"]["x-account"], "session:a:admin")
        self.assertEqual(matrix["session:b:user"]["responseHeaders"]["content-type"], "application/json")
        self.assertIn('"email"', matrix["session:a:admin"]["responseBody"])

    def test_waf_stops_matrix(self):
        first = self.run_for("session:a:admin", 403, [])
        first["authSessionValidation"]["wafDetected"] = True
        merged = frontend_recon.merge_identity_runs([first])
        self.assertTrue(merged["authSessionValidation"]["wafDetected"])
        self.assertEqual(
            merged["runtimeExploration"]["stopReason"],
            "confirmed_waf_or_challenge",
        )

    def test_anonymous_capture_is_not_named_account_a(self):
        anonymous = self.run_for("anonymous", 200, ["id"])
        anonymous["authSessionValidation"] = {"valid": False, "wafDetected": False}
        merged = frontend_recon.merge_identity_runs([anonymous])
        self.assertEqual(merged["identityRuns"][0]["identityLabel"], "匿名访问")
        self.assertEqual(merged["runtimeExploration"]["stopReason"], "anonymous_runtime_complete")

    def test_authenticated_labels_ignore_anonymous_control_position(self):
        anonymous = self.run_for("anonymous", 200, ["id"])
        account = self.run_for("session:a:user", 200, ["id"])
        merged = frontend_recon.merge_identity_runs([anonymous, account])
        labels = {item["identityKey"]: item["identityLabel"] for item in merged["identityRuns"]}
        self.assertEqual(labels["anonymous"], "匿名访问")
        self.assertEqual(labels["session:a:user"], "账号 A")

    def test_response_schema_drops_serialized_body_keys(self):
        polluted = ['{"msg":"","result":{"items":[1,2,3]}}', "msg", "result"]
        self.assertEqual(
            frontend_recon.sanitized_response_keys(polluted),
            ["msg", "result"],
        )

    def test_static_telemetry_and_bootstrap_requests_stay_out_of_identity_matrix(self):
        first = self.run_for("session:a", 200, ["id"])
        second = self.run_for("session:b", 200, ["id"])
        noise = [
            {"method": "GET", "url": "https://cdn.test/avatar/u.jpeg", "resourceType": "Fetch", "statusCode": 200},
            {"method": "POST", "url": "https://fp-it.portal101.cn/deviceprofile/v4", "resourceType": "Fetch", "statusCode": 200},
            {"method": "GET", "url": "https://example.test/bbs/app/topic/categories", "resourceType": "Fetch", "statusCode": 200},
            {"method": "UNKNOWN", "url": "https://example.test/bbs/app/api/general/search/v1/web", "source": "string-heuristic"},
        ]
        first["apis"].extend(noise)
        second["apis"].extend(noise)
        merged = frontend_recon.merge_identity_runs([first, second])
        urls = [item["url"] for item in merged["apis"]]
        self.assertEqual(urls, ["https://example.test/api/profile"])
        self.assertTrue(all("categories" not in item["apiKey"] for item in merged["identityComparisons"]))

    def test_rendered_page_text_never_becomes_an_authorization_difference(self):
        first = self.run_for("session:a", 200, ["id"])
        second = self.run_for("session:b", 200, ["id"])
        first["features"] = [{
            "url": "https://example.test/messagecenter/follow",
            "highValueLabels": ["京公网安备11010502034222号"],
            "interactiveCount": 13,
        }]
        second["features"] = []
        merged = frontend_recon.merge_identity_runs([first, second])
        self.assertFalse(any(
            item.get("differenceType") == "feature_surface"
            for item in merged["identityComparisons"]
        ))
        self.assertNotIn("京公网安备", str(merged["identityComparisons"]))


if __name__ == "__main__":
    unittest.main()
