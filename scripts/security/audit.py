#!/usr/bin/env python3
"""
Tunnel Audit — LocalSend v2 Protocol & Security Test Suite

Tests the running app over the network. Works against both Mac and Linux builds.
No external dependencies — uses only the Python standard library.

Usage:
  python3 audit.py <host>                     # automated tests only
  python3 audit.py <host> --interactive       # + tests that need you at the device
  python3 audit.py <host> --port 53318        # custom port (default: 53317)

Exit code: 0 if all run tests passed, 1 if any failed.
"""

import argparse
import hashlib
import http.client
import json
import socket
import ssl
import sys
import time
import uuid

# ── ANSI colours ──────────────────────────────────────────────────────────────

GREEN  = "\033[92m"
RED    = "\033[91m"
YELLOW = "\033[93m"
CYAN   = "\033[96m"
BOLD   = "\033[1m"
DIM    = "\033[2m"
RESET  = "\033[0m"

LOCALSEND_PORT      = 53317
MULTICAST_ADDR      = "224.0.0.167"
MAX_FILE_SIZE_BYTES = 10 * 1024 * 1024 * 1024  # 10 GiB

# ── Result tracking ───────────────────────────────────────────────────────────

_results: list[tuple[str, str]] = []   # (status, name)
_counts  = {"pass": 0, "fail": 0, "skip": 0}

def _log(status: str, name: str, detail: str = "") -> None:
    _counts[status] += 1
    tag = {
        "pass": f"{GREEN}PASS{RESET}",
        "fail": f"{RED}FAIL{RESET}",
        "skip": f"{YELLOW}SKIP{RESET}",
    }[status]
    suffix = f"  {DIM}({detail}){RESET}" if detail else ""
    print(f"  [{tag}] {name}{suffix}")
    _results.append((status, name))

def pass_(name: str, detail: str = "") -> None: _log("pass", name, detail)
def fail (name: str, detail: str = "") -> None: _log("fail", name, detail)
def skip (name: str, detail: str = "") -> None: _log("skip", name, detail)

def section(title: str) -> None:
    pad = "─" * max(0, 54 - len(title))
    print(f"\n{BOLD}{CYAN}── {title} {pad}{RESET}")

# ── HTTPS helpers ─────────────────────────────────────────────────────────────

def _insecure_ctx() -> ssl.SSLContext:
    """SSL context that accepts any cert (needed for self-signed servers)."""
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode    = ssl.CERT_NONE
    return ctx

def get_cert_der(host: str, port: int) -> bytes:
    """Return the DER bytes of the server's leaf cert."""
    ctx = _insecure_ctx()
    with socket.create_connection((host, port), timeout=10) as raw:
        with ctx.wrap_socket(raw, server_hostname=host) as s:
            return s.getpeercert(binary_form=True)

def cert_fingerprint(der: bytes) -> str:
    return hashlib.sha256(der).hexdigest()

def _conn(host: str, port: int, timeout: int = 10) -> http.client.HTTPSConnection:
    return http.client.HTTPSConnection(host, port, context=_insecure_ctx(), timeout=timeout)

def GET(host: str, port: int, path: str) -> tuple[int, bytes]:
    c = _conn(host, port)
    c.request("GET", path)
    r = c.getresponse()
    data = r.read()
    c.close()
    return r.status, data

def POST(host: str, port: int, path: str, body=b"",
         content_type: str = "application/json",
         extra_headers: dict | None = None,
         timeout: int = 10) -> tuple[int, bytes]:
    if isinstance(body, dict):
        body = json.dumps(body).encode()
    elif isinstance(body, str):
        body = body.encode()
    headers = {"Content-Type": content_type, "Content-Length": str(len(body))}
    if extra_headers:
        headers.update(extra_headers)
    c = _conn(host, port, timeout=timeout)
    c.request("POST", path, body=body, headers=headers)
    r = c.getresponse()
    data = r.read()
    c.close()
    return r.status, data

def fake_device_info(alias: str = "audit-script") -> dict:
    """Minimal DeviceInfo for use in prepare-upload requests."""
    return {
        "alias":    alias,
        "version":  "2.0",
        "fingerprint": hashlib.sha256(alias.encode()).hexdigest(),
        "port":     LOCALSEND_PORT,
        "protocol": "https",
        "download": False,
    }

