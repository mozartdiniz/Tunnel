import os
import re

# Colors for terminal output
GREEN = "\033[92m"
RED = "\033[91m"
YELLOW = "\033[93m"
BOLD = "\033[1m"
RESET = "\033[0m"

class SecurityChecker:
    def __init__(self):
        self.results = []

    def check(self, name, description, file_path, vuln_pattern, fix_pattern):
        status = f"{YELLOW}PENDING{RESET}"
        if not os.path.exists(file_path):
            status = f"{BOLD}N/A (File Missing){RESET}"
        else:
            with open(file_path, 'r') as f:
                content = f.read()
                if re.search(fix_pattern, content):
                    status = f"{GREEN}FIXED{RESET}"
                elif re.search(vuln_pattern, content):
                    status = f"{RED}VULNERABLE{RESET}"
        
        self.results.append((name, status, description))

    def print_report(self):
        print(f"\n{BOLD}=== Tunnel Security Status Report ==={RESET}")
        print(f"{'Requirement':<30} | {'Status':<15} | {'Description'}")
        print("-" * 80)
        for name, status, desc in self.results:
            print(f"{name:<30} | {status:<24} | {desc}")
        print("-" * 80)

def run_audit():
    checker = SecurityChecker()

    # --- LINUX (RUST) CHECKS ---
    
    # 1. Mutual TLS (mTLS)
    checker.check(
        "Linux: mTLS Auth",
        "Server requires client certs",
        "linux/src/tls.rs",
        r"\.with_no_client_auth\(\)",
        r"with_client_cert_verifier"
    )

    # 2. Path Traversal
    checker.check(
        "Linux: Path Sanitization",
        "Handles '..' and '.'",
        "linux/src/transfer.rs",
        r"match c \{.*'/' \| '\\'.*\}", # The weak version we found
        r"name == \"\.\.\"|name == \"\.\"" # Looking for explicit checks
    )

    # 3. Disk Space Check
    checker.check(
        "Linux: Disk Quotas",
        "Checks space before write",
        "linux/src/transfer.rs",
        r"File::create", # If it only has create without a space check
        r"available_space|disk_space" # Looking for quota logic
    )

    # 4. Checksum Cleanup
    checker.check(
        "Linux: Checksum Integrity",
        "Deletes file on failure",
        "linux/src/transfer.rs",
        r"tracing::error!\(.*Checksum FAIL", # Found logging but no removal
        r"fs::remove_file" # Looking for cleanup logic
    )

    # --- MACOS (SWIFT) CHECKS ---

    # 5. mTLS Auth
    checker.check(
        "macOS: mTLS Auth",
        "Peer auth is required",
        "mac/Sources/Tunnel/TLSManager.swift",
        r"authentication_required.*false",
        r"authentication_required.*true"
    )

    # 6. Path Sanitization
    checker.check(
        "macOS: Path Sanitization",
        "Handles '..' and '.'",
        "mac/Sources/Tunnel/Transfer.swift",
        r"\"/\\:\*\?\"<>\|\"", # The weak charset
        r"lastPathComponent == \"\.\.\""
    )

    checker.print_report()

if __name__ == "__main__":
    run_audit()
