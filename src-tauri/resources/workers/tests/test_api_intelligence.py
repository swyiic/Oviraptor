import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch


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

    def test_merges_repeated_identifier_urls_but_preserves_method_and_parameter_shape(self):
        records = [
            {"url": "https://api.example.test/api/users/123?id=123", "method": "GET", "source": "browser-runtime", "statusCode": 200},
            {"url": "https://api.example.test/api/users/456?id=456", "method": "GET", "source": "babel-ast"},
            {"url": "https://api.example.test/api/users/456?id=456", "method": "POST", "parameters": ["id"], "source": "browser-runtime", "statusCode": 204},
            {"url": "https://api.example.test/api/users/456?user_id=456", "method": "GET", "parameters": ["user_id"], "source": "browser-runtime", "statusCode": 200},
        ]
        merged = frontend_recon.merge_api_contracts(records)
        self.assertEqual(len(merged), 3)
        get_id = next(item for item in merged if item["method"] == "GET" and item["parameters"] == ["id"])
        self.assertEqual(get_id["normalizedPath"], "/api/users/{id}")
        self.assertEqual(get_id["observationCount"], 1)
        self.assertEqual(get_id["parameters"], ["id"])

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

    def test_verified_unknown_static_path_becomes_observed_get_contract(self):
        candidates = [{
            "url": "https://example.test/api/search",
            "path": "/api/search",
            "method": "UNKNOWN",
            "confidence": "medium",
            "extractionEngine": "string-heuristic",
        }]
        check = {
            "status": "verified", "verified": True, "probeMethod": "GET",
            "reason": "structured_success_response", "httpStatus": 200,
        }
        with patch.object(frontend_recon, "verify_api_candidate", return_value=check):
            verified, pending = frontend_recon.verify_api_candidates(
                candidates, "https://example.test/app", "<html></html>", 0.1, 1,
            )
        self.assertEqual(pending, [])
        self.assertEqual(verified[0]["method"], "GET")
        self.assertEqual(verified[0]["methodSource"], "verified_safe_probe")
        self.assertFalse(verified[0]["candidateOnly"])

    def test_merge_identity_runs_preserves_runtime_from_any_identity_without_false_comparison(self):
        merged = frontend_recon.merge_identity_runs([
            {
                "identityKey": "session-a",
                "statusCode": 200,
                "runtimeExploration": {
                    "available": False,
                    "captureStatus": "failed",
                    "captureError": "session expired",
                    "states": [],
                    "actions": [],
                    "requests": [],
                    "errors": ["browser validation failed"],
                },
                "authSessionValidation": {
                    "valid": False,
                    "clearSessionInvalid": True,
                    "reason": "session_invalid",
                },
            },
            {
                "identityKey": "session-b",
                "statusCode": 200,
                "apis": [{
                    "method": "GET",
                    "url": "https://example.test/api/profile",
                    "statusCode": 200,
                    "responseKeys": ["id", "name"],
                }],
                "runtimeExploration": {
                    "available": True,
                    "browser": "Chromium",
                    "captureStatus": "complete",
                    "states": [{"id": "state-1"}],
                    "actions": [{"id": "action-1"}],
                    "requests": [{"url": "https://example.test/api/profile"}],
                    "coverage": {"requests": 1},
                },
                "authSessionValidation": {
                    "valid": True,
                    "clearSessionInvalid": False,
                    "reason": "session_active",
                },
            },
        ])
        self.assertTrue(merged["runtimeExploration"]["available"])
        self.assertEqual(merged["runtimeExploration"]["browser"], "Chromium")
        self.assertEqual(merged["runtimeExploration"]["captureStatus"], "partial")
        self.assertEqual(merged["runtimeExploration"]["coverage"]["requests"], 1)
        self.assertEqual(merged["identityRuns"][0]["captureStatus"], "failed")
        self.assertEqual(merged["identityRuns"][1]["captureStatus"], "complete")
        self.assertFalse(merged["analysisSummary"]["identityComparisonComparable"])
        self.assertEqual(merged["identityComparisons"], [])
        self.assertFalse(merged["authSessionValidation"]["valid"])

    def test_only_concrete_requests_are_marked_ready_for_agent_validation(self):
        inferred = {
            "url": "https://example.test/api/account/detail",
            "method": "UNKNOWN",
            "confidence": "high",
            "candidateOnly": True,
            "extractionEngine": "evidence-reconstruction",
        }
        direct = {
            "url": "https://example.test/login.wsp",
            "method": "POST",
            "parameters": ["username", "password"],
            "confidence": "high",
            "extractionEngine": "babel-ast",
        }
        routes = [{"path": "/admin", "confidence": "high", "extractionEngine": "babel-ast"}]
        opportunities = frontend_recon.build_security_opportunities(
            "https://example.test/login", {}, {}, [inferred, direct], routes, {"requests": []},
        )
        inferred_items = [item for item in opportunities if item.get("endpoint", "").endswith("/api/account/detail")]
        route_item = next(item for item in opportunities if item.get("route") == "/admin")
        self.assertEqual(inferred_items, [])
        self.assertFalse(any(item.get("endpoint", "").endswith("/login.wsp") for item in opportunities))
        self.assertEqual(route_item["status"], "queued")

    def test_auth_session_capture_converts_to_real_runtime_requests(self):
        session = {
            "id": "session-a",
            "capturedRequests": [{
                "url": "https://api.example.test/api/profile?id=1&nonce=volatile",
                "method": "GET",
                "transport": "xhr",
                "status": 200,
                "headers": {"accept": "application/json"},
            }],
        }
        requests = frontend_recon._auth_session_runtime_requests(session)
        self.assertEqual(len(requests), 1)
        self.assertEqual(requests[0]["source"], "auth-session-capture")
        self.assertEqual(requests[0]["statusCode"], 200)
        self.assertEqual(requests[0]["queryKeys"], ["id", "nonce"])
        self.assertEqual(requests[0]["requestSafety"]["class"], "read")

    def test_runtime_and_auth_capture_merge_ignores_volatile_query_values(self):
        runtime = [{
            "url": "https://api.example.test/api/profile?id=2&nonce=second",
            "method": "GET",
            "queryKeys": ["id", "nonce"],
            "source": "browser-runtime",
            "statusCode": 200,
        }]
        auth = [{
            "url": "https://api.example.test/api/profile?id=1&nonce=first",
            "method": "GET",
            "queryKeys": ["id", "nonce"],
            "source": "auth-session-capture",
            "statusCode": 200,
        }]
        merged, added = frontend_recon.merge_runtime_and_auth_requests(runtime, auth)
        self.assertEqual(len(merged), 1)
        self.assertEqual(added, 0)
        self.assertEqual(merged[0]["source"], "browser-runtime")

    def test_auth_fallback_does_not_make_identity_api_comparison_comparable(self):
        def run(identity):
            return {
                "identityKey": identity,
                "apis": [{"method": "GET", "url": "https://api.example.test/api/profile?id=1", "statusCode": 200, "responseKeys": ["id"]}],
                "runtimeExploration": {
                    "available": False, "captureStatus": "failed",
                    "captureError": "cdp_command_pipe_unavailable",
                    "requests": [{"method": "GET", "url": "https://api.example.test/api/profile?id=1"}],
                    "authSessionCapture": {"capturedRequestCount": 1, "usedAsFallback": True},
                },
                "authSessionValidation": {"valid": True, "clearSessionInvalid": False},
            }
        merged = frontend_recon.merge_identity_runs([run("a"), run("b")])
        self.assertFalse(merged["analysisSummary"]["apiComparisonComparable"])
        self.assertEqual(len(merged["identityRuns"]), 2)
        self.assertTrue(all(item["sessionValid"] is True for item in merged["identityRuns"]))

    def test_cdp_failure_without_auth_capture_is_not_session_invalid(self):
        run = {
            "identityKey": "a",
            "runtimeExploration": {
                "available": False, "captureStatus": "failed",
                "captureError": "cdp_command_pipe_unavailable", "requests": [],
            },
            "authSessionValidation": {"valid": False, "clearSessionInvalid": False},
        }
        merged = frontend_recon.merge_identity_runs([run])
        self.assertIsNone(merged["identityRuns"][0]["sessionValid"])
        self.assertFalse(merged["authSessionValidation"]["clearSessionInvalid"])

    def test_generic_observed_get_is_kept_as_raw_evidence_not_high_value_opportunity(self):
        runtime = [{
            "url": "https://api.example.test/bbs/app/feeds?offset=0",
            "method": "GET",
            "resourceType": "xhr",
            "status": 200,
            "feature": "authenticated-session-capture",
        }]
        candidate = {
            "url": "https://api.example.test/bbs/app/feeds?offset=0",
            "method": "GET",
            "parameters": ["offset"],
            "source": "browser-runtime",
            "extractionEngine": "browser-runtime",
            "confidence": "high",
        }
        opportunities = frontend_recon.build_security_opportunities(
            "https://example.test/app", {}, {}, [candidate], [], {"requests": runtime},
        )
        self.assertEqual(opportunities, [])

    def test_identity_get_remains_eligible_for_ai_validation(self):
        runtime = [{
            "url": "https://api.example.test/api/account/profile?id=123",
            "method": "GET",
            "resourceType": "xhr",
            "status": 200,
            "feature": "account-profile",
        }]
        candidate = {
            "url": "https://api.example.test/api/account/profile?id=123",
            "method": "GET",
            "parameters": ["id"],
            "source": "browser-runtime",
            "extractionEngine": "browser-runtime",
            "confidence": "high",
        }
        opportunities = frontend_recon.build_security_opportunities(
            "https://example.test/app", {}, {}, [candidate], [], {"requests": runtime},
        )
        self.assertEqual(len(opportunities), 1)
        self.assertEqual(opportunities[0]["verificationMode"], "ai_auto")
        self.assertEqual(opportunities[0]["humanReviewStage"], "final_verdict_only")
        self.assertTrue(opportunities[0]["riskEvidence"]["present"])
        self.assertEqual(opportunities[0]["riskEvidence"]["signals"][0]["type"], "object_boundary_parameter")

    def test_observed_session_restore_is_inventory_not_security_opportunity(self):
        runtime = [{
            "url": "https://api.example.test/account/restore_login",
            "method": "GET",
            "resourceType": "xhr",
            "status": 200,
            "feature": "authenticated-session-capture",
            "responseKeys": ["status", "message"],
        }]
        candidate = {
            "url": "https://api.example.test/account/restore_login",
            "method": "GET",
            "source": "browser-runtime",
            "extractionEngine": "browser-runtime",
            "confidence": "high",
        }
        opportunities = frontend_recon.build_security_opportunities(
            "https://example.test/app", {}, {}, [candidate], [], {"requests": runtime},
        )
        self.assertEqual(opportunities, [])

    def test_transport_device_id_does_not_create_an_idor_opportunity(self):
        runtime = [{
            "url": "https://api.example.test/account/restore_login?device_id=browser-1&nonce=n1",
            "method": "GET", "resourceType": "xhr", "status": 200,
            "queryKeys": ["device_id", "nonce"], "responseKeys": ["status"],
        }]
        candidate = {
            "url": "https://api.example.test/account/restore_login?device_id=browser-1&nonce=n1",
            "method": "GET", "parameters": ["device_id", "nonce"],
            "source": "browser-runtime", "extractionEngine": "browser-runtime",
        }
        opportunities = frontend_recon.build_security_opportunities(
            "https://example.test/app", {}, {}, [candidate], [], {"requests": runtime},
        )
        self.assertEqual(opportunities, [])

    def test_opportunity_contract_ignores_identity_and_volatile_values(self):
        left = {
            "category": "identity_surface", "method": "GET",
            "endpoint": "https://api.example.test/api/account/profile?id=1&nonce=a",
            "parameters": ["id", "nonce"], "identityKeys": ["account-a"],
        }
        right = {
            **left,
            "endpoint": "https://api.example.test/api/account/profile?id=2&nonce=b",
            "identityKeys": ["account-b"],
        }
        self.assertEqual(
            frontend_recon.opportunity_contract_key(left),
            frontend_recon.opportunity_contract_key(right),
        )

    def test_privilege_mutation_has_explicit_risk_evidence(self):
        runtime = [{
            "url": "https://api.example.test/api/admin/member/role",
            "method": "PATCH",
            "resourceType": "xhr",
            "status": 200,
            "bodyKeys": ["member_id", "role_id"],
            "feature": "member-role-editor",
        }]
        candidate = {
            "url": "https://api.example.test/api/admin/member/role",
            "method": "PATCH",
            "parameters": ["member_id", "role_id"],
            "source": "browser-runtime",
            "extractionEngine": "browser-runtime",
            "confidence": "high",
        }
        opportunities = frontend_recon.build_security_opportunities(
            "https://example.test/app", {}, {}, [candidate], [], {"requests": runtime},
        )
        self.assertEqual(len(opportunities), 1)
        signal_types = {item["type"] for item in opportunities[0]["riskEvidence"]["signals"]}
        self.assertIn("object_boundary_parameter", signal_types)
        self.assertIn("security_relevant_mutation", signal_types)

    def test_packaged_sensitive_rules_run_pattern_and_exclusion_passes(self):
        frontend_recon.configured_sensitive_rules.cache_clear()
        findings = frontend_recon.extract_sensitive(
            "https://example.test/app.js",
            'const client_secret = "Abcd1234_secure_value";',
        )
        oauth = next(item for item in findings if item["type"] == "oauth_client_secret")
        self.assertEqual(oauth["ruleId"], "secret-oauth-client-secret")
        self.assertEqual(oauth["reviewPasses"], ["pattern_match", "exclusion_and_semantic_review"])
        self.assertEqual(oauth["severity"], "high")

        placeholder = frontend_recon.extract_sensitive(
            "https://example.test/template.js",
            'const client_secret = "your-secret";',
        )
        self.assertFalse(any(item["type"] == "oauth_client_secret" for item in placeholder))

    def test_mobilee_noise_exclusion_does_not_create_token_finding(self):
        findings = frontend_recon.extract_sensitive(
            "https://example.test/sm9.js",
            'ThreshSign logger.info("client token: %d", signerdata);',
        )
        self.assertFalse(any(item["type"] in {"bearer_token", "jwt", "hardcoded_credential"} for item in findings))

    def test_non_sensitive_rule_channels_do_not_pollute_sensitive_results(self):
        findings = frontend_recon.extract_sensitive(
            "https://example.test/app.js",
            'const endpoint = "/api/users"; const cipher = "AES/ECB"; Cipher.getInstance(cipher);',
        )
        self.assertEqual(findings, [])
        crypto = frontend_recon.extract_crypto_signals(
            "https://example.test/app.js",
            'const cipher = "AES/ECB"; Cipher.getInstance(cipher);',
        )
        self.assertTrue(any(item.get("ruleId") == "crypto-weak-algorithm" for item in crypto))


if __name__ == "__main__":
    unittest.main()