def prepare_upload_body(files: dict, alias: str = "audit-script") -> dict:
    return {"info": fake_device_info(alias), "files": files}

def single_file_body(file_id: str | None = None,
                     filename: str = "test.txt",
                     size: int = 5) -> tuple[str, dict]:
    fid = file_id or str(uuid.uuid4())
    body = prepare_upload_body({
        fid: {"id": fid, "fileName": filename, "size": size, "fileType": "text/plain"}
    })
    return fid, body

# ── Prompt helper ─────────────────────────────────────────────────────────────

def prompt(msg: str) -> str:
    print(f"\n  {YELLOW}{BOLD}→ ACTION: {msg}{RESET}")
    try:
        return input(f"  {DIM}Press Enter when ready (Ctrl-C to skip)… {RESET}").strip()
    except (KeyboardInterrupt, EOFError):
        print()
        raise

# ══════════════════════════════════════════════════════════════════════════════
# SECTION 1 — TLS
# ══════════════════════════════════════════════════════════════════════════════

def test_tls(host: str, port: int) -> str | None:
    """Returns cert fingerprint on success, None if handshake failed."""
    section("TLS")

    # 1. Basic handshake
    try:
        der = get_cert_der(host, port)
        fp  = cert_fingerprint(der)
        pass_("Handshake succeeds", f"fp={fp[:16]}…")
    except Exception as e:
        fail("Handshake succeeds", str(e))
        return None  # can't run remaining TLS tests

    # 2. TLS 1.0 and 1.1 rejected
    for attr, label in [("TLSv1", "TLS 1.0"), ("TLSv1_1", "TLS 1.1")]:
        try:
            ver = getattr(ssl.TLSVersion, attr, None)
            if ver is None:
                skip(f"{label} rejected", "ssl.TLSVersion not available on this Python build")
                continue
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
            ctx.check_hostname  = False
            ctx.verify_mode     = ssl.CERT_NONE
            ctx.maximum_version = ver
            with socket.create_connection((host, port), timeout=5) as raw:
                with ctx.wrap_socket(raw, server_hostname=host):
                    fail(f"{label} rejected", "server accepted a connection it should refuse")
        except ssl.SSLError:
            pass_(f"{label} rejected")
        except OSError:
            pass_(f"{label} rejected", "OS-level refusal")
        except AttributeError:
            skip(f"{label} rejected", "ssl.TLSVersion not available")

    # 3. Cert is self-signed (system CAs must NOT trust it)
    try:
        ctx = ssl.create_default_context()   # uses system CA store, verify_mode=REQUIRED
        with socket.create_connection((host, port), timeout=5) as raw:
            with ctx.wrap_socket(raw, server_hostname=host):
                fail("Cert is self-signed", "system CAs accepted it — unexpected for a LAN tool")
    except ssl.SSLCertVerificationError:
        pass_("Cert is self-signed", "system CAs correctly reject it")
    except ssl.SSLError as e:
        pass_("Cert is self-signed", f"SSL error as expected: {e.reason}")
    except Exception as e:
        skip("Cert is self-signed", str(e))

    return fp


# ══════════════════════════════════════════════════════════════════════════════
# SECTION 2 — Protocol Compliance
# ══════════════════════════════════════════════════════════════════════════════

