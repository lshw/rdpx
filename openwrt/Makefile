include $(TOPDIR)/rules.mk

PKG_NAME:=rdpx
PKG_VERSION:=0.1.0
PKG_RELEASE:=1
PKG_LICENSE:=MIT

# 仓库地址通过环境变量 RDP_GATEWAY_GIT_URL 提供。
PKG_SOURCE_PROTO:=git
PKG_SOURCE_URL:=$(RDP_GATEWAY_GIT_URL)
PKG_SOURCE_DATE:=2026-10-20
PKG_SOURCE_VERSION:=81115ac07017e899ae52bff77a697a4b5e5dbd35
# 使用固定 Git 提交；发布时应计算并填写源码归档的校验值。
PKG_MIRROR_HASH:=skip

PKG_BUILD_DEPENDS:=rust/host
PKG_BUILD_PARALLEL:=1
PKG_BUILD_FLAGS:=no-mips16

include $(INCLUDE_DIR)/package.mk
include $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk

CARGO_PKG_PROFILE:=small

define Package/rdpx
  SECTION:=net
  CATEGORY:=Network
  TITLE:=RDP NLA authentication gateway
  DEPENDS:=$(RUST_ARCH_DEPENDS) +libgcc
endef

define Package/rdpx/description
  Authenticate the first RDP connection and temporarily proxy subsequent
  connections from the same client IP to an RDP backend.
endef

define Package/rdpx/install
	$(INSTALL_DIR) $(1)/usr/bin $(1)/etc/init.d $(1)/usr/share/rdpx
	$(INSTALL_BIN) $(PKG_INSTALL_DIR)/bin/rdpx $(1)/usr/bin/rdpx
	$(INSTALL_DATA) $(PKG_BUILD_DIR)/config/config.toml.example $(1)/usr/share/rdpx/config.toml.example
	printf '%s\n' \
		'#!/bin/sh /etc/rc.common' \
		'START=95' \
		'STOP=10' \
		'USE_PROCD=1' \
		'start_service() {' \
		'    local config=/etc/rdpx/config.toml' \
		'    if [ ! -f "$$$$config" ]; then' \
		'        logger -t rdpx "配置文件不存在：$$$$config"' \
		'        return 0' \
		'    fi' \
		'    procd_open_instance' \
		'    procd_set_param command /usr/bin/rdpx "$$$$config"' \
		'    procd_set_param respawn' \
		'    procd_set_param stdout 1' \
		'    procd_set_param stderr 1' \
		'    procd_close_instance' \
		'}' \
		'reload_service() {' \
		'    stop' \
		'    start' \
		'}' > $(1)/etc/init.d/rdpx
	chmod 0755 $(1)/etc/init.d/rdpx
endef

$(eval $(call RustBinPackage,rdpx))
$(eval $(call BuildPackage,rdpx))
