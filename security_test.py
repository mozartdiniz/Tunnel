#!/usr/bin/env python3
"""
Tunnel Security Test Suite
Runs from macOS against a machine running the Tunnel app.

Tests the four vulnerabilities from the original SECURITY_AUDIT.md against
the current implementation.

Usage:
    python3 security_test.py <TARGET_IP>

Example:
    python3 security_test.py 192.168.1.42

Requirements: Python 3.6+, no external libraries needed.
"""

import hashlib
import json
import ssl
import sys
import time
import urllib.error
import urllib.request
from typing import Optional

PORT = 53317
PREPARE_TIMEOUT   = 70   # seconds — user needs time to see and interact with the dialog
QUICK_TIMEOUT     = 5    # seconds — for requests that don't wait for user interaction


# ── Helpers ────────────────────────────────────────────────────────────────────

def ssl_ctx() -> ssl.SSLContext:
    """Ignore self-signed certificate — we're testing the app, not the cert."""
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    return ctx


def http_get(url: str) -> tuple[int, dict]:
    try:
        with urllib.request.urlopen(url, context=ssl_ctx(), timeout=QUICK_TIMEOUT) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, {}
    except Exception as e:
        return 0, {"error": str(e)}


def http_post_json(url: str, payload: dict, timeout: int = PREPARE_TIMEOUT) -> tuple[int, dict]:
    data = json.dumps(payload).encode()
    req = urllib.request.Request(
        url, data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, context=ssl_ctx(), timeout=timeout) as r:
            body = r.read()
            return r.status, json.loads(body) if body else {}
    except urllib.error.HTTPError as e:
        return e.code, {}
    except Exception as e:
        return 0, {"error": str(e)}


def http_post_raw(url: str, data: bytes, timeout: int = QUICK_TIMEOUT) -> int:
    req = urllib.request.Request(url, data=data, method="POST")
    try:
        with urllib.request.urlopen(req, context=ssl_ctx(), timeout=timeout) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code
    except Exception:
        return 0


def prepare_upload_body(
    alias: str,
    fingerprint: str,
    filename: str,
    size: int,
    sha256: Optional[str] = None,
) -> dict:
    return {
        "info": {
            "alias": alias,
            "version": "2.0",
            "deviceModel": "MacBook Pro",
            "deviceType": "laptop",
            "fingerprint": fingerprint,
            "port": PORT,
            "protocol": "https",
            "download": False,
        },
        "files": {
            "file-001": {
                "id": "file-001",
                "fileName": filename,
                "size": size,
                "fileType": "text/plain",
                "sha256": sha256,
            }
        },
    }


def result(label: str, passed: bool, notes: str = ""):
    icon = "✅ PASS" if passed else "❌ FAIL"
    print(f"  {icon}  {label}")
    if notes:
        for line in notes.splitlines():
            print(f"           {line}")


def section(title: str):
    print(f"\n{'─' * 56}")
    print(f"  {title}")
    print(f"{'─' * 56}")


def prompt(msg: str):
    print(f"\n  👉  {msg}")


# ── Tests ──────────────────────────────────────────────────────────────────────

def test_connectivity(base: str) -> bool:
    section("TEST 0: Connectivity")
    status, body = http_get(f"{base}/api/localsend/v2/info")
    if status == 200:
        result("Reached /api/localsend/v2/info", True,
               f"Device alias: {body.get('alias', '?')}")
        return True
    else:
        result("Reached /api/localsend/v2/info", False,
               f"HTTP {status} — is Tunnel running on that IP?")
        return False