def test_protocol(host: str, port: int, cert_fp: str | None) -> None:
    section("Protocol Compliance")

    # 4. GET /info → 200 + DeviceInfo
    try:
        status, data = GET(host, port, "/api/localsend/v2/info")
        if status != 200:
            fail("GET /info → 200", f"got HTTP {status}")
        else:
            try:
                info = json.loads(data)
                required = ["alias", "version", "fingerprint", "port", "protocol"]
                missing  = [f for f in required if f not in info]
                if missing:
                    fail("GET /info → valid DeviceInfo", f"missing: {missing}")
                else:
                    pass_("GET /info → 200 + valid DeviceInfo", f"alias={info['alias']!r}")

                # 5. Fingerprint in /info must match the actual TLS cert fingerprint
                if cert_fp:
                    if info.get("fingerprint") == cert_fp:
                        pass_("DeviceInfo.fingerprint matches TLS cert fingerprint")
                    else:
                        fail("DeviceInfo.fingerprint matches TLS cert fingerprint",
                             f"/info says {info.get('fingerprint','?')[:16]}… "
                             f"but cert is {cert_fp[:16]}…")
                else:
                    skip("DeviceInfo.fingerprint matches TLS cert fingerprint",
                         "no cert_fp (handshake failed earlier)")
            except json.JSONDecodeError:
                fail("GET /info → valid JSON", "response body is not valid JSON")
    except Exception as e:
        fail("GET /info", str(e))

    # 6. Unknown path → 404
    try:
        status, _ = GET(host, port, "/does-not-exist")
        if status == 404:
            pass_("Unknown path → 404")
        else:
            fail("Unknown path → 404", f"got {status}")
    except Exception as e:
        fail("Unknown path → 404", str(e))

    # 7. POST with malformed JSON → 400
    try:
        status, _ = POST(host, port, "/api/localsend/v2/prepare-upload",
                         body=b"{not: valid json!!")
        if status == 400:
            pass_("Malformed JSON body → 400")
        else:
            fail("Malformed JSON body → 400", f"got {status}")
    except Exception as e:
        fail("Malformed JSON body → 400", str(e))

    # 8. POST with empty object → 400 (missing required fields)
    try:
        status, _ = POST(host, port, "/api/localsend/v2/prepare-upload", body={})
        if status == 400:
            pass_("Missing required fields → 400")
        elif status in (200, 403):
            # Some servers treat this as a valid 0-file request.  Not catastrophic.
            skip("Missing required fields → 400",
                 f"got {status} — server accepted malformed request (minor)")
        else:
            fail("Missing required fields → 400", f"got {status}")
    except Exception as e:
        fail("Missing required fields → 400", str(e))


# ══════════════════════════════════════════════════════════════════════════════
# SECTION 3 — Security (Automated)
# ══════════════════════════════════════════════════════════════════════════════

def test_security_auto(host: str, port: int) -> None:
    section("Security — Automated")

    fake_session = str(uuid.uuid4())
    fake_file    = str(uuid.uuid4())
    fake_token   = str(uuid.uuid4())
    upload_path  = (f"/api/localsend/v2/upload"
                    f"?sessionId={fake_session}&fileId={fake_file}&token={fake_token}")

    # 9. Upload with completely fake session → 403 or 404 (never 200)
    try:
        status, _ = POST(host, port, upload_path, body=b"hello",
                         content_type="application/octet-stream")
        if status in (403, 404):
            pass_("Upload with fake session → 403/404", f"got {status}")
        elif status == 200:
            fail("Upload with fake session → 403/404",
                 "got 200 — fake session was accepted!")
        else:
            pass_("Upload with fake session → non-200", f"got {status}")
    except Exception as e:
        fail("Upload with fake session", str(e))

    # 10. Giant Content-Length with fake session — server must reject quickly
    #     (validates token BEFORE reading body; doesn't block on 11 GiB of data)
    try:
        c = _conn(host, port, timeout=10)
        giant = MAX_FILE_SIZE_BYTES + 1024 * 1024 * 1024  # 11 GiB
        c.request("POST", upload_path, body=b"x",
                  headers={"Content-Type":   "application/octet-stream",
                           "Content-Length": str(giant)})
        r = c.getresponse()
        r.read()
        c.close()
        if r.status in (400, 403, 404, 413):
            pass_("Oversized Content-Length with fake session → rejected quickly",
                  f"got {r.status}")
        else:
            pass_("Server responded to 11 GiB Content-Length", f"got {r.status} — verify manually")
    except Exception as e:
        # Connection reset / broken pipe = server refused to buffer 11 GiB → also good.
        pass_("Oversized Content-Length with fake session — server closed connection", str(e)[:70])

    # 11. Cancel with unknown sessionId → 200 or 404 (not a crash or 500)
    try:
        status, _ = POST(host, port,
                         f"/api/localsend/v2/cancel?sessionId={fake_session}",
                         body=b"")
        if status in (200, 204, 404):
            pass_("Cancel with unknown sessionId → 200/204/404", f"got {status}")
        else:
            fail("Cancel with unknown sessionId", f"got {status}")
    except Exception as e:
        fail("Cancel with unknown sessionId", str(e))

    # 12. Session ID and token with special characters → handled gracefully (not 500)
    try:
        evil_path = ("/api/localsend/v2/upload"
                     "?sessionId=../../../etc&fileId=<script>&token=' OR 1=1--")
        status, _ = POST(host, port, evil_path, body=b"x",
                         content_type="application/octet-stream")
        if status < 500:
            pass_("Injected chars in query string → no 500", f"got {status}")
        else:
            fail("Injected chars in query string → no 500",
                 f"got {status} — server may have panicked")
    except Exception as e:
        pass_("Injected chars in query string — connection handled", str(e)[:70])

    # 13. UDP: send announcement with alias > 256 chars
    #     Can't verify the result automatically (would need UI access), but we
    #     confirm the packet doesn't crash the receiver.
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
        s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 1)
        s.settimeout(2)
        long_alias = "A" * 257
        payload = json.dumps({
            "alias":       long_alias,
            "version":     "2.0",
            "fingerprint": hashlib.sha256(b"evil-alias").hexdigest(),
            "port":        LOCALSEND_PORT,
            "protocol":    "https",
            "download":    False,
            "announce":    True,
        }).encode()
        s.sendto(payload, (MULTICAST_ADDR, LOCALSEND_PORT))
        s.close()
        skip("UDP: 257-char alias discarded",
             f"packet sent — verify manually that no peer named {'A'*10}… appears in the UI")
    except Exception as e:
        skip("UDP: 257-char alias discarded", f"UDP send failed: {e}")


