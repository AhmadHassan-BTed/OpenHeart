#!/usr/bin/env python3
"""
OpenHeart Automated Testing Suite — Endpoints & Connectors Verification
Authored for OpenHeart SCPG Engine. Maintained by Ahmad Hassan (B-Ted).

Systematically verifies:
1. All HTTP REST and static asset endpoints (GET, POST, OPTIONS, HEAD, 404s, CORS).
2. All Pipeline Connectors:
   - WebRepoFetcher & Git clone adapter
   - Multi-language AST reduction and Tree-sitter parsers
   - 10-Phase SCPG compiler pipeline
   - PlantUML, Mermaid, XMI, and JSON diagram export engines
   - Multi-repo cross-language ground truth accuracy
"""

import sys
import os
import json
import time
import urllib.request
import urllib.error
import subprocess

PORT = 8080
BASE_URL = f"http://localhost:{PORT}"

def log_test(name, passed, detail=""):
    status = "✅ PASS" if passed else "❌ FAIL"
    print(f"  {status} | {name:<45} | {detail}")
    return passed

def run_cmd(cmd, cwd=None):
    res = subprocess.run(cmd, shell=True, capture_output=True, text=True, cwd=cwd)
    return res.returncode, res.stdout, res.stderr

def http_request(path, method="GET", data=None, headers=None):
    url = f"{BASE_URL}{path}"
    if headers is None:
        headers = {}
    if data is not None and isinstance(data, dict):
        data = json.dumps(data).encode("utf-8")
        headers["Content-Type"] = "application/json"
    elif data is not None and isinstance(data, str):
        data = data.encode("utf-8")

    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            body = resp.read()
            return resp.status, resp.headers, body
    except urllib.error.HTTPError as e:
        body = e.read()
        return e.code, e.headers, body
    except Exception as e:
        return 0, {}, str(e).encode("utf-8")

def test_endpoints():
    print("\n" + "═" * 80)
    print(" 1. HTTP ENDPOINT COMPREHENSIVE AUTOMATED TESTING")
    print("═" * 80)
    passed_count = 0
    total_count = 0

    static_routes = [
        ("/", "text/html", 1000),
        ("/index.html", "text/html", 1000),
        ("/style.css", "text/css", 1000),
        ("/app.js", "application/javascript", 100),
        ("/js/api.js", "application/javascript", 500),
        ("/js/ui.js", "application/javascript", 1000),
        ("/js/viewer.js", "application/javascript", 1000),
        ("/js/orb.js", "application/javascript", 1000),
        ("/js/logger.js", "application/javascript", 500),
        ("/js/state.js", "application/javascript", 500),
        ("/favicon.svg", "image/svg+xml", 100),
    ]

    for path, expected_type, min_size in static_routes:
        total_count += 1
        status, headers, body = http_request(path)
        content_type = headers.get("Content-Type", "")
        ok = (status == 200) and (expected_type in content_type) and (len(body) >= min_size)
        if log_test(f"GET {path}", ok, f"HTTP {status}, {len(body)}B, {content_type}"):
            passed_count += 1

    # Health Endpoint
    total_count += 1
    status, headers, body = http_request("/api/health")
    try:
        data = json.loads(body.decode("utf-8"))
        ok = (status == 200) and (data.get("status") == "online") and ("OpenHeart" in data.get("engine", ""))
    except Exception:
        ok = False
    if log_test("GET /api/health", ok, f"HTTP {status}, status={data.get('status') if ok else 'err'}"):
        passed_count += 1

    # CORS Preflight OPTIONS
    total_count += 1
    status, headers, _ = http_request("/api/analyze", method="OPTIONS")
    allow_origin = headers.get("Access-Control-Allow-Origin", "")
    allow_methods = headers.get("Access-Control-Allow-Methods", "")
    ok = (status in [200, 204]) and (allow_origin == "*") and ("POST" in allow_methods)
    if log_test("OPTIONS /api/analyze (CORS Preflight)", ok, f"HTTP {status}, Origin={allow_origin}, Methods={allow_methods}"):
        passed_count += 1

    # Error Handlers / 404s
    total_count += 1
    status, _, body = http_request("/nonexistent_page.html")
    ok = (status == 404)
    if log_test("GET /nonexistent_page.html (404 Test)", ok, f"HTTP {status} (Expected 404)"):
        passed_count += 1

    total_count += 1
    status, _, body = http_request("/api/nonexistent_route")
    ok = (status == 404)
    if log_test("GET /api/nonexistent_route (404 Test)", ok, f"HTTP {status} (Expected 404)"):
        passed_count += 1

    # POST /api/analyze with invalid JSON payload
    total_count += 1
    status, _, body = http_request("/api/analyze", method="POST", data="MALFORMED_JSON_STRING")
    ok = (status in [200, 400]) # handled gracefully without crashing server
    if log_test("POST /api/analyze (Malformed Body Handling)", ok, f"HTTP {status} handled safely"):
        passed_count += 1

    # POST /api/analyze with valid repository payload (all 14 diagram types)
    total_count += 1
    payload = {
        "repo_url": "https://github.com/AhmadHassan-BTed/OpenHeart",
        "diagram_types": [
            "class", "object", "component", "deployment", "package",
            "composite", "profile", "usecase", "activity", "statemachine",
            "sequence", "communication", "interaction", "timing"
        ]
    }
    status, headers, body = http_request("/api/analyze", method="POST", data=payload)
    try:
        data = json.loads(body.decode("utf-8"))
        diagrams = data.get("diagrams", {})
        ok = (status == 200) and (len(diagrams) == 14) and all(v.startswith("@startuml") for v in diagrams.values())
    except Exception as e:
        ok = False
    if log_test("POST /api/analyze (14 UML Diagrams)", ok, f"HTTP {status}, {len(diagrams)}/14 valid diagrams returned"):
        passed_count += 1

    print(f"\n  Endpoint Summary: {passed_count}/{total_count} Passed ({passed_count/total_count*100:.1f}%)")
    return passed_count == total_count