def test_impersonation(prepare_url: str):
    section("TEST 1: Sender Impersonation")
    print("  Sends a transfer request with a fully spoofed sender name.")
    print("  The name in the dialog comes from the JSON body, not from TLS.")
    prompt("Watch the Tunnel app — a request from 'SECURITY_TEST_SPOOF' should appear.")
    prompt("Deny it after confirming whether that name is visible.")
    print()

    payload = prepare_upload_body(
        alias="SECURITY_TEST_SPOOF",
        fingerprint="deadbeefdeadbeefdeadbeefdeadbeef",
        filename="innocent.txt",
        size=5,
    )
    status, _ = http_post_json(prepare_url, payload, timeout=PREPARE_TIMEOUT)

    if status == 403:
        # User denied — dialog appeared with spoofed name
        result(
            "Sender alias is verified via TLS (not just JSON)",
            False,
            "The dialog appeared showing 'SECURITY_TEST_SPOOF' — a name we invented.\n"
            "The fingerprint shown is also from JSON, not from the TLS handshake.\n"
            "KNOWN LIMITATION: the LocalSend v2 protocol has no mutual TLS.\n"
            "Any device on the network can claim any identity.",
        )
    elif status == 200:
        result(
            "Spoofed transfer was accepted and processed",
            False,
            "You accepted the spoofed request — same identity issue as above.",
        )
    elif status == 0:
        print("  ⏱  Timed out — did the dialog appear? Run again and interact with it.")
    else:
        print(f"  Unexpected HTTP {status}.")


def test_file_size_limit(prepare_url: str, upload_url: str):
    section("TEST 2: File Size Limit (DoS protection)")
    print("  Declares a file of 10 GiB + 1 byte in the prepare-upload metadata.")
    print("  The upload handler should reject it with 413 Payload Too Large.")
    prompt("A transfer request will appear in Tunnel — ACCEPT it to proceed.")
    print()

    over_limit = 10 * 1024 * 1024 * 1024 + 1  # one byte over MAX_FILE_SIZE
    payload = prepare_upload_body(
        alias="SizeTestSender",
        fingerprint="aabbccddeeff00112233445566778899",
        filename="huge_file.bin",
        size=over_limit,
    )
    status, body = http_post_json(prepare_url, payload, timeout=PREPARE_TIMEOUT)

    if status == 403:
        print("  You denied the dialog — re-run and accept to complete this test.")
        return
    if status != 200:
        print(f"  Unexpected HTTP {status} from prepare-upload.")
        return

    session_id = body.get("sessionId", "")
    token = next(iter(body.get("files", {}).values()), "")
    if not session_id or not token:
        print("  Could not extract session_id / token from response.")
        return

    url = f"{upload_url}?sessionId={session_id}&fileId=file-001&token={token}"
    upload_status = http_post_raw(url, b"A" * 16)  # tiny body; server checks declared size in metadata

    result(
        "Upload rejected with 413 when declared size > MAX_FILE_SIZE (10 GiB)",
        upload_status == 413,
        f"Upload responded with HTTP {upload_status} (expected 413).",
    )


def test_path_traversal(prepare_url: str, upload_url: str):
    section("TEST 3: Path Traversal Filenames")
    print("  Sends files with dangerous names. Each requires you to accept a dialog.")
    print("  After accepting, check ~/Downloads (or your configured folder) to verify")
    print("  no dangerous filename was written to disk.\n")

    cases = [
        ("..",               "must be renamed to 'file'"),
        (".",                "must be renamed to 'file'"),
        ("../../etc/passwd", "must be sanitized to '.._.._etc_passwd'"),
        ("../secret.txt",    "must be sanitized to '.._secret.txt'"),
    ]

    for dangerous_name, expectation in cases:
        prompt(f"Accept the dialog for filename {repr(dangerous_name)} ({expectation})")
        payload = prepare_upload_body(
            alias="PathTestSender",
            fingerprint="112233445566778899aabbccddeeff00",
            filename=dangerous_name,
            size=5,
        )
        status, body = http_post_json(prepare_url, payload, timeout=PREPARE_TIMEOUT)

        if status == 403:
            print(f"  Denied — skipping {repr(dangerous_name)}")
            time.sleep(1)
            continue
        if status != 200:
            print(f"  HTTP {status} — unexpected. Skipping.")
            time.sleep(1)
            continue

        session_id = body.get("sessionId", "")
        token = next(iter(body.get("files", {}).values()), "")
        if not session_id or not token:
            print("  Could not extract session_id / token.")
            continue

        url = f"{upload_url}?sessionId={session_id}&fileId=file-001&token={token}"
        upload_status = http_post_raw(url, b"hello")
        result(
            f"Filename {repr(dangerous_name)} — upload accepted (check disk for safe name)",
            upload_status == 200,
            f"HTTP {upload_status}. {expectation.capitalize()}.",
        )
        time.sleep(1)