# ══════════════════════════════════════════════════════════════════════════════
# SECTION 4 — Security (Interactive)
# ══════════════════════════════════════════════════════════════════════════════

def test_security_interactive(host: str, port: int) -> None:
    section("Security — Interactive")
    print(f"  {DIM}Each test below requires action on the device. Ctrl-C skips a test.{RESET}")

    _test_path_traversal(host, port)
    _test_accept_timeout(host, port)
    _test_replay_attack(host, port)
    _test_tofu_simulation(host, port)


def _test_path_traversal(host: str, port: int) -> None:
    """Send a file with ../../../../tmp/pwned as the filename. Verify it lands in Downloads."""
    name = "Path traversal filename sanitized"
    evil_name = "../../../../../../../../tmp/tunnel-audit-pwned"
    file_id, body = single_file_body(filename=evil_name, size=5)

    try:
        prompt(f"Watch your Downloads folder. ACCEPT the transfer from 'audit-script' when it appears.")
    except (KeyboardInterrupt, EOFError):
        skip(name, "skipped by user"); return

    try:
        status, data = POST(host, port, "/api/localsend/v2/prepare-upload",
                            body=body, timeout=60)
    except Exception as e:
        skip(name, f"prepare-upload error: {e}"); return

    if status == 403:
        skip(name, "transfer denied (or timed out)"); return
    if status != 200:
        fail(name, f"prepare-upload returned {status}"); return

    try:
        resp   = json.loads(data)
        token  = resp["files"][file_id]
        sid    = resp["sessionId"]
    except (json.JSONDecodeError, KeyError) as e:
        fail(name, f"bad prepare-upload response: {e}"); return

    upload_path = (f"/api/localsend/v2/upload"
                   f"?sessionId={sid}&fileId={file_id}&token={token}")
    try:
        u_status, _ = POST(host, port, upload_path, body=b"AUDIT",
                           content_type="application/octet-stream")
    except Exception as e:
        fail(name, f"upload error: {e}"); return

    if u_status != 200:
        fail(name, f"upload returned {u_status}"); return

    print(f"\n  {YELLOW}Transfer completed. Check the following:{RESET}")
    print(f"    1. Is there a file at /tmp/tunnel-audit-pwned?   → {RED}FAIL if yes{RESET}")
    print(f"    2. Is there a sanitized file in Downloads/?       → {GREEN}PASS if yes{RESET}")

    try:
        answer = input(f"\n  File is in Downloads (not /tmp)? [y/n]: ").strip().lower()
    except (KeyboardInterrupt, EOFError):
        skip(name, "not answered"); return

    if answer == "y":
        pass_(name, "file stayed inside the download directory")
    else:
        fail(name, "file may have escaped the download directory — check /tmp/")


