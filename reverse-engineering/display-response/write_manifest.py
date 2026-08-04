#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("experiment")
    parser.add_argument("--firmware", required=True)
    arguments = parser.parse_args()
    experiment = Path(arguments.experiment).resolve()
    files = [
        path
        for path in experiment.rglob("*")
        if path.is_file() and path.name != "manifest.json"
    ]
    manifest = {
        "schema": 1,
        "experiment": experiment.name,
        "firmware": arguments.firmware,
        "files": [
            {
                "path": str(path.relative_to(experiment)),
                "bytes": path.stat().st_size,
                "sha256": sha256(path),
            }
            for path in sorted(files)
        ],
    }
    destination = experiment / "manifest.json"
    destination.write_text(json.dumps(manifest, indent=2) + "\n")
    print(destination)


if __name__ == "__main__":
    main()