def test_checksum(prepare_url: str, upload_url: str):
    section("TEST 4: Checksum Verification")
    print("  Declares the SHA-256 of 'hello', then uploads 'WRONG' bytes.")
    print("  The server should reject the upload and discard the file.")
    prompt("Accept the dialog in Tunnel.")
    print()

    real_sha256 = hashlib.sha256(b"hello").hexdigest()  # correct hash for "hello"

    payload = prepare_upload_body(
        alias="ChecksumTestSender",
        fingerprint="ffeeddccbbaa99887766554433221100",
        filename="checksum_test.txt",
        size=5,
        sha256=real_sha256,
    )
    status, body = http_post_json(prepare_url, payload, timeout=PREPARE_TIMEOUT)

    if status == 403:
        print("  You denied the dialog — re-run and accept to complete this test.")
        return
    if status != 200:
        print(f"  Unexpected HTTP {status} from prepare-upload.")
        return

    session_id = body.get("sessionId", "")
    token = next(iter(body.get("files", {}).values()), "")
    if not session_id or not token:
        print("  Could not extract session_id / token.")
        return

    url = f"{upload_url}?sessionId={session_id}&fileId=file-001&token={token}"
    upload_status = http_post_raw(url, b"WRONG")  # bytes that don't match declared sha256

    result(
        "Server rejects upload with mismatched SHA-256",
        upload_status == 500,
        f"HTTP {upload_status} (expected 500). "
        "If passed: temp file was discarded, nothing written to disk.",
    )


def print_summary():
    section("EXPECTED RESULTS SUMMARY")
    print("""
  TEST 1 — Impersonation   ❌ KNOWN ISSUE
    Sender identity is self-reported in JSON. Any device on the network
    can claim any alias and fingerprint. This is a limitation of the
    LocalSend v2 protocol (no mutual TLS). The fingerprint shown in
    the UI is from req.info.fingerprint, not from the TLS handshake.

  TEST 2 — File size limit ✅ FIXED
    MAX_FILE_SIZE = 10 GiB enforced in the upload handler.
    upload.rs:51  `if file_meta.size > MAX_FILE_SIZE`

  TEST 3 — Path traversal  ✅ FIXED
    transfer/helpers.rs sanitize_filename():
      ".."  → "file"
      "."   → "file"
      "/"   → "_"  (and other separators)

  TEST 4 — Checksum        ✅ FIXED
    SHA-256 computed during streaming, verified before atomic rename.
    Temp file cleaned up on mismatch. upload.rs:132-145
""")


# ── Entry point ─────────────────────────────────────────────────────────────────

def main():
    if len(sys.argv) != 2:
        print(f"Usage: python3 {sys.argv[0]} <TARGET_IP>")
        print(f"       python3 {sys.argv[0]} 192.168.1.42")
        sys.exit(1)

    ip = sys.argv[1]
    base        = f"https://{ip}:{PORT}"
    prepare_url = f"{base}/api/localsend/v2/prepare-upload"
    upload_url  = f"{base}/api/localsend/v2/upload"

    print("\n  Tunnel Security Test Suite")
    print(f"  Target: {base}")
    print("  Make sure the Tunnel app is running and visible on the target machine.")

    if not test_connectivity(base):
        sys.exit(1)

    test_impersonation(prepare_url)
    test_file_size_limit(prepare_url, upload_url)
    test_path_traversal(prepare_url, upload_url)
    test_checksum(prepare_url, upload_url)
    print_summary()


if __name__ == "__main__":
    main()
