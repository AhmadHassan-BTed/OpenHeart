#!/usr/bin/env python3
import sys
import os
import urllib.request
import urllib.parse
import json

DEFAULT_SERVER_URL = "http://localhost:8080"
OUTPUT_BASE_DIR = "./output_diagrams"

def main():
    print("==============================================================")
    print("      OpenHeart Autonomous SCPG PlantUML Diagram Generator   ")
    print("==============================================================")

    if len(sys.argv) > 1:
        repo_url = sys.argv[1].strip()
    else:
        repo_url = input("\nEnter GitHub Repository URL: ").strip()

    if not repo_url:
        print("[ERROR] No repository URL provided. Exiting.")
        sys.exit(1)

    repo_name = repo_url.rstrip("/").split("/")[-1].replace(".git", "")
    output_dir = os.path.join(OUTPUT_BASE_DIR, repo_name)
    os.makedirs(output_dir, exist_ok=True)

    print(f"\n[INFO] Target Repository: {repo_url}")
    print(f"[INFO] Output Directory:  {os.path.abspath(output_dir)}\n")

    payload = json.dumps({"repo_url": repo_url}).encode("utf-8")
    req = urllib.request.Request(
        f"{DEFAULT_SERVER_URL}/api/analyze",
        data=payload,
        headers={"Content-Type": "application/json"}
    )

    print("> Dispatching analysis request to OpenHeart Rust Engine...")
    try:
        with urllib.request.urlopen(req, timeout=300) as response:
            res_data = json.loads(response.read().decode("utf-8"))
    except Exception as e:
        print(f"[ERROR] Failed to communicate with OpenHeart server on {DEFAULT_SERVER_URL}: {e}")
        print("[HINT] Ensure the backend server is running via `./restart_server.sh`.")
        sys.exit(1)

    status = res_data.get("status")
    if status != "success":
        print(f"[ERROR] Analysis failed: {res_data.get('errors')}")
        sys.exit(1)

    stats = res_data.get("stats", {})
    print(f"[SUCCESS] Ingestion & Analysis Completed in {stats.get('execution_time_ms', 0)} ms!")
    print(f"          - Files Processed: {stats.get('files_processed', 0)}")
    print(f"          - Total Tokens:    {stats.get('total_tokens', 0)}")
    print(f"          - Total Classes:   {stats.get('total_classes', 0)}\n")

    diagrams = res_data.get("diagrams", {})
    saved_count = 0

    cfg_path = os.path.join(os.path.dirname(__file__), "ruthless_config.json")
    if os.path.exists(cfg_path):
        with open(cfg_path, "r", encoding="utf-8") as f:
            diagram_mapping = json.load(f).get("diagram_mapping", [])
    else:
        diagram_mapping = []

    print("==============================================================")
    print("               Generated PlantUML Diagram Files               ")
    print("==============================================================")

    for diag_key, filename in diagram_mapping:
        puml_content = diagrams.get(diag_key, "")
        if puml_content:
            file_path = os.path.join(output_dir, filename)
            with open(file_path, "w", encoding="utf-8") as f:
                f.write(puml_content)
            file_size = os.path.getsize(file_path)
            print(f"  ✔ {filename:<36} ({file_size} bytes)")
            saved_count += 1

    print("==============================================================")
    print(f"[COMPLETE] Saved {saved_count} PlantUML diagram files into:")
    print(f"           {os.path.abspath(output_dir)}")
    print("==============================================================")

if __name__ == "__main__":
    main()
