#!/usr/bin/env python3
"""
flatpak-cargo-generator.py — Generate Flatpak source entries for Cargo dependencies.

Reads Cargo.lock and produces a JSON array of Flatpak source objects that
flatpak-builder uses to download all crate archives before the offline build.

Each entry is either:
  • An archive source that extracts a .crate tarball into cargo/vendor/{name}-{version}/
  • An inline source that writes .cargo-checksum.json (Cargo's integrity marker)

Plus one inline source that writes $CARGO_HOME/config to redirect Cargo to the
vendored directory instead of the network registry.

Usage:
    python3 flatpak-cargo-generator.py [Cargo.lock] [-o cargo-sources.json]

Include the output in your Flatpak manifest:
    sources:
      - type: dir
        path: ..
      - cargo-sources.json   # flatpak-builder merges this JSON array automatically
"""

import argparse
import json
import sys
from pathlib import Path

CRATES_IO_BASE = "https://static.crates.io/crates"


# ---------------------------------------------------------------------------
# Cargo.lock parser (no external deps — works on any Python 3.8+)
# ---------------------------------------------------------------------------

def _parse_lockfile(text: str) -> list[dict[str, str]]:
    """Parse Cargo.lock into a list of [[package]] dicts.

    Handles both v2 (plain) and v3 (checksum-in-entry) formats.
    Skips workspace-path dependencies (no source or checksum).
    """
    packages: list[dict[str, str]] = []
    current: dict[str, str] = {}

    for raw_line in text.splitlines():
        line = raw_line.strip()
        if line == "[[package]]":
            if current:
                packages.append(current)
            current = {}
        elif line.startswith("#") or not line:
            continue
        elif "=" in line:
            key, _, val = line.partition("=")
            key = key.strip()
            val = val.strip()
            # Strip surrounding double-quotes (all Cargo.lock string values are quoted)
            if val.startswith('"') and val.endswith('"'):
                val = val[1:-1]
            current[key] = val

    if current:
        packages.append(current)

    return packages


# ---------------------------------------------------------------------------
# Source entry builders
# ---------------------------------------------------------------------------

def _cargo_config_source(module_name: str) -> dict:
    """Inline source that writes $CARGO_HOME/config.

    CARGO_HOME is set to /run/build/{module_name}/cargo in the manifest, so
    this file ends up at /run/build/{module_name}/cargo/config and tells Cargo
    to use the vendored directory instead of downloading from crates.io.
    """
    vendor_path = f"/run/build/{module_name}/cargo/vendor"
    config_toml = (
        "[source.crates-io]\n"
        'replace-with = "vendored-sources"\n'
        "\n"
        "[source.vendored-sources]\n"
        f'directory = "{vendor_path}"\n'
    )
    return {
        "type": "inline",
        "contents": config_toml,
        "dest": "cargo",
        "dest-filename": "config",
    }


def _crate_sources(name: str, version: str, checksum: str) -> list[dict]:
    """Archive + checksum sources for a single crates.io package.

    flatpak-builder extracts the .crate tarball (strip-components=1 by default),
    placing source files directly inside cargo/vendor/{name}-{version}/.

    The .cargo-checksum.json file satisfies Cargo's directory-source integrity
    check. We provide the package-level hash but leave `files` empty — Cargo
    only verifies files that are listed, so this is sufficient.
    """
    dest = f"cargo/vendor/{name}-{version}"
    url = f"{CRATES_IO_BASE}/{name}/{name}-{version}.crate"
    return [
        {
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": url,
            "sha256": checksum,
            "dest": dest,
        },
        {
            "type": "inline",
            "contents": json.dumps({"files": {}, "package": checksum}),
            "dest": dest,
            "dest-filename": ".cargo-checksum.json",
        },
    ]


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def generate_sources(lockfile: Path, module_name: str) -> list[dict]:
    packages = _parse_lockfile(lockfile.read_text(encoding="utf-8"))
    sources: list[dict] = [_cargo_config_source(module_name)]
    seen: set[tuple[str, str]] = set()

    for pkg in packages:
        name = pkg.get("name", "")
        version = pkg.get("version", "")
        checksum = pkg.get("checksum", "")
        source = pkg.get("source", "")

        # Skip workspace members and path dependencies (no source or checksum)
        if not source or not checksum:
            continue

        # Only handle crates.io registry packages
        if "registry+" not in source:
            # Git dependencies would need different handling; skip for now.
            continue

        key = (name, version)
        if key in seen:
            continue
        seen.add(key)

        sources.extend(_crate_sources(name, version, checksum))

    return sources


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "lockfile",
        nargs="?",
        default="Cargo.lock",
        help="Path to Cargo.lock (default: ./Cargo.lock)",
    )
    parser.add_argument(
        "-o", "--output",
        default="-",
        help="Output JSON file (default: stdout)",
    )
    parser.add_argument(
        "--module-name",
        default="tunnel",
        metavar="NAME",
        help=(
            "Flatpak module name — determines the /run/build/<NAME>/cargo path "
            "used in the generated CARGO_HOME config (default: tunnel)"
        ),
    )
    args = parser.parse_args()

    lockfile = Path(args.lockfile)
    if not lockfile.exists():
        print(f"error: {lockfile} not found", file=sys.stderr)
        sys.exit(1)

    sources = generate_sources(lockfile, args.module_name)
    output = json.dumps(sources, indent=2) + "\n"

    if args.output == "-":
        sys.stdout.write(output)
    else:
        out = Path(args.output)
        out.write_text(output, encoding="utf-8")
        n_crates = sum(1 for s in sources if s.get("type") == "archive")
        print(
            f"Wrote {n_crates} crate archives + {n_crates} checksums + 1 config "
            f"= {len(sources)} total sources → {out}",
            file=sys.stderr,
        )


if __name__ == "__main__":
    main()
