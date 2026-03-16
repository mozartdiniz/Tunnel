"""
Tunnel Security Checker
Verifies that the four confirmed vulnerabilities from the security audit are fixed.
Run from the repo root: python3 check_security.py
"""

import os
import re
import sys

GREEN  = "\033[92m"
RED    = "\033[91m"
YELLOW = "\033[93m"
BOLD   = "\033[1m"
RESET  = "\033[0m"

def read_file(path):
    if not os.path.exists(path):
        return None
    with open(path, "r", encoding="utf-8") as f:
        return f.read()

def has(content, pattern):
    """Search with DOTALL so '.' crosses newlines."""
    return bool(re.search(pattern, content, re.DOTALL))


class SecurityChecker:
    def __init__(self):
        self.results = []

    def add(self, name, platform, severity, fixed, file_path, detail):
        self.results.append({
            "name": name,
            "platform": platform,
            "severity": severity,
            "fixed": fixed,       # True=fixed, False=vulnerable, None=file missing
            "file": file_path,
            "detail": detail,
        })

    def report(self):
        print(f"\n{BOLD}{'='*60}{RESET}")
        print(f"{BOLD}  Tunnel Security Status Report{RESET}")
        print(f"{BOLD}{'='*60}{RESET}\n")

        vuln_count = 0
        na_count   = 0

        for r in self.results:
            if r["fixed"] is None:
                badge = f"{YELLOW}{BOLD}[N/A]{RESET}"
                na_count += 1
            elif r["fixed"]:
                badge = f"{GREEN}{BOLD}[FIXED]{RESET}"
            else:
                badge = f"{RED}{BOLD}[VULNERABLE]{RESET}"
                vuln_count += 1

            sev   = r["severity"]
            sev_c = RED if sev == "Critical" else YELLOW
            print(f"  {badge}  {r['platform']} — {r['name']}")
            print(f"         Severity : {sev_c}{sev}{RESET}")
            print(f"         File     : {r['file']}")
            print(f"         Check    : {r['detail']}")
            print()

        print(f"{BOLD}{'='*60}{RESET}")
        if vuln_count == 0 and na_count == 0:
            print(f"{GREEN}{BOLD}  All checks passed.{RESET}")
        else:
            if vuln_count:
                print(f"{RED}{BOLD}  {vuln_count} vulnerability(ies) remain unfixed.{RESET}")
            if na_count:
                print(f"{YELLOW}  {na_count} check(s) skipped (file not found).{RESET}")
        print(f"{BOLD}{'='*60}{RESET}\n")

        return vuln_count == 0