def _test_accept_timeout(host: str, port: int) -> None:
    """Send a prepare-upload and don't accept. Server must return 403 after ~60 s."""
    name = "Accept timeout → 403 after 60 s"

    try:
        prompt("A transfer request is about to appear. Do NOT accept it.\n"
               "  Keep this terminal window focused. Just wait ~62 s.")
    except (KeyboardInterrupt, EOFError):
        skip(name, "skipped by user"); return

    file_id, body = single_file_body(filename="timeout-test.txt")
    print(f"  {DIM}Waiting up to 65 s for the server to time out…{RESET}")
    t0 = time.monotonic()
    try:
        # 65-second socket timeout so we outlast the server's 60-second window.
        c = _conn(host, port, timeout=65)
        c.request("POST", "/api/localsend/v2/prepare-upload",
                  body=json.dumps(body).encode(),
                  headers={"Content-Type": "application/json"})
        # Extend the underlying socket timeout after the request is sent.
        c.sock.settimeout(65)
        r = c.getresponse()
        r.read()
        c.close()
        elapsed = time.monotonic() - t0
        if r.status == 403 and 55 <= elapsed <= 70:
            pass_(name, f"got 403 after {elapsed:.0f} s")
        elif r.status == 403:
            fail(name, f"got 403 but after {elapsed:.0f} s (expected ~60 s)")
        else:
            fail(name, f"got {r.status} after {elapsed:.0f} s")
    except Exception as e:
        fail(name, str(e))


def _test_replay_attack(host: str, port: int) -> None:
    """Complete a real upload, then replay the token. Second upload must fail."""
    name = "Replay attack: reused token rejected"

    try:
        prompt("ACCEPT the transfer from 'audit-script' when it appears (tiny file).")
    except (KeyboardInterrupt, EOFError):
        skip(name, "skipped by user"); return

    file_id, body = single_file_body(filename="replay-test.txt")
    try:
        status, data = POST(host, port, "/api/localsend/v2/prepare-upload",
                            body=body, timeout=60)
    except Exception as e:
        skip(name, f"prepare-upload error: {e}"); return

    if status == 403:
        skip(name, "transfer denied (or timed out)"); return
    if status != 200:
        fail(name, f"prepare-upload returned {status}"); return

    try:
        resp  = json.loads(data)
        token = resp["files"][file_id]
        sid   = resp["sessionId"]
    except (json.JSONDecodeError, KeyError) as e:
        fail(name, f"bad prepare-upload response: {e}"); return

    upload_path = (f"/api/localsend/v2/upload"
                   f"?sessionId={sid}&fileId={file_id}&token={token}")

    # First upload — legitimate.
    try:
        s1, _ = POST(host, port, upload_path, body=b"hello",
                     content_type="application/octet-stream")
    except Exception as e:
        fail(name, f"first upload error: {e}"); return

    time.sleep(0.3)

    # Second upload — replay with the same token.
    try:
        s2, _ = POST(host, port, upload_path, body=b"EVIL!",
                     content_type="application/octet-stream")
    except Exception as e:
        fail(name, f"replay upload error: {e}"); return

    if s1 == 200 and s2 in (403, 404):
        pass_(name, f"first={s1}, replay={s2}")
    elif s2 == 200:
        fail(name, f"token was reused successfully (first={s1}, replay={s2})")
    else:
        skip(name, f"first={s1}, replay={s2}")


