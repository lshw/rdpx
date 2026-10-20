# rdpx（原型）

首次 RDP 连接使用 NLA（CredSSP/NTLM）验证网关账号。验证成功并执行可选钩子后，网关在设定时间内授权客户端的 TCP 来源 IP，然后关闭首次连接。客户端再次连接同一地址时，网关将 TCP 流量原样转发到 Windows；Windows 的账号和 NLA 设置独立于网关。

首次连接成功后客户端可能显示断开，需手动重新连接。网关没有暴力破解限流；共享 NAT 出口的客户端共享授权，不适合直接暴露在公网。

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

## Debian 11 构建

Debian 11 自带的 Rust 工具链过旧，不能编译当前依赖。下面的命令在 Debian 11 上以普通用户执行；安装系统软件包时使用 `sudo`。

1. 安装构建工具和 Debian 打包依赖：

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates curl build-essential dpkg-dev debhelper cargo rustc pkg-config
```

`apt` 安装的 Cargo 和 Rust 编译器用于满足 Debian 打包依赖；实际编译使用下方安装的 rustup 工具链。

2. 安装 [rustup](https://rustup.rs/)。如果已经安装，可以跳过安装命令：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

按提示选择默认安装选项。然后在当前终端加载环境，并进入项目源码目录：

```bash
. "$HOME/.cargo/env"
cd /path/to/rdpx
rustup toolchain install 1.92.0
rustup override set 1.92.0
command -v rustup rustc cargo
rustc --version
cargo --version
```

将 `/path/to/rdpx` 替换为实际源码目录。确认 `rustc` 和 `cargo` 均为 1.92.0，且命令路径位于 `~/.cargo/bin/`。`override` 只针对当前源码目录设置工具链。若提示 `rustup: command not found`，先确认安装成功，再执行 `. "$HOME/.cargo/env"`；安装给其他用户的 rustup 不会自动对当前用户生效。

3. 测试并编译程序：

```bash
cargo test --locked
cargo build --release --locked
```

生成的程序位于 `target/release/rdpx`。首次构建需要联网下载 `Cargo.lock` 中的依赖，或提前准备好 Cargo 缓存。

4. 如需生成 Debian 二进制包，在源码目录执行：

```bash
dpkg-buildpackage -us -uc -b
```

该命令会运行测试并构建软件包，生成的 `.deb` 位于源码目录的上一级。可用 `sudo apt install ../rdpx_*.deb` 安装；若存在多个匹配的包，请指定实际文件名。安装后按照 `/usr/share/doc/rdpx/README.Debian` 配置，服务不会自动启动。

## 小体积构建

运行 `cargo build --locked --profile small`，生成的程序位于 `target/small/rdpx`。此配置优先减小体积，编译和链接较慢；发生 panic 时进程直接退出，且不保留符号用于调试。部署到 OpenWrt 时还需使用与设备架构和 C 库匹配的工具链交叉编译，并自行配置 procd 服务。