def run_audit():
    checker = SecurityChecker()

    # ── Linux (Rust) ──────────────────────────────────────────────────────────

    tls_rs      = read_file("linux/src/tls.rs")
    transfer_rs = read_file("linux/src/transfer.rs")

    # 1. mTLS — server must not use with_no_client_auth()
    #    Vuln  : ServerConfig built with .with_no_client_auth()
    #    Fixed : with_no_client_auth() gone AND with_client_cert_verifier present
    if tls_rs is None:
        checker.add("mTLS (server auth)", "Linux", "Critical", None,
                    "linux/src/tls.rs", "File not found")
    else:
        still_vuln = has(tls_rs, r"\.with_no_client_auth\(\)")
        has_fix    = has(tls_rs, r"with_client_cert_verifier")
        fixed      = has_fix and not still_vuln
        detail = (
            "with_no_client_auth() removed AND with_client_cert_verifier present"
            if fixed else
            "ServerConfig still uses .with_no_client_auth() — sender identity unverified"
        )
        checker.add("mTLS (server auth)", "Linux", "Critical", fixed,
                    "linux/src/tls.rs", detail)

    # 2. Path traversal — sanitize_filename must block ".." and "."
    #    Vuln  : function exists with no ".." / "." check
    #    Fixed : explicit check for those reserved names present in the file
    if transfer_rs is None:
        checker.add("Path traversal (.., .)", "Linux", "Medium", None,
                    "linux/src/transfer.rs", "File not found")
    else:
        # Look for any comparison against ".." or "." after sanitization
        has_fix = has(transfer_rs, r'== "\.\."') or has(transfer_rs, r"== r?\"\.\"")
        detail = (
            'Explicit ".." / "." rejection found'
            if has_fix else
            'sanitize_filename passes ".." through unchanged — path traversal possible'
        )
        checker.add("Path traversal (.., .)", "Linux", "Medium", has_fix,
                    "linux/src/transfer.rs", detail)

    # 3. Disk space — must check available space before accepting a large transfer
    #    Vuln  : File::create called without any disk space check
    #    Fixed : available_space / statvfs / fs2 / free_space check present before create
    if transfer_rs is None:
        checker.add("Disk space check", "Linux", "Medium", None,
                    "linux/src/transfer.rs", "File not found")
    else:
        has_fix = has(transfer_rs, r"available_space|statvfs|free_space|disk_space|fs2::")
        detail = (
            "Disk space check found before file creation"
            if has_fix else
            "No disk space check — attacker can declare huge size_bytes to fill disk"
        )
        checker.add("Disk space check", "Linux", "Medium", has_fix,
                    "linux/src/transfer.rs", detail)

    # 4. Checksum cleanup — corrupt file must be deleted on mismatch
    #    Vuln  : ChecksumFail branch logs but does not delete the partial file
    #    Fixed : remove_file (tokio or std) called somewhere in receive path
    if transfer_rs is None:
        checker.add("Checksum cleanup", "Linux", "Medium", None,
                    "linux/src/transfer.rs", "File not found")
    else:
        has_fix = has(transfer_rs, r"remove_file")
        detail = (
            "remove_file call found — corrupt files cleaned up"
            if has_fix else
            "No remove_file after ChecksumFail — corrupt file left on disk"
        )
        checker.add("Checksum cleanup", "Linux", "Medium", has_fix,
                    "linux/src/transfer.rs", detail)

    # ── macOS (Swift) ─────────────────────────────────────────────────────────

    tls_swift      = read_file("mac/Sources/Tunnel/TLSManager.swift")
    transfer_swift = read_file("mac/Sources/Tunnel/Transfer.swift")

    # 5. mTLS — server side must also verify client certificates
    #    Vuln  : set_verify_block is inside `if !isServer { }` → listener never verifies clients
    #    Fixed : guard removed (verify_block unconditional) OR peer_authentication_required = true
    if tls_swift is None:
        checker.add("mTLS (server auth)", "macOS", "Critical", None,
                    "mac/Sources/Tunnel/TLSManager.swift", "File not found")
    else:
        # Detect the vulnerable pattern: verify_block only for the !isServer branch
        guarded_by_client_only = has(tls_swift, r"if\s+!isServer\s*\{[^}]*set_verify_block")
        peer_auth_required     = has(tls_swift, r"set_peer_authentication_required.*,\s*true")
        fixed = peer_auth_required or not guarded_by_client_only
        detail = (
            "Server also verifies client certificates"
            if fixed else
            "set_verify_block is inside `if !isServer` — listener never verifies incoming clients"
        )
        checker.add("mTLS (server auth)", "macOS", "Critical", fixed,
                    "mac/Sources/Tunnel/TLSManager.swift", detail)

    # 6. Path traversal — sanitizeFilename must block ".." and "."
    #    Vuln  : function only strips special chars, passes ".." through
    #    Fixed : explicit ".." / "." check added
    if transfer_swift is None:
        checker.add("Path traversal (.., .)", "macOS", "Medium", None,
                    "mac/Sources/Tunnel/Transfer.swift", "File not found")
    else:
        has_fix = has(transfer_swift, r'"\.\."|== "\.\."')
        detail = (
            'Explicit ".." / "." rejection found'
            if has_fix else
            'sanitizeFilename passes ".." through unchanged — path traversal possible'
        )
        checker.add("Path traversal (.., .)", "macOS", "Medium", has_fix,
                    "mac/Sources/Tunnel/Transfer.swift", detail)

    # Note: macOS checksum cleanup is NOT checked here.
    # Transfer.swift buffers the entire file in memory and only writes if checksum passes
    # (line: `if checksumOk { try received.write(to: destURL) }`).
    # This is architecturally safe — no partial file is ever written.

    # Note: macOS disk space is also different — the OOM risk from huge in-memory
    # buffers is a separate concern from the Linux disk-fill scenario.

    ok = checker.report()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    run_audit()
