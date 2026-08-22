#!/usr/bin/env python3
"""
OpenHeart Rigorous 14 UML Diagram Full-Spectrum Verification
Authored for OpenHeart SCPG Engine. Maintained by Ahmad Hassan (B-Ted).

Tests ALL 14 UML Diagram Types across multiple real-world repositories:
1. Class Diagram
2. Object Diagram
3. Component Diagram
4. Deployment Diagram
5. Package Diagram
6. Composite Structure Diagram
7. Profile Diagram
8. Use Case Diagram
9. Activity Diagram
10. State Machine Diagram
11. Sequence Diagram
12. Communication Diagram
13. Interaction Overview Diagram
14. Timing Diagram
"""

import os
import sys
import json
import time
import subprocess
import urllib.request

PORT = 8085
BASE_URL = f"http://localhost:{PORT}"
OPENHEART_BIN = os.path.join(os.getcwd(), "target/debug/openheart")

ALL_14_DIAGRAMS = [
    "class", "object", "component", "deployment", "package",
    "composite", "profile", "usecase", "activity", "statemachine",
    "sequence", "communication", "interaction", "timing"
]

def run_cmd(cmd, cwd=None):
    res = subprocess.run(cmd, shell=True, capture_output=True, text=True, cwd=cwd)
    return res.returncode, res.stdout, res.stderr

def post_analyze(repo_url, diagram_types):
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

def validate_diagram_syntax(diag_type, puml_text):
    errors = []
    lines = [l.strip() for l in puml_text.splitlines() if l.strip()]

    if not lines:
        return False, ["Empty diagram output"]

    if not lines[0].startswith("@startuml"):
        errors.append(f"Missing @startuml header: '{lines[0]}'")
    if not lines[-1].startswith("@enduml"):
        errors.append(f"Missing @enduml footer: '{lines[-1]}'")

    # Check balanced braces
    open_curly = puml_text.count("{")
    close_curly = puml_text.count("}")
    if open_curly != close_curly:
        errors.append(f"Unbalanced braces: {open_curly} open vs {close_curly} close")

    # Type-specific semantic assertions
    if diag_type == "class":
        if not any("class " in l or "interface " in l or "enum " in l for l in lines):
            errors.append("No class/interface/enum definitions found in class diagram")
    elif diag_type == "object":
        if not any("object " in l for l in lines):
            errors.append("No object instances found in object diagram")
    elif diag_type == "component":
        if not any("[" in l and "]" in l for l in lines):
            errors.append("No component blocks found in component diagram")
    elif diag_type == "deployment":
        if not any("node " in l or "artifact " in l for l in lines):
            errors.append("No deployment nodes/artifacts found")
    elif diag_type == "package":
        if not any("package " in l for l in lines):
            errors.append("No package structures found in package diagram")
    elif diag_type == "composite":
        if not any("<<composite>>" in l or "port " in l for l in lines):
            errors.append("No composite structures or ports found")
    elif diag_type == "profile":
        if not any("stereotype " in l or "<<metaclass>>" in l for l in lines):
            errors.append("No profile stereotypes found")
    elif diag_type == "usecase":
        if not any("actor " in l or "usecase " in l for l in lines):
            errors.append("No actors or use cases found")
    elif diag_type == "activity":
        if not any(":" in l and ";" in l for l in lines):
            errors.append("No activity action nodes found")
    elif diag_type == "statemachine":
        if not any("-->" in l for l in lines):
            errors.append("No state transitions found")
    elif diag_type == "sequence":
        if not any("participant " in l or "->" in l for l in lines):
            errors.append("No sequence participants/messages found")
    elif diag_type == "communication":
        if not any("object " in l or "obj_" in l or "--" in l for l in lines):
            errors.append("No communication links found")
    elif diag_type == "interaction":
        if not any("partition " in l or "Flow" in l or "Phase" in l for l in lines):
            errors.append("No interaction overview partitions found")
    elif diag_type == "timing":
        if not any("robust " in l or "@0" in l for l in lines):
            errors.append("No timing timelines found")

    return len(errors) == 0, errors

