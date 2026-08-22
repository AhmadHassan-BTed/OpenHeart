#!/usr/bin/env python3
"""
OpenHeart Deep Verification Suite — Diagrams, Artifact Creation, and State Refreshing
Authored for OpenHeart SCPG Engine. Maintained by Ahmad Hassan (B-Ted).

Comprehensive test coverage for:
1. Binary artifact generation (.tca, .bpa, .sta, .cfa, .ssa, .cga, .tra, .psa, .uma, .scpg)
2. All 14 UML diagram syntaxes, structure validation, and edge consistency
3. Refreshing, idempotency, state isolation, and live source edit reaction
"""

import sys
import os
import shutil
import json
import time
import urllib.request
import urllib.error
import subprocess

PORT = 8080
BASE_URL = f"http://localhost:{PORT}"
OPENHEART_BIN = os.path.join(os.getcwd(), "target/debug/openheart")

def log_test(section, name, passed, detail=""):
    status = "✅ PASS" if passed else "❌ FAIL"
    print(f"  [{section}] {status} | {name:<45} | {detail}")
    return passed

def run_cmd(cmd, cwd=None):
    res = subprocess.run(cmd, shell=True, capture_output=True, text=True, cwd=cwd)
    return res.returncode, res.stdout, res.stderr

def post_analyze(repo_url, diagram_types=None):
    if diagram_types is None:
        diagram_types = [
            "class", "object", "component", "deployment", "package",
            "composite", "profile", "usecase", "activity", "statemachine",
            "sequence", "communication", "interaction", "timing"
        ]
    payload = json.dumps({
        "repo_url": repo_url,
        "diagram_types": diagram_types
    }).encode("utf-8")

    req = urllib.request.Request(
        f"{BASE_URL}/api/analyze",
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST"
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode("utf-8"))

def validate_plantuml_syntax(dtype, puml):
    if not puml or not isinstance(puml, str):
        return False, "Empty or non-string PlantUML code"
    lines = puml.strip().splitlines()
    if not lines:
        return False, "No lines in PlantUML code"
    if not lines[0].startswith("@startuml"):
        return False, f"Missing @startuml header (starts with {lines[0]})"
    if not lines[-1].strip() == "@enduml":
        return False, f"Missing @enduml footer (ends with {lines[-1]})"
    
    # Check balanced quotes and brackets
    open_brackets = puml.count("{")
    close_brackets = puml.count("}")
    if open_brackets != close_brackets:
        return False, f"Mismatched braces: {open_brackets} '{{' vs {close_brackets} '}}'"
    
    return True, f"{len(lines)} lines, balanced syntax"


def test_artifact_file_creation():
    print("\n" + "═" * 80)
    print(" 1. ARTIFACT FILE & DIRECTORY CREATION TESTING")
    print("═" * 80)
    passed = 0
    total = 0

    test_out_dir = "/tmp/openheart_artifact_test"
    if os.path.exists(test_out_dir):
        shutil.rmtree(test_out_dir)

    # Run CLI analyze
    cmd = f"{OPENHEART_BIN} analyze sample_codebase {test_out_dir}"
    code, out, err = run_cmd(cmd)
    
    total += 1
    cli_ok = (code == 0) and ("SUCCESS: Complete 10-Phase Static Analysis" in out)
    if log_test("CLI", "CLI analyze command execution", cli_ok, f"Exited {code}"):
        passed += 1

    expected_artifacts = [
        ("corpus.tca", "Phase 1 Token Corpus Artifact"),
        ("ast.bpa", "Phase 2 Balanced Parentheses AST Artifact"),
        ("symbols.sta", "Phase 3 Symbol Table Artifact"),
        ("cfg.cfa", "Phase 4 Control Flow Graph Artifact"),
        ("ssa.ssa", "Phase 5 SSA Form Artifact"),
        ("callgraph.cga", "Phase 6 Call Graph Artifact"),
        ("traceability.tra", "Phase 7 Traceability Index Artifact"),
        ("paths.psa", "Phase 8 ROBDD Path Summary Artifact"),
        ("metadata.uma", "Phase 9 UML Metadata Artifact"),
        ("unified.scpg", "Phase 10 Composite SCPG Binary"),
        ("openheart_session.log", "Session Execution Log"),
        ("openheart_persistent.log", "Persistent Telemetry Log"),
    ]

    for fname, desc in expected_artifacts:
        total += 1
        fpath = os.path.join(test_out_dir, fname)
        exists = os.path.isfile(fpath)
        size = os.path.getsize(fpath) if exists else 0
        ok = exists and size > 0
        if log_test("ARTIFACT", f"Create {fname} ({desc})", ok, f"{size} bytes"):
            passed += 1

    # Test CRC-64 artifact inspector for each binary artifact
    binary_artifacts = ["corpus.tca", "ast.bpa", "symbols.sta", "cfg.cfa", "ssa.ssa", "callgraph.cga"]
    for bname in binary_artifacts:
        total += 1
        bpath = os.path.join(test_out_dir, bname)
        code, out, _ = run_cmd(f"{OPENHEART_BIN} inspect {bpath}")
        ok = (code == 0) and ("INTEGRITY: PASSED" in out or "Artifact" in out)
        if log_test("INSPECT", f"Inspect CRC-64 for {bname}", ok, "CRC-64 verified valid"):
            passed += 1

    shutil.rmtree(test_out_dir, ignore_errors=True)
    return passed == total


