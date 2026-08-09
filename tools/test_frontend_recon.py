import importlib.util
import json
import subprocess
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "src-tauri/resources/workers/7_frontend_recon.py"
HELPER = ROOT / "src-tauri/resources/workers/8_js_ast_analyzer.cjs"
RUNTIME_HELPER = ROOT / "src-tauri/resources/workers/9_frontend_runtime_probe.cjs"
SPEC = importlib.util.spec_from_file_location("frontend_recon", WORKER)
RECON = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(RECON)


class FrontendReconTests(unittest.TestCase):
    def test_node_helper_path_strips_windows_extended_length_prefix(self):
        self.assertEqual(
            RECON.node_compatible_path(r"\\?\C:\Program Files\Oviraptor\worker.cjs"),
            r"C:\Program Files\Oviraptor\worker.cjs",
        )
        self.assertEqual(
            RECON.node_compatible_path(r"\\?\UNC\server\share\worker.cjs"),
            r"\\server\share\worker.cjs",
        )

    def test_babel_ast_resolves_constants_and_request_shapes(self):
        source = """
        const root = '/api';
        const users = root + '/users';
        const client = axios.create({baseURL: 'https://api.example.test/v2/'});
        client.post(users, {data: {id: 1, role: 'admin'}});
        fetch(`/files/${fileId}`, {method: 'DELETE'});
        const routes = [{path: '/admin', component: AdminPage}];
        """
        result = RECON.run_babel_ast("https://app.example.test/main.js", source, str(HELPER), RECON.ReconCache())
        records = RECON.normalize_ast_apis("https://app.example.test/", result["apis"], result["baseUrls"])
        by_method = {item["method"]: item for item in records}
        self.assertEqual(by_method["POST"]["url"], "https://api.example.test/v2/api/users")
        self.assertEqual(by_method["POST"]["parameters"], ["id", "role"])
        self.assertEqual(by_method["DELETE"]["path"], "/files/<fileId>")
        self.assertEqual(result["routes"][0]["path"], "/admin")
        self.assertTrue(result["codeSlices"])
        self.assertTrue(any(item["kind"] == "network-call" for item in result["codeSlices"]))
        self.assertTrue(any(item["kind"] == "dependency-definition" for item in result["codeSlices"]))

    def test_route_url_object_is_not_reported_as_api(self):
        source = "const routes=[{path:'/project',url:'/project',component:ProjectPage}]"
        self.assertEqual(RECON.extract_apis("https://app.example.test/", "main.js", source, []), [])

    def test_route_fallback_rejects_svg_and_generic_path_properties(self):
        source = """
        const icon = {path: 'M', d: 'M716.3 69.9.1.1 20.4z'};
        const ordinary = {path: '/images/logo'};
        const routes = [{path: '/admin', component: AdminPage}];
        """
        records = RECON.extract_regex_routes("main.js", source)
        self.assertEqual([item["path"] for item in records], ["/admin"])
        self.assertTrue(RECON.valid_frontend_route("/"))
        for value in ("M", "path", "d", "*"):
            self.assertFalse(RECON.valid_frontend_route(value))

    def test_framework_versions_are_extracted_for_vue_react_and_angular(self):
        vue = RECON.framework_signals("", [("vendor.js", "/*! Vue.js v3.5.13 */ createApp({})")], "")
        react = RECON.framework_signals("", [("vendor.js", "/*! React v18.3.1 */ ReactDOM.createRoot(node)")], "")
        angular = RECON.framework_signals('<app-root ng-version="17.3.12"></app-root>', [], "17.3.12")
        self.assertEqual(vue["version"], "3.5.13")
        self.assertEqual(react["version"], "18.3.1")
        self.assertEqual(angular["version"], "17.3.12")

    def test_ast_joins_nested_routes_and_reads_jsx_expression_paths(self):
        source = """
        const ADMIN = '/admin';
        const routes = [{
          path: '/account',
          component: Layout,
          children: [{path: 'profile', component: Profile}]
        }];
        const view = <Routes><Route path={ADMIN} element={<Admin />} /></Routes>;
        """
        result = RECON.run_babel_ast("main.jsx", source, str(HELPER), RECON.ReconCache())
        paths = {item["path"] for item in result["routes"]}
        self.assertIn("/account/profile", paths)
        self.assertIn("/admin", paths)

    def test_registration_detection_covers_core_support_and_localized_labels(self):
        for value in (
            "/auth/register", "/api/v1/signup", "/sys/user/register",
            "/sendSmsCode", "/invite/validate",
        ):
            self.assertIsNotNone(RECON.registration_signal(value))
        for label in (
            "创建账户", "新規登録", "회원가입", "Registrarse",
            "Créer un compte", "Registrieren", "Регистрация", "إنشاء حساب",
        ):
            localized = RECON.registration_signal("/join", label)
            self.assertEqual(localized["category"], "account_registration")
        self.assertIsNone(RECON.registration_signal("/reports/registration-report"))

    def test_ast_keeps_all_discovered_request_parameters(self):
        fields = ", ".join(f"field{index}: {index}" for index in range(75))
        source = f"axios.post('/api/register', {{ data: {{ {fields} }} }})"
        result = RECON.run_babel_ast(
            "https://app.example.test/main.js",
            source,
            str(HELPER),
            RECON.ReconCache(),
        )
        self.assertEqual(len(result["apis"][0]["parameters"]), 75)

    def test_html_parser_collects_inline_scripts_forms_and_multi_rel_preloads(self):
        parser = RECON.PageParser()
        parser.feed("""
          <link rel="preload modulepreload" as="script" href="/assets/app.js">
          <a href="/join"><span>新規登録</span></a>
          <form action="/account/new" method="post" id="signup">创建账户</form>
          <script>const routes=[{path:'/join',component:Join}]</script>
        """)
        self.assertEqual(parser.scripts, ["/assets/app.js"])
        self.assertEqual(parser.forms[0]["method"], "POST")
        self.assertEqual(parser.forms[0]["text"], "创建账户")
        self.assertEqual(parser.link_records[0], {"url": "/join", "text": "新規登録"})
        self.assertIn("const routes", parser.inline_scripts[0])

    def test_static_localized_link_becomes_registration_entrypoint_without_browser(self):
        records = RECON.registration_entrypoints(
            "https://app.example.test/",
            [],
            [],
            [],
            [{"url": "/join", "text": "회원가입"}],
            [],
            [],
        )
        self.assertEqual(records[0]["url"], "https://app.example.test/join")
        self.assertEqual(records[0]["category"], "account_registration")

    def test_svg_decimal_coordinates_are_not_ip_addresses(self):
        source = 'attrs:{d:"M716.3 92.4l.1.1.1.1c16 37.6 24.1 78"}'
        self.assertEqual(RECON.extract_sensitive("main.js", source), [])

    def test_real_ip_configuration_is_retained(self):
        source = 'const apiHost = "https://203.0.113.17:8443/v1";'
        records = RECON.extract_sensitive("config.js", source)
        ips = [item for item in records if item["type"] == "ip_address"]
        self.assertEqual([item["value"] for item in ips], ["203.0.113.17"])

    def test_contact_numbers_need_contact_context(self):
        self.assertEqual(RECON.extract_sensitive("main.js", 'const build = "13800138000";'), [])
        records = RECON.extract_sensitive("main.js", 'const mobile = "13800138000";')
        self.assertTrue(any(item["type"] == "cn_phone" for item in records))

    def test_crypto_signals_are_classified_locally(self):
        source = """
        const encoded = btoa(payload);
        const digest = CryptoJS.SHA256(payload);
        const sealed = CryptoJS.AES.encrypt(payload, key);
        const rsa = new JSEncrypt();
        const china = sm4.encrypt(payload, key);
        """
        records = RECON.extract_crypto_signals("main.js", source)
        self.assertEqual({item["algorithm"] for item in records}, {"Base64", "SHA-256", "AES", "RSA", "SM4"})
        self.assertTrue(all(item["localOnly"] for item in records))

    def test_ai_fallback_is_bounded_and_only_used_for_low_quality_framework_results(self):
        framework = {"framework": "React", "confidence": "high"}
        body = "const client=axios.create({baseURL:'/gateway'});" + (
            "fetch('/computed/'+tenantId); router.push('/admin'); upload(file);" * 500
        )
        fallback = RECON.ai_fallback_evidence(
            framework,
            [("https://app.example.test/main.12345678.js", body)],
            [],
            [],
        )
        self.assertTrue(fallback["enabled"])
        self.assertLessEqual(fallback["maxChars"], 12_000)
        self.assertTrue(fallback["snippets"])
        self.assertTrue(fallback["codeSlices"])
        self.assertEqual(fallback["maxCumulativeSliceChars"], fallback["maxChars"])
        self.assertTrue(all(item["kind"] == "marker-window" for item in fallback["codeSlices"]))
        self.assertLessEqual(len(fallback["codeSlices"]), 8)
        sufficient = [
            {"path": f"/api/item/{index}", "dynamic": False, "extractionEngine": "babel-ast"}
            for index in range(5)
        ]
        self.assertEqual(
            RECON.ai_fallback_evidence(framework, [("main.js", body)], sufficient, []),
            {},
        )
        self.assertEqual(
            RECON.ai_fallback_evidence({"framework": "Unknown"}, [("main.js", body)], [], []),
            {},
        )

    def test_ai_fallback_indexes_ast_slices_without_exposing_a_complete_bundle(self):
        context = "function loadTenant(){return fetch(base + '/tenant/' + id)}" * 300
        fallback = RECON.ai_fallback_evidence(
            {"framework": "Vue", "confidence": "high"},
            [("https://app.example.test/app.js", context)],
            [],
            [{"codeSlices": [{
                "id": "deadbeef12345678", "source": "https://app.example.test/app.js",
                "kind": "network-call", "marker": "fetch", "start": 100, "end": 12100,
                "focusStart": 5000, "context": context[:12000],
            }]}],
        )
        self.assertEqual(fallback["maxSliceReads"], 3)
        self.assertLessEqual(fallback["maxCumulativeSliceChars"], 72_000)
        self.assertEqual(fallback["snippets"][0]["sliceId"], "deadbeef12345678")
        self.assertLess(len(fallback["snippets"][0]["context"]), len(fallback["codeSlices"][0]["context"]))

    def test_compiled_parser_is_bundled_for_offline_runs(self):
        parser = HELPER.with_name("babel-parser.cjs")
        self.assertTrue(parser.is_file())
        self.assertGreater(parser.stat().st_size, 100_000)
        self.assertTrue(RUNTIME_HELPER.is_file())

    def test_runtime_probe_skips_locale_noise_and_captures_mutation_without_forwarding(self):
        fixture = (ROOT / "src-tauri/resources/workers/tests/fixtures/runtime_probe.html").read_bytes()
        received_posts = []

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path.startswith("/api/"):
                    body = b'{"ok":true}'
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                else:
                    body = fixture
                    self.send_response(200)
                    self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                received_posts.append(self.path)
                self.send_response(204)
                self.end_headers()

            def log_message(self, *_args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            target = f"http://127.0.0.1:{server.server_port}/"
            completed = subprocess.run(
                ["node", str(RUNTIME_HELPER)],
                input=json.dumps({
                    "url": target,
                    "timeoutMs": 12000,
                    "explorationTimeoutMs": 20000,
                    "maxActions": 12,
                    "maxStates": 6,
                    "maxDepth": 2,
                    "settleMs": 350,
                }),
                text=True,
                capture_output=True,
                timeout=35,
                check=True,
            )
            result = json.loads(completed.stdout)
            if not result.get("available"):
                self.skipTest("Chrome/Chromium is unavailable for the runtime probe")
            blocked = [item for item in result["blockedRequests"] if item["method"] == "POST"]
            self.assertTrue(any(item["url"].endswith("/api/v1/user/profile") for item in blocked))
            self.assertTrue(any(item.get("reason") == "capture_only_action_observed_without_forwarding" for item in blocked))
            self.assertEqual(received_posts, [], "capture-only controls must never forward the mutation")
            skipped_noise = (
                result["coverage"]["lowValueStateSkipped"]
                + result["coverage"]["deduplicatedStateCount"]
            )
            self.assertGreaterEqual(skipped_noise, 4)
            self.assertLessEqual(result["coverage"]["stateCount"], 2)
        finally:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
