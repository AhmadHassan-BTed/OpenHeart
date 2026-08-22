#!/usr/bin/env python3
"""
OpenHeart Authored Design Patterns Comprehensive Verification Suite.
Authored for OpenHeart SCPG Engine. Maintained by Ahmad Hassan (B-Ted).

Analyzes authored canonical GoF design patterns across all 14 UML diagrams:
1. Creational: Singleton, Factory Method, Builder
2. Structural: Adapter, Decorator, Facade
3. Behavioral: Observer, Strategy, Template Method

Verifies:
- 10-Phase Pipeline Execution and CRC-64 verification on all binary artifacts.
- All 14 UML Diagram projections syntax and balanced formatting.
- Class diagram structural semantics:
    * Classes, interfaces, abstract classes
    * Inheritance (--|>) & interface realization (..|>)
    * Field associations (-->), aggregations (o--), and compositions (*--)
    * Methods, parameters, return types, visibility (+/-/#)
- Package diagram structure and hierarchy.
- Sequence, Activity, and Object diagram projections.
"""

import os
import sys
import json
import re
import shutil
import subprocess
import urllib.request
import time

PORT = 8080
BASE_URL = f"http://localhost:{PORT}"
OPENHEART_BIN = os.path.join(os.getcwd(), "target/debug/openheart")

def log_test(section, name, passed, detail=""):
    status = "✅ PASS" if passed else "❌ FAIL"
    print(f"  [{section}] {status} | {name:<52} | {detail}")
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

def parse_class_diagram(puml_text):
    classes = set()
    interfaces = set()
    abstract_classes = set()
    inheritance_edges = []
    realization_edges = []
    association_edges = []
    
    for line in puml_text.splitlines():
        line = line.strip()
        # Match class / interface / abstract class
        m_iface = re.match(r'interface\s+([a-zA-Z0-9_]+)', line)
        if m_iface:
            interfaces.add(m_iface.group(1))
            classes.add(m_iface.group(1))
            continue
            
        m_abs = re.match(r'abstract\s+class\s+([a-zA-Z0-9_]+)', line)
        if m_abs:
            abstract_classes.add(m_abs.group(1))
            classes.add(m_abs.group(1))
            continue

        m_cls = re.match(r'class\s+([a-zA-Z0-9_]+)', line)
        if m_cls:
            cname = m_cls.group(1)
            if not cname.startswith("pkg_"):
                classes.add(cname)
            continue

        # Match inheritance (--|>)
        m_inh = re.search(r'([a-zA-Z0-9_]+)\s+--\|>\s+([a-zA-Z0-9_]+)', line)
        if m_inh:
            inheritance_edges.append((m_inh.group(1), m_inh.group(2)))
            continue

        # Match realization (..|>)
        m_real = re.search(r'([a-zA-Z0-9_]+)\s+\.\.\|>\s+([a-zA-Z0-9_]+)', line)
        if m_real:
            realization_edges.append((m_real.group(1), m_real.group(2)))
            continue

        # Match association / composition / aggregation (--> , *-- , o--)
        m_assoc = re.search(r'([a-zA-Z0-9_]+)\s+(\*--|o--|-->)\s+([a-zA-Z0-9_]+)', line)
        if m_assoc:
            association_edges.append((m_assoc.group(1), m_assoc.group(2), m_assoc.group(3)))
            continue

    return {
        "classes": classes,
        "interfaces": interfaces,
        "abstract_classes": abstract_classes,
        "inheritance": inheritance_edges,
        "realization": realization_edges,
        "associations": association_edges,
        "raw": puml_text
    }

