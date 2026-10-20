# Rust RDP 敲门网关（原型）

首次 RDP 连接使用 NLA（CredSSP/NTLM）验证网关账号。验证成功并执行可选钩子后，网关在设定时间内授权客户端的 TCP 来源 IP，然后关闭首次连接。客户端再次连接同一地址时，网关将 TCP 流量原样转发到 Windows；Windows 的账号和 NLA 设置独立于网关。

这是原型，尚未完成 Windows `mstsc` 的端到端互操作验证。首次连接成功后客户端可能显示断开，需手动重新连接。网关没有暴力破解限流；共享 NAT 出口的客户端共享授权，不适合直接暴露在公网。

## 安装和配置

1. 运行 `cargo test` 和 `cargo build --release`。
2. 准备客户端信任的 TLS 证书与私钥，证书身份应匹配客户端连接使用的主机名。NLA 也使用 TLS。
3. 创建单独的网关账号密码文件。文件是 UTF-8 单行明文密码，允许末尾一个换行，不要复用 Windows 密码。例如以 root 创建，避免在命令行和 shell 历史中输入密码：

   ```bash
   sudo install -d -m 750 -o root -g rdpknock /etc/rdpx
   sudo sh -c 'umask 077; stty -echo; printf "Gateway password: " >&2; IFS= read -r password; stty echo; printf "\\n" >&2; printf "%s\\n" "$password" > /etc/rdpx/password'
   sudo chown rdpknock:rdpknock /etc/rdpx/password
   sudo chmod 600 /etc/rdpx/password
   ```

   密码文件可被服务进程读取，服务进程被攻陷时密码也会泄露。妥善保护备份，不要提交密码文件。
4. 从 `config/config.toml.example` 创建 `/etc/rdpx/config.toml`，设置 `password_file`、`username`、证书、监听及后端地址。旧版 `password_hash_file` 和 Argon2 哈希不能用于 NLA，升级时必须改为密码文件。
5. 将 `target/release/rdpx` 安装到 `/usr/local/bin/`，安装 `systemd` 服务并重启。确保证书私钥和密码文件可由 `rdpknock` 读取。
6. 首次用网关账号连接；看到 `frontend NLA authentication succeeded` 后重新连接，并使用 Windows 账号登录。`journalctl -u rdpx -f` 可查看诊断信息。后端超时应检查网关到 Windows 的路由和 TCP 3389 防火墙。

授权存储在内存中，服务重启后失效；仅转发 TCP，不支持 UDP 加速。证书校验失败时应配置受信任且与连接名称匹配的证书，不要忽略证书警告。