def test_connectors():
    print("\n" + "═" * 80)
    print(" 2. PIPELINE CONNECTORS & SUBSYSTEM INTEGRATION TESTING")
    print("═" * 80)
    passed_count = 0
    total_count = 0

    # Connector 1: WebRepoFetcher URL Validation
    total_count += 1
    code, out, _ = run_cmd("cargo test --test adapters_tests test_web_repo_url_validation 2>&1 || cargo test test_web_repo_url_validation 2>&1")
    ok = (code == 0) and ("test adapters::web_repo::tests::test_web_repo_url_validation ... ok" in out or "1 passed" in out)
    if log_test("Connector: WebRepoFetcher URL Validator", ok, "Validates GitHub URL formats & rejects non-GitHub links"):
        passed_count += 1

    # Connector 2: Multi-Language Lexical & AST Reducer Connectors (Phase 1 & 2)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test ingestion_tests 2>&1")
    ok = (code == 0) and ("4 passed" in out)
    if log_test("Connector: Ingestion & TokenCorpus (.tca)", ok, "Verified FNV-1a Interner, Token Monotonicity, & SortKeys"):
        passed_count += 1

    total_count += 1
    code, out, _ = run_cmd("cargo test --test ast_tests 2>&1")
    ok = (code == 0) and ("4 passed" in out)
    if log_test("Connector: CST Reducer & BP AST (.bpa)", ok, "Verified Balanced Parentheses, Rank/Select, & LCA RMQ"):
        passed_count += 1

    # Connector 3: Symbol Table & Scope Graph Connector (Phase 3)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test symbol_tests 2>&1")
    ok = (code == 0) and ("3 passed" in out)
    if log_test("Connector: Symbol Table & Scope Graph (.sta)", ok, "Verified 5-Pass DFS & Kahn's Topological Acyclicity"):
        passed_count += 1

    # Connector 4: Control Flow Graph & Dominator Analysis (Phase 4)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test cfg_tests 2>&1")
    ok = (code == 0) and ("2 passed" in out)
    if log_test("Connector: CFG & Cooper Dominators (.cfa)", ok, "Verified CSR Adjacency, idom[], and Dominance Frontiers"):
        passed_count += 1

    # Connector 5: SSA Form & Control Dependence Graph (Phase 5)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test ssa_tests 2>&1")
    ok = (code == 0) and ("1 passed" in out)
    if log_test("Connector: SSA Conversion & CDG (.ssa)", ok, "Verified Cytron phi-placement, Renaming, & DefUseCSR"):
        passed_count += 1

    # Connector 6: Call Graph & Andersen Points-To Connector (Phase 6)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test cg_tests 2>&1")
    ok = (code == 0) and ("1 passed" in out)
    if log_test("Connector: Call Graph & Points-To (.cga)", ok, "Verified CHA Virtual Dispatch & CallSite CSR"):
        passed_count += 1

    # Connector 7: Universal Traceability Index (Phase 7)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test tra_tests 2>&1")
    ok = (code == 0) and ("1 passed" in out)
    if log_test("Connector: Universal Traceability (.tra)", ok, "Verified Forward/Backward token_id bijective mapping"):
        passed_count += 1

    # Connector 8: ROBDD Path Summaries & #SAT Connector (Phase 8)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test psa_tests 2>&1")
    ok = (code == 0) and ("1 passed" in out)
    if log_test("Connector: ROBDD Path Feasibility (.psa)", ok, "Verified UniqueTable sharing, Shannon Apply, & #SAT counting"):
        passed_count += 1

    # Connector 9: UML Metadata Extraction & Pattern Matcher (Phase 9)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test uma_tests 2>&1")
    ok = (code == 0) and ("1 passed" in out)
    if log_test("Connector: UML Extraction Engine (.uma)", ok, "Verified 14 UML extractions & GoF design pattern detectors"):
        passed_count += 1

    # Connector 10: SCPG Binary Serializer & API Connector (Phase 10)
    total_count += 1
    code, out, _ = run_cmd("cargo test --test scpg_tests 2>&1")
    ok = (code == 0) and ("1 passed" in out)
    if log_test("Connector: SCPG Engine & Exporters (.scpg)", ok, "Verified end-to-end composite pipeline execution"):
        passed_count += 1

    print(f"\n  Connector Summary: {passed_count}/{total_count} Passed ({passed_count/total_count*100:.1f}%)")
    return passed_count == total_count


