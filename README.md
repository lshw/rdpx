# rdpx (prototype)

The first RDP connection authenticates the gateway account using NLA (CredSSP/NTLM). After successful authentication and the optional hook, the gateway grants the TCP peer IP for a configured period and closes the connection. A subsequent connection from that IP is proxied byte-for-byte to the Windows backend, whose credentials and NLA settings are independent.

This is a prototype: end-to-end interoperability with Windows `mstsc` has not yet been verified. The first connection can show a disconnect and requires a manual reconnect. There is no brute-force rate limiting, and clients behind the same NAT share a grant; do not expose it directly to the Internet.

## Setup

1. Run `cargo test` and `cargo build --release`.
2. Provide a trusted TLS certificate and private key. The certificate identity must match the hostname used by clients. NLA also uses TLS.
3. Create a separate UTF-8 single-line plaintext gateway password file, readable only by the `rdpknock` service account (owner `rdpknock:rdpknock`, mode `600`). A single final newline is permitted. Do not reuse the Windows password, commit the file, or expose it in shell history. A compromised service can read this password.
4. Copy `config/config.toml.example` to `/etc/rdpx/config.toml` and configure `password_file`, `username`, certificate, listen and backend addresses. The old `password_hash_file` and Argon2 hash cannot be used for NLA; migrate to the password file.
5. Install `target/release/rdpx` to `/usr/local/bin/`, install the systemd unit, and restart the service. Ensure `rdpknock` can read the certificate key and password file.
6. Connect using the gateway account. After `frontend NLA authentication succeeded`, reconnect and use the Windows account. Check `journalctl -u rdpx -f` for errors. For backend timeouts check routing and TCP 3389 firewall rules from the gateway to Windows.

Grants are in memory and disappear on restart. Only TCP is forwarded; UDP acceleration is not supported. Use a trusted certificate matching the connection name rather than ignoring certificate warnings.

## Size-optimized build

Run `cargo build --locked --profile small` to produce `target/small/rdpx`. This profile favors size, takes longer to build, strips symbols, and aborts the process on panic. For OpenWrt, cross-compile with a toolchain matching the device architecture and C library, and configure a procd service.
