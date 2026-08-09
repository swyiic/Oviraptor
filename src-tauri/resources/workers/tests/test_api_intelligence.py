import importlib.util
import unittest
from pathlib import Path


WORKER = Path(__file__).resolve().parents[1] / "7_frontend_recon.py"
SPEC = importlib.util.spec_from_file_location("frontend_recon", WORKER)
frontend_recon = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(frontend_recon)


class ApiIntelligenceTests(unittest.TestCase):
    def test_splits_prefix_business_path_and_normalizes_identifiers(self):
        split = frontend_recon.infer_api_split("/gateway/api/v2/users/1234567890123456/detail")
        self.assertEqual(split["apiPrefix"], "/gateway/api/v2")
        self.assertEqual(split["businessEndpoint"], "/users/1234567890123456/detail")
        self.assertEqual(
            frontend_recon.normalized_endpoint_path("/gateway/api/v2/users/1234567890123456/detail"),
            "/gateway/api/v2/users/{id}/detail",
        )

    def test_reconstructs_business_path_from_runtime_supported_prefix(self):
        runtime = [
            {"resourceType": "Fetch", "url": "https://api.example.test/api/v1/auth/login", "method": "POST"},
            {"resourceType": "XHR", "url": "https://api.example.test/api/v1/user/profile", "method": "GET"},
        ]
        ast = [{"stringEvidence": {
            "baseUrls": [], "apiPrefixes": [],
            "businessPaths": [{"value": "/user/list", "source": "app.js", "line": 42}],
            "storageReferences": [],
        }}]
        result = frontend_recon.build_api_intelligence("https://www.example.test/login", runtime, ast, [])
        candidate = result["candidates"][0]
        self.assertEqual(candidate["url"], "https://api.example.test/api/v1/user/list")
        self.assertEqual(candidate["apiPrefix"], "/api/v1")
        self.assertEqual(candidate["businessEndpoint"], "/user/list")
        self.assertGreaterEqual(candidate["reconstructionConfidence"], 0.84)
        self.assertEqual(candidate["extractionEngine"], "evidence-reconstruction")

    def test_does_not_reconstruct_from_an_orphan_business_string(self):
        ast = [{"stringEvidence": {
            "baseUrls": [], "apiPrefixes": [],
            "businessPaths": [{"value": "/admin/user/list", "source": "chunk.js"}],
            "storageReferences": [],
        }}]
        result = frontend_recon.build_api_intelligence("https://example.test", [], ast, [])
        self.assertEqual(result["candidates"], [])

    def test_client_base_resolution_does_not_duplicate_an_existing_prefix(self):
        records = [{
            "path": "/api/v1/user/list", "method": "GET", "clientBaseUrl": "https://api.example.test/api/v1",
            "source": "app.js", "confidence": "high", "extractionEngine": "babel-ast",
        }]
        normalized = frontend_recon.normalize_ast_apis("https://www.example.test", records, [])
        self.assertEqual(normalized[0]["url"], "https://api.example.test/api/v1/user/list")

    def test_header_intelligence_separates_observed_declared_and_possible(self):
        runtime = [{
            "resourceType": "Fetch",
            "url": "https://api.example.test/api/v1/user/profile",
            "method": "GET",
            "headers": {"Accept": "application/json"},
            "effectiveRequestHeaders": {
                "Accept": "application/json",
                "Authorization": "Bearer runtime-token",
                "Sec-Fetch-Site": "same-site",
            },
            "extraInfoRequestHeaderNames": ["Authorization", "Sec-Fetch-Site"],
            "extraRequestHeaderNames": ["Authorization", "Sec-Fetch-Site"],
        }]
        ast = [{"headerEvidence": [{
            "name": "X-Tenant-Id", "value": "<tenantId>", "dynamic": True,
            "sourceKind": "request-config", "source": "app.js", "line": 21,
        }]}]
        result = frontend_recon.build_header_intelligence(runtime, ast)
        observed = {item["name"].lower(): item for item in result["observed"]}
        declared = {item["name"].lower(): item for item in result["declared"]}
        self.assertTrue(observed["authorization"]["observed"])
        self.assertIn("browser-extra-info", observed["authorization"]["sources"])
        self.assertTrue(declared["x-tenant-id"]["declared"])
        self.assertTrue(all(item["possibleOnly"] for item in result["possibleBrowserManaged"]))

    def test_runtime_observed_api_is_not_downgraded_by_unauthenticated_probe(self):
        candidates = [{
            "url": "https://api.example.test/api/v1/account",
            "path": "/api/v1/account",
            "method": "GET",
            "confidence": "high",
            "extractionEngine": "browser-runtime",
            "statusCode": 200,
            "contentType": "application/json",
        }]
        verified, pending = frontend_recon.verify_api_candidates(
            candidates, "https://api.example.test/app", "<html></html>", 0.1, 0,
        )
        self.assertEqual(pending, [])
        self.assertEqual(verified[0]["verification"]["reason"], "runtime_request_observed")


if __name__ == "__main__":
    unittest.main()