def test_repo_all_14_diagrams(repo_name, repo_url):
    print(f"\n────────────────────────────────────────────────────────────────────────────────")
    print(f" 📦 Testing All 14 Diagrams on Repository: {repo_name}")
    print(f"────────────────────────────────────────────────────────────────────────────────")

    t0 = time.perf_counter()
    data = post_analyze(repo_url, ALL_14_DIAGRAMS)
    elapsed = time.perf_counter() - t0

    if data.get("status") != "success":
        print(f"❌ Analysis failed: {data.get('error')}")
        return False

    diagrams = data.get("diagrams", {})
    all_ok = True

    for diag_type in ALL_14_DIAGRAMS:
        puml = diagrams.get(diag_type, "")
        line_count = len(puml.splitlines())
        valid, errors = validate_diagram_syntax(diag_type, puml)

        if valid:
            print(f"  [{diag_type.upper():<13}] ✅ PASS | {line_count:>4} lines | Syntax & Structure Verified")
        else:
            all_ok = False
            err_str = "; ".join(errors)
            print(f"  [{diag_type.upper():<13}] ❌ FAIL | {line_count:>4} lines | {err_str}")

    print(f"⏱️ 10-Phase Pipeline + 14 Diagrams Projected in {elapsed*1000.0:.2f} ms")
    return all_ok

def main():
    print("╔══════════════════════════════════════════════════════════════════════════════╗")
    print("║   OPENHEART ALL 14 UML DIAGRAMS FULL-SPECTRUM VERIFICATION                  ║")
    print("║   Grounded, High-Fidelity Projections Across All Standard UML Diagram Types   ║")
    print("╚══════════════════════════════════════════════════════════════════════════════╝")

    run_cmd(f"fuser -k {PORT}/tcp 2>/dev/null")
    time.sleep(1)

    server_proc = subprocess.Popen(
        [OPENHEART_BIN, "server", str(PORT)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )
    time.sleep(2)

    try:
        repos_to_test = [
            ("Authored GoF Design Patterns", "https://github.com/patterns/authored-design-patterns"),
            ("Spring PetClinic (Enterprise Domain)", "https://github.com/spring-projects/spring-petclinic"),
            ("Abstract Factory (Java Design Patterns)", "https://github.com/iluwatar/java-design-patterns/abstract-factory"),
            ("Observer Pattern (Java Design Patterns)", "https://github.com/iluwatar/java-design-patterns/observer"),
            ("Strategy Pattern (Java Design Patterns)", "https://github.com/iluwatar/java-design-patterns/strategy"),
        ]

        total_tested = 0
        total_passed = 0

        for name, url in repos_to_test:
            ok = test_repo_all_14_diagrams(name, url)
            total_tested += 1
            if ok:
                total_passed += 1

        print("\n" + "═" * 80)
        print(" 📊 14 UML DIAGRAMS VERIFICATION SUMMARY")
        print("═" * 80)
        print(f" • Total Repositories Tested        : {total_tested}")
        print(f" • Total Diagram Projections Checked: {total_tested * 14} (14 per repository)")
        print(f" • Perfect 14-Diagram Pass Rate     : {total_passed} / {total_tested} ({total_passed/total_tested*100:.1f}%)")
        print("═" * 80 + "\n")

        if total_passed == total_tested:
            print(" 🏆 ALL 14 UML DIAGRAM TYPES FULLY VERIFIED WITH 100% SUCCESS ACROSS ALL REPOSITORIES!")
            sys.exit(0)
        else:
            print(" ⚠️ Some diagram checks failed. Review output above.")
            sys.exit(1)

    finally:
        server_proc.terminate()
        server_proc.wait()

if __name__ == "__main__":
    main()
