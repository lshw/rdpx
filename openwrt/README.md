# OpenWrt 打包

`Makefile` 是独立的 OpenWrt 包定义：构建时从 Git 下载固定提交，不依赖本地项目源码或其他包文件。当前项目尚未配置 Git 远程地址，构建时必须通过 `RDP_GATEWAY_GIT_URL` 指定能访问到该提交的仓库。仓库必须包含 Makefile 中 `PKG_SOURCE_VERSION` 指定的提交。

在 OpenWrt buildroot 或 SDK 中启用 packages feed，将 `Makefile` 复制到 `package/rdpx/Makefile`，然后运行：

```sh
./scripts/feeds install rust
export RDP_GATEWAY_GIT_URL='https://你的服务器/你的仓库.git'
make menuconfig
make package/rdpx/compile V=s
```

在 menuconfig 的 Network 分类中选中 `rdpx`。SDK 的 Rust 工具链须支持目标架构及本项目锁定的依赖；首次构建可能需要下载 Rust 工具链源码和 Cargo 依赖，并消耗大量时间和磁盘空间。更新源码时同时更新 `PKG_SOURCE_VERSION` 和 `PKG_SOURCE_DATE`；发布前应计算源码归档的 `PKG_MIRROR_HASH`。构建产物位于 `bin/packages/` 下的目标架构目录；安装方式取决于 OpenWrt 版本使用的包管理器。

安装后，将 `/usr/share/rdpx/config.toml.example` 复制到 `/etc/rdpx/config.toml`，配置后端、证书和独立的网关密码文件。创建目录及密码文件时应限制权限（例如目录 `700`，密码文件 `600`）。启动脚本以 root 运行，以便监听 3389 端口；务必保护配置和密钥。配置完成后运行：

```sh
/etc/init.d/rdpx enable
/etc/init.d/rdpx start
logread -e rdpx
```

配置文件不存在时服务不会启动。此包使用项目的 `small` 编译配置；交叉编译是否成功、最终大小和运行情况需要在目标设备上验证。