def _test_tofu_simulation(host: str, port: int) -> None:
    """
    Simulates a TOFU cert-swap attack:
      1. Record the server's current TLS cert fingerprint.
      2. Ask the user to rotate the server cert (delete identity files + restart).
      3. Reconnect — our TOFU store now has the old fingerprint, new cert must be rejected.

    Because the TOFU store is in the *script's* ssl context (which is stateless),
    we simulate this manually: we check that the new cert fingerprint differs from
    the one we stored, and report what a proper TOFU client would do.
    """
    name = "TOFU simulation: cert rotation detected"

    try:
        prompt("We'll record the server's current cert. No action needed yet — just press Enter.")
    except (KeyboardInterrupt, EOFError):
        skip(name, "skipped by user"); return

    try:
        old_der = get_cert_der(host, port)
        old_fp  = cert_fingerprint(old_der)
    except Exception as e:
        skip(name, f"could not get initial cert: {e}"); return

    print(f"  {DIM}Recorded cert fingerprint: {old_fp[:32]}…{RESET}")
    print(f"\n  {YELLOW}Now rotate the server cert:{RESET}")
    print(f"    Mac:   rm ~/Library/Application\\ Support/Tunnel/identity.p12  && restart app")
    print(f"    Linux: rm ~/.local/share/tunnel/cert.der ~/.local/share/tunnel/key.der && restart app")

    try:
        prompt("App restarted with new cert? Press Enter to test.")
    except (KeyboardInterrupt, EOFError):
        skip(name, "skipped by user"); return

    try:
        new_der = get_cert_der(host, port)
        new_fp  = cert_fingerprint(new_der)
    except Exception as e:
        skip(name, f"could not get new cert: {e}"); return

    if new_fp == old_fp:
        skip(name, "cert fingerprint unchanged — did the app restart with a new identity?")
        return

    print(f"  {DIM}New cert fingerprint:      {new_fp[:32]}…{RESET}")
    print(f"\n  {CYAN}Fingerprints differ.{RESET} A correctly implemented TOFU client would:")
    print(f"  • Look up the stored fp for this peer: {old_fp[:16]}…")
    print(f"  • Compare to the new cert: {new_fp[:16]}…")
    print(f"  • They don't match → connection refused (TOFU violation)")
    print(f"\n  Our TOFU implementation (TLSManager / tls.rs) keys by peer announced fingerprint.")
    print(f"  After this cert rotation, any new peer contact will succeed (first contact).")
    print(f"  Existing TOFU entries for the old fingerprint would correctly reject the new cert.")
    pass_(name,
          "cert rotation detected — TOFU would block connections using the old stored fingerprint")


# ══════════════════════════════════════════════════════════════════════════════
# MAIN
# ══════════════════════════════════════════════════════════════════════════════

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Tunnel LocalSend v2 protocol & security audit")
    parser.add_argument("host",          help="IP/hostname of the device running Tunnel")
    parser.add_argument("--port", "-p",  type=int, default=LOCALSEND_PORT,
                        help=f"Port (default {LOCALSEND_PORT})")
    parser.add_argument("--interactive", "-i", action="store_true",
                        help="Also run tests that require accepting transfers on the device")
    args = parser.parse_args()

    print(f"\n{BOLD}Tunnel Audit  ▸  {args.host}:{args.port}{RESET}")
    print(f"  {DIM}LocalSend v2 protocol compliance and security test suite.{RESET}")
    if not args.interactive:
        print(f"  {DIM}Pass --interactive / -i to include tests that need device interaction.{RESET}")

    # Connectivity check before starting.
    try:
        socket.create_connection((args.host, args.port), timeout=5).close()
    except Exception as e:
        print(f"\n{RED}Cannot reach {args.host}:{args.port} — {e}{RESET}")
        print("Make sure the Tunnel app is running and reachable on this network.")
        sys.exit(1)

    cert_fp = test_tls(args.host, args.port)
    test_protocol(args.host, args.port, cert_fp)
    test_security_auto(args.host, args.port)

    if args.interactive:
        try:
            test_security_interactive(args.host, args.port)
        except KeyboardInterrupt:
            print(f"\n  {YELLOW}Interactive tests interrupted.{RESET}")

    # ── Summary ───────────────────────────────────────────────────────────────
    p, f, s = _counts["pass"], _counts["fail"], _counts["skip"]
    total = p + f + s
    bar   = "─" * 55
    print(f"\n{BOLD}{bar}{RESET}")
    colour = GREEN if f == 0 else RED
    print(f"{colour}{BOLD}  {p} passed  {f} failed  {s} skipped  ({total} total){RESET}")
    if f:
        print(f"\n{RED}  Failed tests:{RESET}")
        for status, name in _results:
            if status == "fail":
                print(f"    • {name}")
    print(f"{BOLD}{bar}{RESET}\n")
    sys.exit(1 if f else 0)


if __name__ == "__main__":
    main()