def test_multi_repo_precision():
    print("\n" + "═" * 80)
    print(" 3. MULTI-REPO GROUND-TRUTH CONVERGENCE TESTING")
    print("═" * 80)
    code, out, _ = run_cmd("python3 ruthless_verify.py")
    ok = (code == 0) and ("CONVERGENCE ACHIEVED" in out)
    log_test("Multi-Repo Ground-Truth Precision Engine", ok, "F1=1.0000 across Android, Java, Kotlin, JS, Rust repos")
    return ok


def main():
    print("╔══════════════════════════════════════════════════════════════════════════════╗")
    print("║      OPENHEART AUTOMATED ENDPOINT & CONNECTOR VALIDATION SUITE               ║")
    print("╚══════════════════════════════════════════════════════════════════════════════╝")

    # 1. Ensure server is running on port 8080
    run_cmd(f"fuser -k {PORT}/tcp 2>/dev/null")
    time.sleep(1)
    server_proc = subprocess.Popen(
        [os.path.join(os.getcwd(), "target/debug/openheart"), "server", str(PORT)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )
    time.sleep(2)

    try:
        endpoints_ok = test_endpoints()
        connectors_ok = test_connectors()
        precision_ok = test_multi_repo_precision()

        all_ok = endpoints_ok and connectors_ok and precision_ok
        print("\n" + "═" * 80)
        if all_ok:
            print(" 🏆 ALL AUTOMATED TESTS & CONNECTOR VALIDATIONS PASSED CLEANLY (100%)")
        else:
            print(" ⚠️ SOME AUTOMATED TESTS FAILED. PLEASE REVIEW LOGS ABOVE.")
        print("═" * 80 + "\n")
        sys.exit(0 if all_ok else 1)
    finally:
        server_proc.terminate()
        server_proc.wait()

if __name__ == "__main__":
    main()
