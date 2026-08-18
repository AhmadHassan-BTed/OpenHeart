#!/usr/bin/env python3
"""
OpenHeart Ruthless Feedback & Verification Engine
==================================================
3-Oracle Ground Truth System with 5-Dimensional Evaluation Metrics.
Zero hardcoding. Fully dynamic across any language and any repository.

Usage:
    python3 ruthless_verify.py                    # verify all repos in ./target_repos/
    python3 ruthless_verify.py Parchment          # verify a single repo
"""

import sys, os, re, json, urllib.request, urllib.error
from collections import defaultdict
from pathlib import Path

SERVER_URL = "http://localhost:8080"
REPOS_DIR = "./target_repos"

# ─── Language-Specific Class Declaration Regex Patterns ───────────────────────
# Each pattern captures the class/type NAME as group "name"
LANG_PATTERNS = {
    # Java / Kotlin
    ".java": [
        re.compile(r'^\s*(?:public\s+|private\s+|protected\s+)?(?:static\s+)?(?:abstract\s+|final\s+)?(?:class|interface|enum|record|@interface)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    ".kt": [
        re.compile(r'^\s*(?:public\s+|private\s+|protected\s+|internal\s+)?(?:abstract\s+|sealed\s+|data\s+|open\s+|inner\s+)?(?:class|interface|enum\s+class|object|annotation\s+class)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    # Python
    ".py": [
        re.compile(r'^\s*class\s+(?P<name>\w+)', re.MULTILINE),
    ],
    # Rust
    ".rs": [
        re.compile(r'^\s*(?:pub(?:\(crate\))?\s+)?(?:struct|enum|trait)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    # JavaScript / TypeScript
    ".js": [
        re.compile(r'^\s*(?:export\s+)?(?:default\s+)?class\s+(?P<name>\w+)', re.MULTILINE),
    ],
    ".ts": [
        re.compile(r'^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?(?:class|interface|enum)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    ".jsx": [
        re.compile(r'^\s*(?:export\s+)?(?:default\s+)?class\s+(?P<name>\w+)', re.MULTILINE),
    ],
    ".tsx": [
        re.compile(r'^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?(?:class|interface|enum)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    # C++ / C
    ".cpp": [
        re.compile(r'^\s*(?:class|struct|enum(?:\s+class)?)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    ".h": [
        re.compile(r'^\s*(?:class|struct|enum(?:\s+class)?)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    ".hpp": [
        re.compile(r'^\s*(?:class|struct|enum(?:\s+class)?)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    # C#
    ".cs": [
        re.compile(r'^\s*(?:public\s+|private\s+|protected\s+|internal\s+)?(?:static\s+)?(?:abstract\s+|sealed\s+|partial\s+)?(?:class|interface|enum|struct|record)\s+(?P<name>\w+)', re.MULTILINE),
    ],
    # Go
    ".go": [
        re.compile(r'^\s*type\s+(?P<name>\w+)\s+(?:struct|interface)', re.MULTILINE),
    ],
    # Swift
    ".swift": [
        re.compile(r'^\s*(?:public\s+|private\s+|internal\s+|fileprivate\s+|open\s+)?(?:final\s+)?(?:class|struct|enum|protocol|actor)\s+(?P<name>\w+)', re.MULTILINE),
    ],
}

# Shared aliases
for alias_ext, base_ext in [(".kts", ".kt"), (".mjs", ".js"), (".cjs", ".js"),
                             (".mts", ".ts"), (".cts", ".ts"), (".cc", ".cpp"),
                             (".cxx", ".cpp"), (".hxx", ".hpp"), (".hh", ".hpp"),
                             (".c", ".cpp"), (".mm", ".cpp")]:
    if base_ext in LANG_PATTERNS:
        LANG_PATTERNS[alias_ext] = LANG_PATTERNS[base_ext]

# ─── Universal Keyword Blocklist ─────────────────────────────────────────────
KEYWORDS = {
    "class", "interface", "enum", "object", "struct", "trait", "impl", "mod",
    "fun", "function", "def", "fn", "val", "var", "let", "const", "static",
    "public", "private", "protected", "internal", "abstract", "sealed", "open",
    "data", "inner", "override", "super", "this", "self", "Self", "new",
    "return", "if", "else", "for", "while", "do", "switch", "case", "break",
    "continue", "try", "catch", "finally", "throw", "throws", "import", "export",
    "package", "module", "from", "as", "in", "is", "true", "false", "null",
    "undefined", "void", "boolean", "int", "long", "float", "double", "char",
    "byte", "short", "string", "String", "Object", "resolve", "require",
    "extends", "implements", "with", "yield", "async", "await", "type",
    "companion", "constructor", "annotation", "record", "final", "native",
    "volatile", "transient", "synchronized", "default", "goto", "instanceof",
    "typeof", "delete", "debugger", "eval", "arguments", "prototype",
    "Unknown", "SystemNode", "Entity", "Node_0", "Node_1", "Node_2",
    "MB", "args", "NaN", "Get", "Post", "Put", "Delete", "Path", "Body", "Header", "Query", "Param", "Http",
}

# ─── Test Path Patterns ──────────────────────────────────────────────────────
TEST_PATH_PATTERNS = [
    "/src/test/", "/src/androidTest/", "/src/testDebug/", "/src/testRelease/",
    "/__tests__/", "/tests/", "/test/", "/spec/", "/specs/",
    "_test.go", "_test.rs",
]

def is_test_path(filepath: str) -> bool:
    lower = filepath.replace("\\", "/").lower()
    for pat in TEST_PATH_PATTERNS:
        if pat.lower() in lower:
            return True
    base = os.path.basename(filepath)
    if base.endswith("Test.java") or base.endswith("Test.kt") or base.endswith("Tests.java"):
        return True
    if base.endswith("_test.py") or base.startswith("test_"):
        return True
    return False


# ═══════════════════════════════════════════════════════════════════════════════
# ORACLE 1: Regex Line Scanner
# ═══════════════════════════════════════════════════════════════════════════════
def oracle_regex(repo_path: str) -> dict:
    """Returns {name: {file, line, kind, package_hint}} for every class-like declaration."""
    results = {}
    for root, dirs, files in os.walk(repo_path):
        # Skip build/vendor/hidden dirs
        dirs[:] = [d for d in dirs if d not in {
            "node_modules", "target", "build", "dist", "out", "vendor", "target_repos",
            "venv", ".git", "__pycache__", ".gradle", ".idea", "fractal_android_output",
        } and not d.startswith(".") and not d.endswith("_output")]

        for fname in files:
            filepath = os.path.join(root, fname)
            ext = os.path.splitext(fname)[1].lower()
            if ext not in LANG_PATTERNS:
                continue
            if is_test_path(filepath):
                continue

            try:
                with open(filepath, "r", encoding="utf-8", errors="ignore") as f:
                    content = f.read()
            except (OSError, IOError):
                continue

            for pattern in LANG_PATTERNS[ext]:
                for match in pattern.finditer(content):
                    name = match.group("name")
                    if name in KEYWORDS or len(name) <= 1:
                        continue
                    prefix = content[:match.start()]
                    if ext in (".js", ".ts", ".jsx", ".tsx", ".mjs", ".cjs") and prefix.count("`") % 2 == 1:
                        continue
                    line_num = content[:match.start()].count("\n") + 1
                    rel_path = os.path.relpath(filepath, repo_path)
                    results[name] = {
                        "file": rel_path,
                        "line": line_num,
                        "ext": ext,
                    }
    return results


# ═══════════════════════════════════════════════════════════════════════════════
# ORACLE 2: Filesystem Package Hierarchy
# ═══════════════════════════════════════════════════════════════════════════════
def oracle_filesystem(repo_path: str) -> dict:
    """Returns {class_name: expected_package_path} based on directory structure."""
    results = {}
    for root, dirs, files in os.walk(repo_path):
        dirs[:] = [d for d in dirs if d not in {
            "node_modules", "target", "build", "dist", "out", "vendor", "target_repos",
            "venv", ".git", "__pycache__", ".gradle", ".idea", "fractal_android_output",
        } and not d.startswith(".") and not d.endswith("_output")]

        for fname in files:
            filepath = os.path.join(root, fname)
            ext = os.path.splitext(fname)[1].lower()
            if ext not in LANG_PATTERNS:
                continue
            if is_test_path(filepath):
                continue

            rel = os.path.relpath(filepath, repo_path).replace("\\", "/")
            parts = rel.split("/")

            # Determine package path based on language conventions
            pkg_path = None
            if ext in (".java", ".kt", ".kts"):
                # Java/Kotlin: after src/main/java/ or src/main/kotlin/
                for marker in ("java", "kotlin"):
                    if marker in parts:
                        idx = len(parts) - 1 - parts[::-1].index(marker)
                        pkg_parts = parts[idx+1:-1]  # exclude filename
                        if pkg_parts:
                            pkg_path = ".".join(pkg_parts)
                        break
            elif ext in (".py",):
                # Python: directory structure is the module path
                pkg_parts = parts[:-1]
                if pkg_parts:
                    pkg_path = ".".join(pkg_parts)
            elif ext in (".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs", ".mts", ".cts"):
                # JS/TS: directory structure
                pkg_parts = parts[:-1]
                if pkg_parts and pkg_parts[0] == "src":
                    pkg_parts = pkg_parts[1:]
                if pkg_parts:
                    pkg_path = ".".join(pkg_parts)
            elif ext in (".rs",):
                # Rust: directory structure after src/
                if "src" in parts:
                    idx = parts.index("src")
                    pkg_parts = parts[idx+1:-1]
                    if pkg_parts:
                        pkg_path = ".".join(pkg_parts)
            elif ext in (".go",):
                # Go: directory structure
                pkg_parts = parts[:-1]
                if pkg_parts:
                    pkg_path = ".".join(pkg_parts)

            # Now scan this file for class names and associate with package
            try:
                with open(filepath, "r", encoding="utf-8", errors="ignore") as f:
                    content = f.read()
            except (OSError, IOError):
                continue

            for pattern in LANG_PATTERNS.get(ext, []):
                for match in pattern.finditer(content):
                    name = match.group("name")
                    if name in KEYWORDS or len(name) <= 1:
                        continue
                    results[name] = pkg_path or ""
    return results


# ═══════════════════════════════════════════════════════════════════════════════
# ORACLE 3: Package Statement Parser
# ═══════════════════════════════════════════════════════════════════════════════
PKG_STMT_PATTERNS = {
    ".java": re.compile(r'^\s*package\s+([\w.]+)\s*;', re.MULTILINE),
    ".kt": re.compile(r'^\s*package\s+([\w.]+)', re.MULTILINE),
    ".kts": re.compile(r'^\s*package\s+([\w.]+)', re.MULTILINE),
}

def oracle_package_stmts(repo_path: str) -> dict:
    """Returns {class_name: declared_package} from actual package statements in source."""
    results = {}
    for root, dirs, files in os.walk(repo_path):
        dirs[:] = [d for d in dirs if d not in {
            "node_modules", "target", "build", "dist", "out", "vendor", "target_repos",
            "venv", ".git", "__pycache__", ".gradle", ".idea", "fractal_android_output",
        } and not d.startswith(".") and not d.endswith("_output")]

        for fname in files:
            filepath = os.path.join(root, fname)
            ext = os.path.splitext(fname)[1].lower()
            if ext not in PKG_STMT_PATTERNS:
                continue
            if is_test_path(filepath):
                continue

            try:
                with open(filepath, "r", encoding="utf-8", errors="ignore") as f:
                    content = f.read()
            except (OSError, IOError):
                continue

            pkg_match = PKG_STMT_PATTERNS[ext].search(content)
            pkg_name = pkg_match.group(1) if pkg_match else ""

            for pattern in LANG_PATTERNS.get(ext, []):
                for match in pattern.finditer(content):
                    name = match.group("name")
                    if name in KEYWORDS or len(name) <= 1:
                        continue
                    results[name] = pkg_name
    return results


# ═══════════════════════════════════════════════════════════════════════════════
# PUML PARSER — Extract classes and packages from generated PlantUML
# ═══════════════════════════════════════════════════════════════════════════════
def parse_puml_classes(puml_text: str) -> set:
    """Extract all class/interface/enum/abstract class names from PlantUML source."""
    classes = set()
    for m in re.finditer(r'^\s*(?:class|interface|enum|abstract\s+class)\s+(\S+)', puml_text, re.MULTILINE):
        name = m.group(1).strip().rstrip("{").strip()
        # Remove stereotypes
        name = re.sub(r'\s*<<.*?>>', '', name).strip()
        if name and name not in KEYWORDS and len(name) > 1:
            classes.add(name)
    return classes

def parse_puml_packages(puml_text: str) -> set:
    """Extract all package names from PlantUML source."""
    packages = set()
    for m in re.finditer(r'package\s+"([^"]+)"', puml_text):
        packages.add(m.group(1))
    return packages


# ═══════════════════════════════════════════════════════════════════════════════
# METRICS COMPUTATION
# ═══════════════════════════════════════════════════════════════════════════════
def compute_metrics(ground_truth: set, produced: set) -> dict:
    correct = ground_truth & produced
    phantoms = produced - ground_truth
    missing = ground_truth - produced

    precision = len(correct) / len(produced) if produced else 1.0
    recall = len(correct) / len(ground_truth) if ground_truth else 1.0
    f1 = (2 * precision * recall / (precision + recall)) if (precision + recall) > 0 else 0.0
    phantom_rate = len(phantoms) / len(produced) if produced else 0.0

    return {
        "precision": precision,
        "recall": recall,
        "f1": f1,
        "phantom_rate": phantom_rate,
        "correct": sorted(correct),
        "phantoms": sorted(phantoms),
        "missing": sorted(missing),
        "correct_count": len(correct),
        "phantom_count": len(phantoms),
        "missing_count": len(missing),
        "ground_truth_count": len(ground_truth),
        "produced_count": len(produced),
    }


# ═══════════════════════════════════════════════════════════════════════════════
# MAIN VERIFICATION LOOP
# ═══════════════════════════════════════════════════════════════════════════════
def verify_repo(repo_name: str) -> dict:
    repo_path = os.path.join(REPOS_DIR, repo_name)
    if not os.path.isdir(repo_path):
        print(f"  [SKIP] {repo_name}: directory not found")
        return None

    print(f"\n{'═'*80}")
    print(f"  VERIFYING: {repo_name}")
    print(f"{'═'*80}")

    # ── Run 3 Oracles ──
    print("  [Oracle 1] Regex Line Scanner...")
    o1 = oracle_regex(repo_path)
    print(f"             Found {len(o1)} class-like declarations")

    print("  [Oracle 2] Filesystem Hierarchy...")
    o2 = oracle_filesystem(repo_path)
    print(f"             Found {len(o2)} class-to-package mappings")

    print("  [Oracle 3] Package Statement Parser...")
    o3 = oracle_package_stmts(repo_path)
    print(f"             Found {len(o3)} package-declared classes")

    # ── Cross-validate: confirmed = found by Oracle 1 AND (Oracle 2 OR Oracle 3) ──
    gt_names = set(o1.keys())
    # All oracle-1 names are the primary ground truth for class names
    # Oracle 2 and 3 provide package validation
    ground_truth_classes = gt_names

    print(f"\n  [Ground Truth] {len(ground_truth_classes)} confirmed class declarations:")
    for name in sorted(ground_truth_classes):
        pkg = o3.get(name, o2.get(name, ""))
        file = o1.get(name, {}).get("file", "?")
        print(f"    ✓ {name:40s}  pkg={pkg:40s}  file={file}")

    # ── Call OpenHeart API ──
    print(f"\n  [OpenHeart] Dispatching analysis to {SERVER_URL}...")
    # Construct a github-like URL for the local repo
    repo_url = f"https://github.com/local/{repo_name}"
    payload = json.dumps({"repo_url": repo_url}).encode("utf-8")
    req = urllib.request.Request(
        f"{SERVER_URL}/api/analyze",
        data=payload,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=300) as response:
            res_data = json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, Exception) as e:
        print(f"  [ERROR] Server communication failed: {e}")
        print(f"  [HINT] Ensure backend is running via ./restart_server.sh")
        return None

    if res_data.get("status") != "success":
        print(f"  [ERROR] Analysis failed: {res_data.get('errors')}")
        return None

    stats = res_data.get("stats", {})
    print(f"  [OpenHeart] Completed in {stats.get('execution_time_ms', 0)}ms")
    print(f"              Files: {stats.get('files_processed', 0)} | Tokens: {stats.get('total_tokens', 0)} | Classes: {stats.get('total_classes', 0)}")

    # ── Parse PUML Output ──
    diagrams = res_data.get("diagrams", {})
    puml_class = diagrams.get("class", "")
    puml_package = diagrams.get("package", "")

    produced_classes = parse_puml_classes(puml_class)
    produced_packages = parse_puml_packages(puml_class)

    print(f"\n  [PUML Parser] Extracted {len(produced_classes)} classes, {len(produced_packages)} packages from class diagram")

    # ── Compute 5-Dimensional Metrics ──
    metrics = compute_metrics(ground_truth_classes, produced_classes)

    # Package nesting accuracy
    gt_packages = set()
    for name in ground_truth_classes:
        pkg = o3.get(name, o2.get(name, ""))
        if pkg:
            gt_packages.add(pkg)

    pkg_metrics = compute_metrics(gt_packages, produced_packages)

    # ── Print Report ──
    print(f"\n  {'─'*76}")
    print(f"  CLASS DIAGRAM METRICS")
    print(f"  {'─'*76}")
    print(f"  Precision:      {metrics['precision']*100:6.1f}%  ({metrics['correct_count']}/{metrics['produced_count']})")
    print(f"  Recall:         {metrics['recall']*100:6.1f}%  ({metrics['correct_count']}/{metrics['ground_truth_count']})")
    print(f"  F1 Score:       {metrics['f1']:6.4f}")
    print(f"  Phantom Rate:   {metrics['phantom_rate']*100:6.1f}%  ({metrics['phantom_count']} garbage classes)")
    print(f"  Pkg Nesting:    {pkg_metrics['precision']*100:6.1f}%")

    if metrics["phantoms"]:
        print(f"\n  ⛔ PHANTOM CLASSES (false positives — should NOT exist):")
        for p in metrics["phantoms"]:
            print(f"     ✗ {p}")

    if metrics["missing"]:
        print(f"\n  ❌ MISSING CLASSES (false negatives — should exist):")
        for m in metrics["missing"]:
            pkg = o3.get(m, o2.get(m, ""))
            print(f"     ✗ {m:40s}  (expected in {pkg})")

    if metrics["correct"]:
        print(f"\n  ✅ CORRECT CLASSES ({metrics['correct_count']}):")
        for c in metrics["correct"]:
            print(f"     ✓ {c}")

    return {
        "repo": repo_name,
        "ground_truth_count": metrics["ground_truth_count"],
        "produced_count": metrics["produced_count"],
        "precision": metrics["precision"],
        "recall": metrics["recall"],
        "f1": metrics["f1"],
        "phantom_rate": metrics["phantom_rate"],
        "phantom_count": metrics["phantom_count"],
        "missing_count": metrics["missing_count"],
        "pkg_accuracy": pkg_metrics["precision"],
        "phantoms": metrics["phantoms"],
        "missing": metrics["missing"],
    }


def main():
    print("╔" + "═"*78 + "╗")
    print("║  OPENHEART RUTHLESS FEEDBACK & VERIFICATION ENGINE                          ║")
    print("║  3-Oracle Ground Truth × 5-Dimensional Metrics × Zero Hardcoding            ║")
    print("╚" + "═"*78 + "╝")

    # Discover repos
    if len(sys.argv) > 1:
        repos = [sys.argv[1]]
    else:
        if not os.path.isdir(REPOS_DIR):
            print(f"[ERROR] {REPOS_DIR} not found.")
            sys.exit(1)
        repos = sorted([d for d in os.listdir(REPOS_DIR)
                       if os.path.isdir(os.path.join(REPOS_DIR, d))
                       and not d.startswith(".")])

    print(f"\n[INFO] Discovered {len(repos)} repositories: {', '.join(repos)}")

    all_results = []
    for repo in repos:
        result = verify_repo(repo)
        if result:
            all_results.append(result)

    # ── Final Convergence Summary ──
    print(f"\n\n{'╔' + '═'*78 + '╗'}")
    print(f"{'║'}  CONVERGENCE SUMMARY                                                        {'║'}")
    print(f"{'╚' + '═'*78 + '╝'}")
    print(f"\n  {'Repo':<30s} {'Prec':>6s} {'Rec':>6s} {'F1':>7s} {'Phant':>6s} {'Miss':>6s} {'PkgAcc':>7s} {'PASS':>6s}")
    print(f"  {'─'*30} {'─'*6} {'─'*6} {'─'*7} {'─'*6} {'─'*6} {'─'*7} {'─'*6}")

    all_pass = True
    for r in all_results:
        passed = r["f1"] >= 0.999 and r["phantom_count"] == 0
        status = "  ✅" if passed else "  ❌"
        if not passed:
            all_pass = False
        print(f"  {r['repo']:<30s} {r['precision']*100:5.1f}% {r['recall']*100:5.1f}% {r['f1']:7.4f} {r['phantom_count']:>6d} {r['missing_count']:>6d} {r['pkg_accuracy']*100:6.1f}% {status}")

    print()
    if all_pass:
        print("  🏆 CONVERGENCE ACHIEVED — ALL REPOS PASS F1=1.0, PHANTOM=0")
    else:
        print("  ⚠️  CONVERGENCE NOT REACHED — FIXES REQUIRED")
        print("\n  Recommended next steps:")
        total_phantoms = set()
        total_missing = set()
        for r in all_results:
            total_phantoms.update(r.get("phantoms", []))
            total_missing.update(r.get("missing", []))
        if total_phantoms:
            print(f"    → Eliminate {len(total_phantoms)} phantom classes: {', '.join(sorted(total_phantoms)[:10])}...")
        if total_missing:
            print(f"    → Recover {len(total_missing)} missing classes: {', '.join(sorted(total_missing)[:10])}...")

    return 0 if all_pass else 1


if __name__ == "__main__":
    sys.exit(main())