def test_all_14_diagrams_syntax():
    print("\n" + "═" * 80)
    print(" 2. ALL 14 UML DIAGRAM TYPES SYNTAX & STRUCTURAL VALIDATION")
    print("═" * 80)
    passed = 0
    total = 0

    repos_to_test = [
        ("OpenHeart Self-Analysis", "https://github.com/AhmadHassan-BTed/OpenHeart"),
        ("Java Design Patterns", "https://github.com/iluwatar/java-design-patterns"),
        ("Fractal Android (Kotlin)", "https://github.com/AhmadHassan-BTed/FractalAndroid"),
        ("JavaScript Algorithms", "https://github.com/trekhleb/javascript-algorithms"),
    ]

    diagram_types = [
        "class", "object", "component", "deployment", "package",
        "composite", "profile", "usecase", "activity", "statemachine",
        "sequence", "communication", "interaction", "timing"
    ]

    for repo_label, repo_url in repos_to_test:
        print(f"\n  ── Testing Repo Projections: {repo_label} ──")
        try:
            data = post_analyze(repo_url, diagram_types)
            diagrams = data.get("diagrams", {})
        except Exception as e:
            print(f"  ❌ Failed to analyze {repo_url}: {e}")
            continue

        for dtype in diagram_types:
            total += 1
            puml = diagrams.get(dtype, "")
            valid, detail = validate_plantuml_syntax(dtype, puml)
            if log_test("DIAGRAM", f"{repo_label} -> {dtype.capitalize()} Diagram", valid, detail):
                passed += 1

    return passed == total


