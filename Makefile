# ─────────────────────────────────────────────────────────────────────────────
# WeFlow Native CLI — Cross-platform Makefile
# 支持平台: macOS · Linux · Windows (Git Bash / WSL)
# 用法: make help
# ─────────────────────────────────────────────────────────────────────────────

# ── 平台检测 ──────────────────────────────────────────────────────────────────
ifeq ($(OS),Windows_NT)
  PLATFORM     := windows
  EXE          := .exe
  BIN          := target/release/weflow.exe
  INSTALL_DIR  := $(USERPROFILE)/.cargo/bin
  OPEN         := start
  PKG_INSTALL  := winget install -e --id Git.Git
  RUST_INSTALL := powershell -NoProfile -Command \
    "Invoke-WebRequest -Uri https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe -OutFile rustup-init.exe; \
     Start-Process -Wait -FilePath rustup-init.exe -ArgumentList '-y'; \
     Remove-Item rustup-init.exe"
else
  UNAME := $(shell uname -s)
  EXE   :=
  BIN   := target/release/weflow
  ifeq ($(UNAME),Darwin)
    PLATFORM    := macos
    OPEN        := open
    INSTALL_DIR := /usr/local/bin
    RUST_INSTALL := curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    GIT_INSTALL  := xcode-select --install
    BREW_INSTALL := /bin/bash -c "$$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  else
    PLATFORM    := linux
    OPEN        := xdg-open
    INSTALL_DIR := /usr/local/bin
    RUST_INSTALL := curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    GIT_INSTALL  := $(shell \
      if command -v apt-get >/dev/null 2>&1; then echo "sudo apt-get install -y git"; \
      elif command -v dnf >/dev/null 2>&1; then echo "sudo dnf install -y git"; \
      elif command -v pacman >/dev/null 2>&1; then echo "sudo pacman -S --noconfirm git"; \
      elif command -v zypper >/dev/null 2>&1; then echo "sudo zypper install -y git"; \
      else echo "echo 'Please install git via your package manager'"; fi)
  endif
endif

# ── 版本 ──────────────────────────────────────────────────────────────────────
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*= *"\(.*\)"/\1/' 2>/dev/null || echo "unknown")

