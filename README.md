# Rust RDP Knock Gateway (V1 prototype)

Front-end authentication on first RDP TCP connection; successful validation grants the *TCP peer IP* for a fixed period, then closes that connection. Subsequent connections from that IP are raw TCP-proxied to Windows Server 2019. No nftables DNAT or NeutrinoRDP is used.

## Status: prototype; NOT production-tested

This project was generated without `cargo` or network access in the authoring environment. **Rust compilation and actual mstsc interoperability have not been verified.** In particular, verify `ironrdp-server=0.13.0` server builder and display-handler signatures with `cargo check`; verify that aborting `run_connection` after the validator's accept result reliably produces reconnectable behavior. The current callback signals grant before the accept response finishes, so the client may receive a transport disconnect rather than a successful login screen. This is a deliberate V1 choice, not a seamless handover.

## Security / compatibility

* Front-end uses IronRDP TLS security, **not CredSSP/NLA**. The credentials originate in RDP `ClientInfoPdu` after TLS negotiation. Client-side connection policies that require NLA may reject it. Backend Windows 2019 **can retain NLA** because subsequent RDP traffic is byte-for-byte proxied.
* Configure a trusted TLS certificate. Never use an untrusted cert for internet credentials.
* Frontend grants by IPv4/IPv6 source address. Shared NAT IPs share access for 5 minutes. VPN / zero-trust access is preferable on hostile networks. For public deployments implement brute-force IP/account throttling and credential auditing before exposing port 3389.
* In-memory grants vanish when the process restarts; existing raw TCP sessions may drop on restart.
* No UDP RDP acceleration. TCP-only RDP connections work; UDP transport features are not forwarded.
* The optional hook is executed after password verification and **before** grant; the gateway runs under a restricted user, so the hook must not require elevated privileges unless securely delegated.
* Choose a high-entropy unique gateway password; it is distinct from backend Windows credentials.

## Install for a local test

1. Install Debian 13 `cargo` / Rust toolchain, OpenSSL, and build tools. Use a stable recent Rust toolchain compatible with IronRDP 0.13.
2. Run `cargo check && cargo test && cargo build --release` in the project directory. Resolve any upstream API changes before deployment.
3. Create TLS certificate/key (production: use CA-signed certificate):

   `openssl req -x509 -newkey rsa:3072 -nodes -days 7 -keyout key.pem -out cert.pem -subj '/CN=rdp-gateway.local'`

4. Generate an Argon2id PHC password hash. One option is `python3 -m pip install argon2-cffi` then:

   `python3 -c 'from argon2 import PasswordHasher; import getpass; print(PasswordHasher().hash(getpass.getpass()))'`

5. Create dedicated `rdpknock` system user, copy configuration/certificate/hash with permissions readable only by that user, binary to `/usr/local/bin/`, and systemd unit to `/etc/systemd/system/`. Do not put plaintext passwords in TOML.
6. Ensure `3389/tcp` is free (`ss -ltnp '( sport = :3389 )'`), and only reachable from intended clients. Configure Windows firewall to accept backend RDP only from the gateway's IP. Do not forward Windows RDP to the Internet separately.
7. `systemctl daemon-reload && systemctl enable --now rdpx`; inspect `journalctl -u rdpx -f`.
8. First `mstsc /v:gateway:3389`: TLS-mode front-end login as `gateway` with the chosen password. It disconnects after acceptance. Second `mstsc /v:gateway:3389`: byte-for-byte TCP proxy to Windows, and normal Windows login/NLA.

## Limitations and next work

The first implementation uses in-memory IP grants, and is suitable only for private test networks until brute-force controls, durable test suite, per-user access controls, error-screen behavior, and Windows mstsc integration tests are added. Front-end RDP security should eventually support CredSSP/NLA via a validated server-side authentication pathway. Consider explicit deny/allow-IP policy, bounded handshake tasks, TLS cert rotation, configurable session maximum duration, and per-address rate limits.
