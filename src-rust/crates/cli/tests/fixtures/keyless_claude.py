#!/usr/bin/env python3
"""Record process context for synthetic keyless Claude conversations."""
import json
import os
from pathlib import Path
import sys

root = Path(os.environ["COVEN_CODE_HOME"])
request = json.loads(sys.stdin.readline())
text = request["message"]["content"][-1]["text"]
label = text.strip().splitlines()[-1].split()[-1]
resume = sys.argv[sys.argv.index("--resume") + 1] if "--resume" in sys.argv else None
with (root / "launches").open("a") as log:
    print(json.dumps([label, os.getcwd(), resume]), file=log)
if resume and label == "A3":
    sys.exit(1)
print(json.dumps({"type": "system", "subtype": "init", "session_id": resume or "fixture-" + label}))
print(json.dumps({"type": "assistant", "message": {"content": [{"type": "text", "text": "ok"}]}}))
print(json.dumps({"type": "result", "subtype": "success", "result": "ok"}))
