# Security Audit Report: Tunnel App (Linux & macOS)

## Executive Summary
The Tunnel application claims to provide secure file transfers between peers. However, a security audit has revealed critical vulnerabilities that allow for **sender impersonation** and **unauthenticated access**. The application relies on a flawed implementation of TLS and Trust-On-First-Use (TOFU) that only protects the receiver's identity, leaving the sender's identity completely unverified.

---

## Critical Vulnerabilities

### 1. Unauthenticated Sender (Impersonation)
**Severity:** Critical
**Description:** 
The application uses TLS but does not require client certificates. While the sender verifies the receiver's certificate (TOFU), the receiver **never** verifies the sender's certificate. 
- In `linux/src/tls.rs`, the `ServerConfig` is built with `.with_no_client_auth()`.
- The protocol relies on a JSON `Message::Ask` containing a `sender_name` field, which is entirely self-reported and unvalidated.

**Impact:** 
An attacker on the same network can send a file claiming to be any user (e.g., "Admin", "CEO"). The victim's UI will display this spoofed name in the confirmation dialog.

**Proof of Concept (PoC):**
A Python script was successfully used to:
1. Connect to the Tunnel port.
2. Complete a TLS handshake (ignoring the server's self-signed cert).
3. Send a `Message::Ask` with `"sender_name": "SPOOFED_ADMIN"`.
4. The application accepted the request and prompted the user with the fraudulent name.

### 2. Lack of Resource/Disk Quotas (DoS)
**Severity:** Medium/High
**Description:**
The receiver starts writing the file to disk immediately after the user clicks "Accept", based on the `size_bytes` provided in the `Ask` message.
- There is no check for available disk space.
- There is no maximum file size limit enforced.

**Impact:**
An attacker can send a multi-terabyte file (or a stream that never ends) to fill the victim's disk, causing system instability or crashes (Denial of Service).

### 3. Unauthenticated Discovery (mDNS)
**Severity:** Low/Medium
**Description:**
Peer discovery via mDNS (`_tunnel-p2p._tcp.local.`) is entirely unauthenticated. 
- Anyone can advertise a service with a "trusted" name.
- While the subsequent TLS connection uses TOFU, the initial "discovery" phase is susceptible to noise and mass-impersonation.

### 4. Inadequate Filename Sanitization (Path Traversal)
**Severity:** Medium
**Description:**
The `sanitize_filename` function (in both Linux and macOS) replaces characters like `/` and `\` with underscores. However, it does **not** check if the resulting filename is a reserved system name like `.` (current directory) or `..` (parent directory).

**Impact:**
- On Linux, sending `..` as a filename results in an attempt to write to the parent directory of the downloads folder. While this specific case fails because `..` is a directory, it demonstrates that the sanitization logic is incomplete.
- A more sophisticated attacker might find ways to leverage this in combination with other OS-specific features (e.g., hidden files, special device files).

### 5. Checksum Logic Flaw
**Severity:** Medium
**Description:**
The application receives file bytes first and **then** waits for a `Done` message containing the SHA-256 checksum.
- If the connection is severed after the bytes are sent but before the `Done` message is received, the partially/fully written file remains on disk without any verification.
- There is no mechanism to "roll back" or delete a file that fails the checksum.

---

## Technical Analysis of Flaws

### Linux (Rust)
```rust
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}
```
*Note: Returns ".." unchanged.*

### macOS (Swift)
```swift
private func sanitizeFilename(_ name: String) -> String {
    name.unicodeScalars
        .map { "/\\:*?\"<>|".unicodeScalars.contains($0) ? Character("_") : Character($0) }
        .map(String.init)
        .joined()
}
```
*Note: Also returns ".." unchanged.*

---

## Recommendations

1. **Implement Mutual TLS (mTLS):**
   - The receiver **must** require a client certificate during the TLS handshake.
   - Both sides should perform TOFU (Trust On First Use) on each other's certificate fingerprints.

2. **Cryptographically Bind Identities:**
   - Instead of trusting the `sender_name` in the JSON message, the UI should display the name associated with the verified certificate fingerprint.

3. **Enforce Resource Limits:**
   - Check available disk space before accepting a transfer.
   - Implement a maximum allowable file size.

4. **Verify Protocol Versioning:**
   - Ensure the `version` field in the protocol is strictly checked to prevent downgrade attacks.
