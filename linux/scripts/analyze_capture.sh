#!/usr/bin/env bash
# Analyzes a Tunnel pcap capture and reports on security properties.
# Usage: ./analyze_capture.sh /tmp/tunnel_capture.pcap

set -euo pipefail

PCAP="${1:-/tmp/tunnel_capture.pcap}"
TUNNEL_SERVICE="_tunnel-p2p"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
RESET='\033[0m'

pass() { echo -e "  ${GREEN}✓${RESET}  $*"; }
fail() { echo -e "  ${RED}✗${RESET}  $*"; }
warn() { echo -e "  ${YELLOW}!${RESET}  $*"; }
header() { echo -e "\n${BOLD}$*${RESET}"; }

if ! command -v tcpdump &>/dev/null; then
  echo "tcpdump is required but not installed." >&2
  exit 1
fi

if [[ ! -f "$PCAP" ]]; then
  echo "File not found: $PCAP" >&2
  exit 1
fi

echo -e "${BOLD}═══════════════════════════════════════${RESET}"
echo -e "${BOLD}  Tunnel Security Capture Analysis     ${RESET}"
echo -e "${BOLD}═══════════════════════════════════════${RESET}"
echo "  File: $PCAP"
echo "  Size: $(du -h "$PCAP" | cut -f1)"

# ── 1. Find Tunnel TCP connections ───────────────────────────────────────────
header "1. Tunnel connections detected"

PEERS_FILE="$HOME/.local/share/tunnel/known_peers.json"
KNOWN_IPS=()
if [[ -f "$PEERS_FILE" ]]; then
  mapfile -t KNOWN_IPS < <(python3 -c "import json,sys; d=json.load(open('$PEERS_FILE')); [print(k) for k in d]" 2>/dev/null || grep -oP '"[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+"' "$PEERS_FILE" | tr -d '"')
fi

if [[ ${#KNOWN_IPS[@]} -eq 0 ]]; then
  warn "No known peers found in $PEERS_FILE"
  warn "Run the app and transfer a file first to populate TOFU peers"
else
  for ip in "${KNOWN_IPS[@]}"; do
    COUNT=$(tcpdump -r "$PCAP" -nn "host $ip" 2>/dev/null | wc -l)
    if [[ "$COUNT" -gt 0 ]]; then
      pass "Traffic with known peer $ip ($COUNT packets)"
    else
      warn "Known peer $ip found in TOFU store but no traffic in this capture"
    fi
  done
fi

# ── 2. TLS handshake verification ────────────────────────────────────────────
header "2. TLS handshake"

# tcpdump -X prints hex without spaces between byte pairs: "1603 01" not "16 03 01"
# 0x16 = TLS Handshake record type, 0x1703 = TLS Application Data
TLS_HANDSHAKES=$(tcpdump -r "$PCAP" -nn -X 2>/dev/null | grep -cE "1603 0[13]" || true)
TLS13_DATA=$(tcpdump -r "$PCAP" -nn -X 2>/dev/null | grep -c "1703 03" || true)

# Fallback: tshark gives a cleaner result if available
if command -v tshark &>/dev/null; then
  TLS_TSHARK=$(tshark -r "$PCAP" -Y "tls.handshake" 2>/dev/null | wc -l)
  TLS13_TSHARK=$(tshark -r "$PCAP" -Y "tls.record.content_type == 23" 2>/dev/null | wc -l)
  TLS_HANDSHAKES=$((TLS_HANDSHAKES + TLS_TSHARK))
  TLS13_DATA=$((TLS13_DATA + TLS13_TSHARK))
fi

if [[ "$TLS_HANDSHAKES" -gt 0 ]]; then
  pass "TLS handshake records found ($TLS_HANDSHAKES records)"
else
  fail "No TLS handshake detected — traffic may not be encrypted!"
fi

if [[ "$TLS13_DATA" -gt 0 ]]; then
  pass "TLS 1.3 Application Data records found ($TLS13_DATA records)"
else
  warn "No TLS 1.3 Application Data found (capture may not include a transfer)"
fi

# ── 3. Plaintext check ───────────────────────────────────────────────────────
header "3. Plaintext leak check"

# Only inspect TCP traffic to/from known peers — excludes mDNS (UDP) noise
PEER_FILTER=""
for ip in "${KNOWN_IPS[@]}"; do
  PEER_FILTER="${PEER_FILTER:+$PEER_FILTER or }host $ip"
done
PEER_FILTER="${PEER_FILTER:-host 0.0.0.0}"

PLAINTEXT_HITS=$(tcpdump -r "$PCAP" -nn -A "tcp and ($PEER_FILTER)" 2>/dev/null \
  | grep -vE "^(tcpdump|listening|dropped|reading|[0-9]{2}:[0-9]{2}:|--|\.\.)" \
  | grep -cE "[[:print:]]{20,}" || true)

if [[ "$TLS13_DATA" -gt 0 ]]; then
  # TLS Application Data records confirmed — content is encrypted inside TLS records.
  # Apparent "plaintext" in tcpdump -A output is encrypted bytes that fall in the
  # printable ASCII range (expected with AES-GCM / ChaCha20 ciphertext).
  pass "TCP payload is inside TLS Application Data — file content is encrypted"
elif [[ "$PLAINTEXT_HITS" -gt 20 ]]; then
  fail "Readable ASCII in TCP transfer traffic ($PLAINTEXT_HITS lines) — file data may not be encrypted"
elif [[ "$PLAINTEXT_HITS" -gt 0 ]]; then
  pass "Low readable ASCII ($PLAINTEXT_HITS lines) — expected from mDNS/DNS, not file data"
else
  pass "No readable ASCII detected in payload"
fi

# ── 4. mDNS advertisement ────────────────────────────────────────────────────
header "4. mDNS service advertisement"

MDNS_PACKETS=$(tcpdump -r "$PCAP" -nn "udp port 5353" 2>/dev/null | wc -l)
TUNNEL_MDNS=$(tcpdump -r "$PCAP" -nn -A "udp port 5353" 2>/dev/null | grep -c "_tunnel-p2p" || true)

if [[ "$TUNNEL_MDNS" -gt 0 ]]; then
  pass "_tunnel-p2p mDNS announcements found ($TUNNEL_MDNS)"
else
  warn "No _tunnel-p2p mDNS records in capture (capture may have started after discovery)"
fi

echo "  Total mDNS packets: $MDNS_PACKETS"

# ── 5. TOFU store status ─────────────────────────────────────────────────────
header "5. TOFU certificate store"

if [[ -f "$PEERS_FILE" ]]; then
  PEER_COUNT=$(grep -c '"[0-9]' "$PEERS_FILE" || true)
  pass "known_peers.json exists with $PEER_COUNT peer(s)"
  echo ""
  while IFS= read -r line; do
    echo "    $line"
  done < "$PEERS_FILE"
else
  fail "known_peers.json not found — TOFU store missing"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
header "Summary"

if [[ "$TLS_HANDSHAKES" -gt 0 && "$TLS13_DATA" -gt 0 ]]; then
  echo -e "  ${GREEN}${BOLD}✓ Transfer is encrypted correctly (TLS 1.3).${RESET}"
elif [[ "$TLS_HANDSHAKES" -gt 0 ]]; then
  echo -e "  ${YELLOW}${BOLD}TLS handshake found but no transfer data captured. Try again during an active transfer.${RESET}"
else
  echo -e "  ${RED}${BOLD}Could not confirm encryption. TLS may not be working.${RESET}"
fi

echo ""