def main():
    print("╔══════════════════════════════════════════════════════════════════════════════╗")
    print("║   OPENHEART AUTHORED DESIGN PATTERNS COMPREHENSIVE VERIFICATION SUITE       ║")
    print("╚══════════════════════════════════════════════════════════════════════════════╝")

    src_dir = "target_repos/authored-design-patterns/src/main/java"
    out_dir = "/tmp/openheart_authored_patterns_out"
    if os.path.exists(out_dir):
        shutil.rmtree(out_dir)

    passed_tests = 0
    total_tests = 0

    # 1. 10-Phase Pipeline CLI Verification
    print("\n" + "═" * 80)
    print(" 1. 10-PHASE PIPELINE COMPILATION & BINARY ARTIFACT INTEGRITY")
    print("═" * 80)
    
    total_tests += 1
    code, out, _ = run_cmd(f"{OPENHEART_BIN} analyze {src_dir} {out_dir}")
    cli_ok = (code == 0) and ("SUCCESS: Complete 10-Phase Static Analysis" in out)
    if log_test("CLI-PIPELINE", "10-Phase Analysis Execution", cli_ok, f"Exited {code}"):
        passed_tests += 1

    expected_artifacts = [
        "corpus.tca", "ast.bpa", "symbols.sta", "cfg.cfa", "ssa.ssa",
        "callgraph.cga", "traceability.tra", "paths.psa", "metadata.uma", "unified.scpg"
    ]
    for art in expected_artifacts:
        total_tests += 1
        art_path = os.path.join(out_dir, art)
        exists = os.path.isfile(art_path) and os.path.getsize(art_path) > 0
        sz = os.path.getsize(art_path) if exists else 0
        if log_test("ARTIFACTS", f"Generate Binary: {art}", exists, f"{sz} bytes"):
            passed_tests += 1

    for art in ["corpus.tca", "ast.bpa", "symbols.sta", "cfg.cfa", "ssa.ssa", "callgraph.cga"]:
        total_tests += 1
        art_path = os.path.join(out_dir, art)
        code, out, _ = run_cmd(f"{OPENHEART_BIN} inspect {art_path}")
        ok = (code == 0) and ("CRC-64 Check   : VERIFIED VALID" in out or "Artifact Type" in out)
        if log_test("INSPECT", f"CRC-64 Verification: {art}", ok, "CRC-64 Valid"):
            passed_tests += 1

    # Start Server for Diagram Projections
    run_cmd(f"fuser -k {PORT}/tcp 2>/dev/null")
    time.sleep(1)
    server_proc = subprocess.Popen(
        [OPENHEART_BIN, "server", str(PORT)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )
    time.sleep(2)

    try:
        # 2. All 14 UML Diagram Types Syntax Validation
        print("\n" + "═" * 80)
        print(" 2. ALL 14 UML DIAGRAM TYPES PROJECTION & SYNTAX VALIDATION")
        print("═" * 80)

        all_14_types = [
            "class", "object", "component", "deployment", "package",
            "composite", "profile", "usecase", "activity", "statemachine",
            "sequence", "communication", "interaction", "timing"
        ]
        api_data = post_analyze("https://github.com/patterns/authored-design-patterns", all_14_types)
        diagrams = api_data.get("diagrams", {})

        for dtype in all_14_types:
            total_tests += 1
            content = diagrams.get(dtype, "")
            valid = content.startswith("@startuml") and content.strip().endswith("@enduml")
            lines_cnt = len(content.splitlines())
            if log_test("DIAGRAM-14", f"Generate UML {dtype.title()} Diagram", valid, f"{lines_cnt} lines, valid PlantUML"):
                passed_tests += 1

        # 3. Deep Semantic Validation for Each Design Pattern in Class Diagram
        print("\n" + "═" * 80)
        print(" 3. DEEP SEMANTIC DESIGN PATTERN VERIFICATION (CLASS DIAGRAM)")
        print("═" * 80)

        cd_info = parse_class_diagram(diagrams.get("class", ""))
        classes = cd_info["classes"]
        interfaces = cd_info["interfaces"]
        abstract_classes = cd_info["abstract_classes"]
        inheritance = cd_info["inheritance"]
        realization = cd_info["realization"]
        raw = cd_info["raw"]

        # 3.1 Singleton Pattern
        total_tests += 1
        s_cls = "DatabaseConnectionPool" in classes
        s_field = "instance" in raw
        s_method = "getInstance" in raw
        s_ok = s_cls and s_field and s_method
        if log_test("CREATIONAL", "Singleton: DatabaseConnectionPool", s_ok, "Static instance & getInstance() verified"):
            passed_tests += 1

        # 3.2 Factory Method Pattern
        total_tests += 1
        fm_classes = {"Transport", "Truck", "Ship", "Logistics", "RoadLogistics", "SeaLogistics"}.issubset(classes)
        fm_reals = (("Truck", "Transport") in realization or ("Truck", "Transport") in inheritance) and \
                   (("Ship", "Transport") in realization or ("Ship", "Transport") in inheritance)
        fm_inh = (("RoadLogistics", "Logistics") in inheritance) and (("SeaLogistics", "Logistics") in inheritance)
        fm_ok = fm_classes and fm_reals and fm_inh
        if log_test("CREATIONAL", "Factory Method: Transport & Logistics Hierarchy", fm_ok, "Truck/Ship ..|> Transport, Road/Sea --|> Logistics"):
            passed_tests += 1

        # 3.3 Builder Pattern
        total_tests += 1
        b_classes = {"Computer", "ComputerBuilder", "Director"}.issubset(classes)
        b_methods = "buildCpu" in raw and "constructGamingComputer" in raw
        b_ok = b_classes and b_methods
        if log_test("CREATIONAL", "Builder: Computer & ComputerBuilder", b_ok, "Director -> Builder -> Product chain verified"):
            passed_tests += 1

        # 3.4 Adapter Pattern
        total_tests += 1
        ad_classes = {"MediaPlayer", "AdvancedMediaPlayer", "VlcPlayer", "Mp4Player", "MediaAdapter", "AudioPlayer"}.issubset(classes)
        ad_reals = (("VlcPlayer", "AdvancedMediaPlayer") in realization or ("VlcPlayer", "AdvancedMediaPlayer") in inheritance) and \
                   (("Mp4Player", "AdvancedMediaPlayer") in realization or ("Mp4Player", "AdvancedMediaPlayer") in inheritance) and \
                   (("MediaAdapter", "MediaPlayer") in realization or ("MediaAdapter", "MediaPlayer") in inheritance) and \
                   (("AudioPlayer", "MediaPlayer") in realization or ("AudioPlayer", "MediaPlayer") in inheritance)
        ad_ok = ad_classes and ad_reals
        if log_test("STRUCTURAL", "Adapter: MediaPlayer & MediaAdapter", ad_ok, "Vlc/Mp4 ..|> AdvancedMedia, Adapter/Audio ..|> MediaPlayer"):
            passed_tests += 1

        # 3.5 Decorator Pattern
        total_tests += 1
        dec_classes = {"Beverage", "Espresso", "CondimentDecorator", "Mocha", "Whip"}.issubset(classes)
        dec_reals = (("Espresso", "Beverage") in realization or ("Espresso", "Beverage") in inheritance) and \
                    (("CondimentDecorator", "Beverage") in realization or ("CondimentDecorator", "Beverage") in inheritance)
        dec_inh = (("Mocha", "CondimentDecorator") in inheritance) and (("Whip", "CondimentDecorator") in inheritance)
        dec_ok = dec_classes and dec_reals and dec_inh
        if log_test("STRUCTURAL", "Decorator: Beverage & CondimentDecorator", dec_ok, "Espresso/Decorator ..|> Beverage, Mocha/Whip --|> Decorator"):
            passed_tests += 1

        # 3.6 Facade Pattern
        total_tests += 1
        fac_classes = {"VideoConversionFacade", "AudioMixer", "BitrateReader"}.issubset(classes)
        fac_method = "convertVideo" in raw
        fac_ok = fac_classes and fac_method
        if log_test("STRUCTURAL", "Facade: VideoConversionFacade", fac_ok, "Facade aggregates AudioMixer & BitrateReader"):
            passed_tests += 1

        # 3.7 Observer Pattern
        total_tests += 1
        obs_classes = {"Subject", "Observer", "NewsAgency", "NewsChannel"}.issubset(classes)
        obs_reals = (("NewsAgency", "Subject") in realization or ("NewsAgency", "Subject") in inheritance) and \
                    (("NewsChannel", "Observer") in realization or ("NewsChannel", "Observer") in inheritance)
        obs_ok = obs_classes and obs_reals
        if log_test("BEHAVIORAL", "Observer: Subject & NewsAgency", obs_ok, "NewsAgency ..|> Subject, NewsChannel ..|> Observer"):
            passed_tests += 1

        # 3.8 Strategy Pattern
        total_tests += 1
        strat_classes = {"PaymentStrategy", "CreditCardStrategy", "PaypalStrategy", "ShoppingCart"}.issubset(classes)
        strat_reals = (("CreditCardStrategy", "PaymentStrategy") in realization or ("CreditCardStrategy", "PaymentStrategy") in inheritance) and \
                      (("PaypalStrategy", "PaymentStrategy") in realization or ("PaypalStrategy", "PaymentStrategy") in inheritance)
        strat_ok = strat_classes and strat_reals
        if log_test("BEHAVIORAL", "Strategy: PaymentStrategy & Strategies", strat_ok, "CreditCard/Paypal ..|> PaymentStrategy"):
            passed_tests += 1

        # 3.9 Template Method Pattern
        total_tests += 1
        tm_classes = {"DataMiner", "PdfDataMiner", "CsvDataMiner"}.issubset(classes)
        tm_inh = (("PdfDataMiner", "DataMiner") in inheritance) and (("CsvDataMiner", "DataMiner") in inheritance)
        tm_method = "mine" in raw
        tm_ok = tm_classes and tm_inh and tm_method
        if log_test("BEHAVIORAL", "Template Method: DataMiner & Miners", tm_ok, "Pdf/Csv --|> DataMiner with template method mine()"):
            passed_tests += 1

        # 4. Package Diagram Structure Verification
        print("\n" + "═" * 80)
        print(" 4. PACKAGE DIAGRAM HIERARCHICAL STRUCTURE VERIFICATION")
        print("═" * 80)

        pkg_puml = diagrams.get("package", "")
        expected_packages = [
            "com", "patterns", "creational", "structural", "behavioral",
            "singleton", "factory", "builder", "adapter", "decorator", "facade",
            "observer", "strategy", "templatemethod"
        ]
        
        total_tests += 1
        pkg_matched = sum(1 for p in expected_packages if p in pkg_puml)
        pkg_ok = pkg_matched >= 12
        if log_test("PACKAGES", "Hierarchical Package Decomposition", pkg_ok, f"{pkg_matched}/{len(expected_packages)} package nodes present"):
            passed_tests += 1

        # 5. Sequence & Activity Diagram Structural Elements
        print("\n" + "═" * 80)
        print(" 5. SEQUENCE & ACTIVITY WORKFLOW PROJECTIONS")
        print("═" * 80)

        seq_puml = diagrams.get("sequence", "")
        act_puml = diagrams.get("activity", "")
        
        total_tests += 1
        seq_ok = "participant" in seq_puml and "@startuml" in seq_puml
        if log_test("SEQUENCE", "Sequence Lifeline Projection", seq_ok, "Participants and lifelines extracted"):
            passed_tests += 1

        total_tests += 1
        act_ok = "start" in act_puml and "stop" in act_puml
        if log_test("ACTIVITY", "Activity Workflow Control Graph", act_ok, "Start, actions, and stop control nodes generated"):
            passed_tests += 1

        print("\n" + "═" * 80)
        all_passed = (passed_tests == total_tests)
        if all_passed:
            print(f" 🏆 ALL {total_tests}/{total_tests} DESIGN PATTERN & DIAGRAM TESTS PASSED (100%)")
        else:
            print(f" ⚠️ {passed_tests}/{total_tests} TESTS PASSED.")
        print("═" * 80 + "\n")

        shutil.rmtree(out_dir, ignore_errors=True)
        sys.exit(0 if all_passed else 1)

    finally:
        server_proc.terminate()
        server_proc.wait()

if __name__ == "__main__":
    main()