# ── 颜色（TTY 自动降级）────────────────────────────────────────────────────────
ifneq ($(NO_COLOR),1)
  BOLD   := \033[1m
  GREEN  := \033[0;32m
  YELLOW := \033[0;33m
  RED    := \033[0;31m
  CYAN   := \033[0;36m
  DIM    := \033[2m
  RESET  := \033[0m
endif

# ── 跨平台编译目标 ────────────────────────────────────────────────────────────
TARGET_MACOS_ARM  := aarch64-apple-darwin
TARGET_LINUX_X64  := x86_64-unknown-linux-gnu
TARGET_WIN_X64    := x86_64-pc-windows-gnu
TARGET_WIN_X64_MS := x86_64-pc-windows-msvc

.DEFAULT_GOAL := help

# ─────────────────────────────────────────────────────────────────────────────
# 帮助
# ─────────────────────────────────────────────────────────────────────────────
.PHONY: help
help:
	@printf "$(BOLD)WeFlow CLI v$(VERSION) — $(PLATFORM)$(RESET)\n"
	@printf "\n"
	@printf "$(BOLD)基础命令$(RESET)\n"
	@printf "  $(CYAN)make build$(RESET)          当前平台 debug 构建\n"
	@printf "  $(CYAN)make release$(RESET)         当前平台 release 构建 → $(BIN)\n"
	@printf "  $(CYAN)make test$(RESET)            运行全部单元测试\n"
	@printf "  $(CYAN)make check$(RESET)           cargo check（只检查，不编译）\n"
	@printf "  $(CYAN)make fmt$(RESET)             格式化代码\n"
	@printf "  $(CYAN)make lint$(RESET)            Clippy 静态分析\n"
	@printf "  $(CYAN)make clean$(RESET)           清理 target/\n"
	@printf "\n"
	@printf "$(BOLD)安装$(RESET)\n"
	@printf "  $(CYAN)make install$(RESET)         将 release binary 安装到 $(INSTALL_DIR)\n"
	@printf "  $(CYAN)make uninstall$(RESET)        从 $(INSTALL_DIR) 删除 weflow\n"
	@printf "\n"
	@printf "$(BOLD)跨平台编译$(RESET)\n"
	@printf "  $(CYAN)make cross-macos$(RESET)     → weflow-macos-arm64     ($(TARGET_MACOS_ARM))\n"
	@printf "  $(CYAN)make cross-linux$(RESET)     → weflow-linux-x64       ($(TARGET_LINUX_X64))\n"
	@printf "  $(CYAN)make cross-windows$(RESET)   → weflow-windows-x64.exe ($(TARGET_WIN_X64))\n"
	@printf "  $(CYAN)make cross-all$(RESET)       构建所有跨平台目标\n"
	@printf "\n"
	@printf "$(BOLD)环境$(RESET)\n"
	@printf "  $(CYAN)make check-tools$(RESET)     检查并自动安装所有必要工具\n"
	@printf "  $(CYAN)make env$(RESET)             显示当前环境信息\n"
	@printf "  $(CYAN)make ci$(RESET)              本地模拟 CI 流程 (check+test+release)\n"
	@printf "\n"

# ─────────────────────────────────────────────────────────────────────────────
# 工具检查与自动安装
# ─────────────────────────────────────────────────────────────────────────────
.PHONY: check-tools check-rust check-git check-curl

check-curl:
ifeq ($(PLATFORM),windows)
	@powershell -NoProfile -Command "if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue)) { Write-Error 'curl not found. Please install it or use Windows 10 1803+.' }" || exit 1
else
	@command -v curl >/dev/null 2>&1 || { \
		printf "$(RED)✗ curl 未找到$(RESET)\n"; \
		printf "  macOS:  brew install curl\n"; \
		printf "  Linux:  sudo apt-get install -y curl  # 或对应包管理器\n"; \
		exit 1; \
	}
endif

check-git:
ifeq ($(PLATFORM),windows)
	@command -v git >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ git 未找到，正在安装...$(RESET)\n"; \
		printf "  执行: $(GIT_INSTALL)\n"; \
		winget install -e --id Git.Git --accept-package-agreements --accept-source-agreements || { \
			printf "$(RED)  自动安装失败，请手动安装: https://git-scm.com/download/win$(RESET)\n"; exit 1; }; \
		printf "$(YELLOW)  安装完成，请重新打开终端使 git 生效$(RESET)\n"; \
	}
else ifeq ($(PLATFORM),macos)
	@command -v git >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ git 未找到，正在安装 Xcode Command Line Tools...$(RESET)\n"; \
		xcode-select --install 2>&1 | grep -v "already installed" || true; \
		printf "$(CYAN)  请在弹出的窗口中点击「安装」，完成后重新运行 make$(RESET)\n"; \
		exit 1; \
	}
else
	@command -v git >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ git 未找到，正在安装...$(RESET)\n"; \
		$(GIT_INSTALL) || { \
			printf "$(RED)  自动安装失败，请手动安装 git$(RESET)\n"; exit 1; }; \
		printf "$(GREEN)  git 安装完成$(RESET)\n"; \
	}
endif
	@printf "$(GREEN)✓ $(shell git --version)$(RESET)\n"

check-rust:
	@command -v cargo >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ Rust/Cargo 未找到，正在安装...$(RESET)\n"; \
		$(MAKE) check-curl; \
		$(RUST_INSTALL) || { \
			printf "$(RED)  自动安装失败，请手动访问 https://rustup.rs$(RESET)\n"; exit 1; }; \
		printf "$(GREEN)  Rust 安装完成$(RESET)\n"; \
		printf "$(CYAN)  请执行: source \$$HOME/.cargo/env  (或重开终端)$(RESET)\n"; \
		. "$$HOME/.cargo/env" 2>/dev/null || true; \
	}
	@printf "$(GREEN)✓ $(shell rustc --version)$(RESET)\n"
	@printf "$(GREEN)✓ $(shell cargo --version)$(RESET)\n"

check-tools: check-git check-rust
	@printf "$(GREEN)$(BOLD)所有工具就绪！$(RESET)\n"

# ─────────────────────────────────────────────────────────────────────────────
# 核心构建目标
# ─────────────────────────────────────────────────────────────────────────────
.PHONY: build release test check fmt lint clean

build: check-rust
	@printf "$(BOLD)▶ cargo build$(RESET)\n"
	cargo build -p weflow-cli

release: check-rust
	@printf "$(BOLD)▶ cargo build --release$(RESET)\n"
	cargo build --release -p weflow-cli
	@printf "$(GREEN)$(BOLD)✓ 构建完成 → $(BIN)$(RESET)\n"
	@ls -lh "$(BIN)" 2>/dev/null || true

test: check-rust
	@printf "$(BOLD)▶ cargo test --workspace$(RESET)\n"
	cargo test --workspace

check: check-rust
	@printf "$(BOLD)▶ cargo check --workspace$(RESET)\n"
	cargo check --workspace

fmt: check-rust
	@printf "$(BOLD)▶ cargo fmt$(RESET)\n"
	cargo fmt --all

lint: check-rust
	@printf "$(BOLD)▶ cargo clippy$(RESET)\n"
	cargo clippy --workspace --all-targets -- -D warnings

clean:
	@printf "$(BOLD)▶ cargo clean$(RESET)\n"
	cargo clean
	@printf "$(GREEN)✓ target/ 已清理$(RESET)\n"

# ─────────────────────────────────────────────────────────────────────────────
# 安装 / 卸载
# ─────────────────────────────────────────────────────────────────────────────
.PHONY: install uninstall

install: release
ifeq ($(PLATFORM),windows)
	@printf "$(BOLD)▶ 安装到 $(INSTALL_DIR)$(RESET)\n"
	@cp "$(BIN)" "$(INSTALL_DIR)/weflow.exe"
	@printf "$(GREEN)✓ 已安装到 $(INSTALL_DIR)/weflow.exe$(RESET)\n"
else
	@printf "$(BOLD)▶ 安装到 $(INSTALL_DIR)$(RESET)\n"
	@sudo cp "$(BIN)" "$(INSTALL_DIR)/weflow"
	@sudo chmod +x "$(INSTALL_DIR)/weflow"
	@printf "$(GREEN)✓ 已安装: $$(which weflow) — $$(weflow --version)$(RESET)\n"
endif

uninstall:
ifeq ($(PLATFORM),windows)
	@rm -f "$(INSTALL_DIR)/weflow.exe" && printf "$(GREEN)✓ 已卸载$(RESET)\n" || printf "$(DIM)未找到已安装的 weflow$(RESET)\n"
else
	@sudo rm -f "$(INSTALL_DIR)/weflow" && printf "$(GREEN)✓ 已卸载$(RESET)\n" || printf "$(DIM)未找到已安装的 weflow$(RESET)\n"
endif

# ─────────────────────────────────────────────────────────────────────────────
# 跨平台编译
# ─────────────────────────────────────────────────────────────────────────────
.PHONY: cross-macos cross-linux cross-windows cross-all
.PHONY: _ensure-target-macos _ensure-target-linux _ensure-target-windows

_ensure-target-macos:
	@rustup target list --installed | grep -q "$(TARGET_MACOS_ARM)" || { \
		printf "$(YELLOW)⚠ 安装编译目标 $(TARGET_MACOS_ARM)...$(RESET)\n"; \
		rustup target add $(TARGET_MACOS_ARM); \
	}

_ensure-target-linux:
	@rustup target list --installed | grep -q "$(TARGET_LINUX_X64)" || { \
		printf "$(YELLOW)⚠ 安装编译目标 $(TARGET_LINUX_X64)...$(RESET)\n"; \
		rustup target add $(TARGET_LINUX_X64); \
	}
ifeq ($(PLATFORM),macos)
	@command -v x86_64-linux-gnu-gcc >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ 交叉编译器未找到，正在安装 x86_64-linux-gnu gcc...$(RESET)\n"; \
		command -v brew >/dev/null 2>&1 || { printf "$(RED)  需要 Homebrew: $(BREW_INSTALL)$(RESET)\n"; exit 1; }; \
		brew install SergioBenitez/osxct/x86_64-unknown-linux-gnu || { \
			printf "$(RED)  安装失败，备选方案: brew install filosottile/musl-cross/musl-cross$(RESET)\n"; exit 1; }; \
	}
endif
ifeq ($(PLATFORM),linux)
	@command -v x86_64-linux-gnu-gcc >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ 安装 gcc multilib...$(RESET)\n"; \
		$(GIT_INSTALL:-y git=-y gcc-multilib) 2>/dev/null || sudo apt-get install -y gcc-multilib || true; \
	}
endif

_ensure-target-windows:
	@rustup target list --installed | grep -q "$(TARGET_WIN_X64)" || { \
		printf "$(YELLOW)⚠ 安装编译目标 $(TARGET_WIN_X64)...$(RESET)\n"; \
		rustup target add $(TARGET_WIN_X64); \
	}
ifeq ($(PLATFORM),macos)
	@command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ 安装 mingw-w64 交叉编译器...$(RESET)\n"; \
		command -v brew >/dev/null 2>&1 || { printf "$(RED)  需要 Homebrew: $(BREW_INSTALL)$(RESET)\n"; exit 1; }; \
		brew install mingw-w64; \
	}
endif
ifeq ($(PLATFORM),linux)
	@command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || { \
		printf "$(YELLOW)⚠ 安装 mingw-w64...$(RESET)\n"; \
		$(GIT_INSTALL:-y git=-y mingw-w64) 2>/dev/null || sudo apt-get install -y mingw-w64 || true; \
	}
endif

cross-macos: check-rust _ensure-target-macos
	@printf "$(BOLD)▶ 编译 macOS arm64$(RESET)\n"
	cargo build --release -p weflow-cli --target $(TARGET_MACOS_ARM)
	@cp target/$(TARGET_MACOS_ARM)/release/weflow weflow-macos-arm64
	@printf "$(GREEN)✓ → weflow-macos-arm64 ($$(ls -lh weflow-macos-arm64 | awk '{print $$5}'))$(RESET)\n"

cross-linux: check-rust _ensure-target-linux
	@printf "$(BOLD)▶ 编译 Linux x64$(RESET)\n"
ifeq ($(PLATFORM),macos)
	CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
	  cargo build --release -p weflow-cli --target $(TARGET_LINUX_X64)
else
	cargo build --release -p weflow-cli --target $(TARGET_LINUX_X64)
endif
	@cp target/$(TARGET_LINUX_X64)/release/weflow weflow-linux-x64
	@printf "$(GREEN)✓ → weflow-linux-x64 ($$(ls -lh weflow-linux-x64 | awk '{print $$5}'))$(RESET)\n"

cross-windows: check-rust _ensure-target-windows
	@printf "$(BOLD)▶ 编译 Windows x64 (GNU ABI)$(RESET)\n"
ifeq ($(PLATFORM),macos)
	CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
	  cargo build --release -p weflow-cli --target $(TARGET_WIN_X64)
else ifeq ($(PLATFORM),linux)
	CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
	  cargo build --release -p weflow-cli --target $(TARGET_WIN_X64)
else
	cargo build --release -p weflow-cli --target $(TARGET_WIN_X64_MS)
endif
	@cp target/$(TARGET_WIN_X64)/release/weflow.exe weflow-windows-x64.exe 2>/dev/null || \
	 cp target/$(TARGET_WIN_X64_MS)/release/weflow.exe weflow-windows-x64.exe
	@printf "$(GREEN)✓ → weflow-windows-x64.exe ($$(ls -lh weflow-windows-x64.exe | awk '{print $$5}'))$(RESET)\n"

cross-all: cross-macos cross-linux cross-windows
	@printf "\n$(GREEN)$(BOLD)所有平台构建完成:$(RESET)\n"
	@ls -lh weflow-macos-arm64 weflow-linux-x64 weflow-windows-x64.exe 2>/dev/null | awk '{printf "  %s  %s\n", $$5, $$9}'

# ─────────────────────────────────────────────────────────────────────────────
# CI / 环境信息
# ─────────────────────────────────────────────────────────────────────────────
.PHONY: ci env

ci: check check-tools test release
	@printf "$(GREEN)$(BOLD)✓ CI 全流程通过$(RESET)\n"

env:
	@printf "$(BOLD)WeFlow 构建环境$(RESET)\n"
	@printf "  平台:      $(PLATFORM)\n"
	@printf "  版本:      $(VERSION)\n"
	@printf "  输出:      $(BIN)\n"
	@printf "  安装目录:  $(INSTALL_DIR)\n"
	@printf "\n$(BOLD)工具版本$(RESET)\n"
	@command -v rustc  >/dev/null 2>&1 && printf "  rustc:   $$(rustc  --version)\n" || printf "  rustc:   $(RED)未安装$(RESET)\n"
	@command -v cargo  >/dev/null 2>&1 && printf "  cargo:   $$(cargo  --version)\n" || printf "  cargo:   $(RED)未安装$(RESET)\n"
	@command -v git    >/dev/null 2>&1 && printf "  git:     $$(git    --version)\n" || printf "  git:     $(RED)未安装$(RESET)\n"
	@command -v curl   >/dev/null 2>&1 && printf "  curl:    $$(curl   --version | head -1)\n" || printf "  curl:    $(RED)未安装$(RESET)\n"
	@printf "\n$(BOLD)已安装 Rust 目标$(RESET)\n"
	@rustup target list --installed 2>/dev/null | sed 's/^/  /' || printf "  $(DIM)rustup 不可用$(RESET)\n"