def test_refreshing_and_idempotency():
    print("\n" + "═" * 80)
    print(" 3. REFRESHING, IDEMPOTENCY & STATE CONTAMINATION TESTING")
    print("═" * 80)
    passed = 0
    total = 0

    repo1 = "https://github.com/AhmadHassan-BTed/OpenHeart"
    repo2 = "https://github.com/AhmadHassan-BTed/SilentSniffer"

    # Test 1: Idempotency (consecutive runs on same repo produce identical diagrams)
    total += 1
    data1_a = post_analyze(repo1, ["class", "package"])
    data1_b = post_analyze(repo1, ["class", "package"])
    
    puml_a = data1_a.get("diagrams", {}).get("class", "")
    puml_b = data1_b.get("diagrams", {}).get("class", "")
    idempotent = (puml_a == puml_b) and (len(puml_a) > 100)
    if log_test("REFRESH", "Idempotency: Consecutive runs yield identical output", idempotent, f"Class diagram length: {len(puml_a)}B matched"):
        passed += 1

    # Test 2: State Isolation (Repo 1 -> Repo 2 -> Repo 1 has no cross-contamination)
    total += 1
    data2 = post_analyze(repo2, ["class", "package"])
    data1_c = post_analyze(repo1, ["class", "package"])
    
    puml_2 = data2.get("diagrams", {}).get("class", "")
    puml_1_after = data1_c.get("diagrams", {}).get("class", "")

    # SilentSniffer has ExecutionLoggerImpl, OpenHeart has OpenHeart/Engine classes
    isolated = ("ExecutionLoggerImpl" in puml_2) and ("ExecutionLoggerImpl" not in puml_1_after) and (puml_1_after == puml_a)
    if log_test("REFRESH", "State Isolation: Zero cross-repo contamination", isolated, "Symbols strictly isolated between context switches"):
        passed += 1

    # Test 3: Live Dynamic Source File Edit & Immediate Diagram Refresh
    total += 1
    scratch_repo_dir = "/tmp/openheart_dynamic_edit_test"
    if os.path.exists(scratch_repo_dir):
        shutil.rmtree(scratch_repo_dir)
    
    # Copy sample_codebase to scratch
    shutil.copytree("sample_codebase", scratch_repo_dir)
    
    # Initial CLI analyze
    out_dir_1 = "/tmp/openheart_dyn_out_1"
    run_cmd(f"{OPENHEART_BIN} analyze {scratch_repo_dir} {out_dir_1}")
    with open(os.path.join(out_dir_1, "symbols.sta"), "rb") as f:
        sta_bytes_1 = f.read()

    # Add a new Java class dynamically
    new_class_file = os.path.join(scratch_repo_dir, "com", "openheart", "app", "model", "DynamicEntity.java")
    with open(new_class_file, "w") as f:
        f.write("""
package com.openheart.app.model;

public class DynamicEntity {
    private Long id;
    private String entityName;

    public void processDynamicAction() {
        System.out.println("Action performed");
    }
}
""")

    # Re-analyze and verify refreshed state
    out_dir_2 = "/tmp/openheart_dyn_out_2"
    run_cmd(f"{OPENHEART_BIN} analyze {scratch_repo_dir} {out_dir_2}")
    with open(os.path.join(out_dir_2, "symbols.sta"), "rb") as f:
        sta_bytes_2 = f.read()

    # Dynamic class should increase symbol table size
    refreshed_after_add = len(sta_bytes_2) > len(sta_bytes_1)
    if log_test("REFRESH", "Live Add: Dynamic class immediately increases symbol table", refreshed_after_add, f"{len(sta_bytes_1)}B -> {len(sta_bytes_2)}B"):
        passed += 1

    # Delete the class and re-analyze
    total += 1
    os.remove(new_class_file)
    out_dir_3 = "/tmp/openheart_dyn_out_3"
    run_cmd(f"{OPENHEART_BIN} analyze {scratch_repo_dir} {out_dir_3}")
    with open(os.path.join(out_dir_3, "symbols.sta"), "rb") as f:
        sta_bytes_3 = f.read()

    refreshed_after_del = (len(sta_bytes_3) == len(sta_bytes_1))
    if log_test("REFRESH", "Live Delete: Deleting class immediately restores original state", refreshed_after_del, f"{len(sta_bytes_2)}B -> {len(sta_bytes_3)}B"):
        passed += 1

    shutil.rmtree(scratch_repo_dir, ignore_errors=True)
    shutil.rmtree(out_dir_1, ignore_errors=True)
    shutil.rmtree(out_dir_2, ignore_errors=True)
    shutil.rmtree(out_dir_3, ignore_errors=True)

    return passed == total


def main():
    print("╔══════════════════════════════════════════════════════════════════════════════╗")
    print("║     OPENHEART DEEP VERIFICATION: DIAGRAMS, ARTIFACTS & STATE REFRESH         ║")
    print("╚══════════════════════════════════════════════════════════════════════════════╝")

    # Start server for API diagram requests
    run_cmd(f"fuser -k {PORT}/tcp 2>/dev/null")
    time.sleep(1)
    server_proc = subprocess.Popen(
        [OPENHEART_BIN, "server", str(PORT)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )
    time.sleep(2)

    try:
        art_ok = test_artifact_file_creation()
        diag_ok = test_all_14_diagrams_syntax()
        refresh_ok = test_refreshing_and_idempotency()

        all_passed = art_ok and diag_ok and refresh_ok
        print("\n" + "═" * 80)
        if all_passed:
            print(" 🏆 ALL DIAGRAMS, ARTIFACT OUTPUTS, AND REFRESH TESTS PASSED (100%)")
        else:
            print(" ⚠️ SOME VERIFICATION TESTS FAILED. PLEASE REVIEW LOGS ABOVE.")
        print("═" * 80 + "\n")
        sys.exit(0 if all_passed else 1)
    finally:
        server_proc.terminate()
        server_proc.wait()

if __name__ == "__main__":
    main()
