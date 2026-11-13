# ─────────────────────────────────────────────────────────────────────────────
# WeFlow Native CLI — Cross-platform Makefile
# 支持平台: macOS · Linux · Windows (Git Bash / WSL)
# 用法: make help
# ─────────────────────────────────────────────────────────────────────────────

# ── 平台检测 ──────────────────────────────────────────────────────────────────
ifeq ($(OS),Windows_NT)
  PLATFORM     := windows
  BIN          := target/release/weflow.exe
  DEBUG_BIN    := target/debug/weflow.exe
  # Forward slashes: printf would read the backslashes of a Windows path as escapes (\U of C:\Users).
  INSTALL_DIR  := $(subst \,/,$(USERPROFILE))/.cargo/bin
  GIT_INSTALL  := winget install -e --id Git.Git --accept-package-agreements --accept-source-agreements
  RUST_INSTALL := powershell -NoProfile -Command \
    "Invoke-WebRequest -Uri https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe -OutFile rustup-init.exe; \
     Start-Process -Wait -FilePath rustup-init.exe -ArgumentList '-y'; \
     Remove-Item rustup-init.exe"
else
  UNAME     := $(shell uname -s)
  BIN       := target/release/weflow
  DEBUG_BIN := target/debug/weflow
  ifeq ($(UNAME),Darwin)
    PLATFORM    := macos
    INSTALL_DIR := /usr/local/bin
    RUST_INSTALL := curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    BREW_INSTALL := /bin/bash -c "$$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  else
    PLATFORM    := linux
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
# cross-windows builds the GNU target on macOS/Linux (linked by MinGW) and the MSVC target on Windows itself
ifeq ($(PLATFORM),windows)
  WIN_TARGET := $(TARGET_WIN_X64_MS)
  CROSS_WINDOWS_ENV :=
else
  WIN_TARGET := $(TARGET_WIN_X64)
  CROSS_WINDOWS_ENV := CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
endif
# cross-linux needs a cross linker (and C compiler for the vendored C code) unless the host already is x86_64 Linux:
# without CARGO_TARGET_*_LINKER rustc links with the host's `cc`, which fails for another architecture.
# On Windows it builds inside WSL instead, whose own toolchain is that linker and C compiler; the build cache stays
# on WSL's disk (WSL_TARGET_DIR, a path inside WSL), far faster than one under /mnt/c. A WSL that is x86_64 Linux
# itself builds for its host, in the folders its other builds use. `wsl.exe -e` hands the script to bash as it is:
# without it WSL's default shell expands the script's variables first, in a shell that has no cargo.
WSL_TARGET_DIR ?= $$HOME/wf-target
CROSS_LINUX_ENV :=
ifeq ($(PLATFORM),macos)
  CROSS_LINUX_ENV := CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc
else ifeq ($(PLATFORM),linux)
  ifeq ($(filter x86_64 amd64,$(shell uname -m)),)
    CROSS_LINUX_ENV := CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc
  endif
endif

.DEFAULT_GOAL := help

# ─────────────────────────────────────────────────────────────────────────────
# 帮助
# ─────────────────────────────────────────────────────────────────────────────
define HELP_TEXT
$(BOLD)WeFlow CLI v$(VERSION) — $(PLATFORM)$(RESET)

$(BOLD)基础命令$(RESET)
  $(CYAN)make build$(RESET)           当前平台 debug 构建 → $(DEBUG_BIN)
  $(CYAN)make release$(RESET)         当前平台 release 构建 → $(BIN)
  $(CYAN)make test$(RESET)            运行全部测试（单元测试和端到端测试）
  $(CYAN)make check$(RESET)           cargo check（只检查，不编译）
  $(CYAN)make docs-check$(RESET)      检查指定 Markdown 文档的一致性
  $(CYAN)make fmt$(RESET)             格式化代码
  $(CYAN)make fmt-check$(RESET)       检查代码是否已按 rustfmt 格式化
  $(CYAN)make lint$(RESET)            Clippy 静态分析
  $(CYAN)make rustdoc$(RESET)         生成 API 文档，rustdoc 的警告视为错误
  $(CYAN)make clean$(RESET)           清理 target/

$(BOLD)安装$(RESET)
  $(CYAN)make install$(RESET)         将 release binary 安装到 $(INSTALL_DIR)
  $(CYAN)make uninstall$(RESET)       从 $(INSTALL_DIR) 删除 weflow

$(BOLD)跨平台编译$(RESET)
  $(CYAN)make cross-macos$(RESET)     → weflow-macos-arm64     ($(TARGET_MACOS_ARM))
  $(CYAN)make cross-linux$(RESET)     → weflow-linux-x64       ($(TARGET_LINUX_X64))
  $(CYAN)make cross-windows$(RESET)   → weflow-windows-x64.exe ($(WIN_TARGET))
  $(CYAN)make cross-all$(RESET)       构建所有跨平台目标

$(BOLD)环境$(RESET)
  $(CYAN)make check-tools$(RESET)     检查并自动安装所有必要工具
  $(CYAN)make env$(RESET)             显示当前环境信息
  $(CYAN)make ci$(RESET)              代码 CI，与 GitHub Actions 一致 (fmt-check+lint+rustdoc+test；文档检查另跑 make docs-check)

endef
export HELP_TEXT

.PHONY: help
help:
	@printf '%b' "$$HELP_TEXT"

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
		$(GIT_INSTALL) || { \
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
.PHONY: build release test check fmt fmt-check lint rustdoc clean docs-check

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

docs-check: check-rust
	@printf "$(BOLD)▶ 检查指定 Markdown 文档的一致性$(RESET)\n"
	cargo test -p weflow-native --test docs_consistency

check: check-rust
	@printf "$(BOLD)▶ cargo check --workspace$(RESET)\n"
	cargo check --workspace

fmt: check-rust
	@printf "$(BOLD)▶ cargo fmt$(RESET)\n"
	cargo fmt --all

# The code follows rustfmt's default style. rustfmt is a rustup component: install it when it is missing, so the
# check cannot pass by not running.
fmt-check: check-rust
	@printf "$(BOLD)▶ cargo fmt --check$(RESET)\n"
	@cargo fmt --version >/dev/null 2>&1 || rustup component add rustfmt
	cargo fmt --all -- --check

lint: check-rust
	@printf "$(BOLD)▶ cargo clippy$(RESET)\n"
	@cargo clippy --version >/dev/null 2>&1 || rustup component add clippy
	cargo clippy --workspace --all-targets -- -D warnings

# The API documentation with rustdoc's warnings as errors: a link to a private or unknown item, or a doc comment
# read as an unclosed HTML tag, fails the check.
rustdoc: check-rust
	@printf "$(BOLD)▶ cargo doc$(RESET)\n"
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

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
ifneq ($(PLATFORM),macos)
	@printf "$(RED)✗ 交叉编译 macOS 需要 macOS 的 SDK，请在 macOS 上运行 make cross-macos$(RESET)\n"; exit 1
endif
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
ifneq ($(CROSS_LINUX_ENV),)
	@command -v x86_64-linux-gnu-gcc >/dev/null 2>&1 || { \
		command -v apt-get >/dev/null 2>&1 || { \
			printf "$(RED)✗ 需要 x86_64-linux-gnu-gcc 交叉编译器，请用发行版的包管理器安装$(RESET)\n"; exit 1; }; \
		printf "$(YELLOW)⚠ 安装 gcc-x86-64-linux-gnu...$(RESET)\n"; \
		sudo apt-get install -y gcc-x86-64-linux-gnu || { printf "$(RED)  安装失败$(RESET)\n"; exit 1; }; \
	}
endif
endif

_ensure-target-windows:
	@rustup target list --installed | grep -q "$(WIN_TARGET)" || { \
		printf "$(YELLOW)⚠ 安装编译目标 $(WIN_TARGET)...$(RESET)\n"; \
		rustup target add $(WIN_TARGET); \
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
		command -v apt-get >/dev/null 2>&1 || { \
			printf "$(RED)✗ 需要 x86_64-w64-mingw32-gcc (mingw-w64)，请用发行版的包管理器安装$(RESET)\n"; exit 1; }; \
		printf "$(YELLOW)⚠ 安装 mingw-w64...$(RESET)\n"; \
		sudo apt-get install -y mingw-w64 || { printf "$(RED)  安装失败$(RESET)\n"; exit 1; }; \
	}
endif

cross-macos: check-rust _ensure-target-macos
	@printf "$(BOLD)▶ 编译 macOS arm64$(RESET)\n"
	cargo build --release -p weflow-cli --target $(TARGET_MACOS_ARM)
	@cp target/$(TARGET_MACOS_ARM)/release/weflow weflow-macos-arm64
	@printf "$(GREEN)✓ → weflow-macos-arm64 ($$(ls -lh weflow-macos-arm64 | awk '{print $$5}'))$(RESET)\n"

ifeq ($(PLATFORM),windows)
cross-linux:
	@command -v wsl.exe >/dev/null 2>&1 || { \
		printf "$(RED)✗ 在 Windows 上编译 Linux 版本需要 WSL: wsl --install$(RESET)\n"; exit 1; }
	@printf "$(BOLD)▶ 编译 Linux x64（在 WSL 中）$(RESET)\n"
	MSYS_NO_PATHCONV=1 wsl.exe -e bash -lc '. "$$HOME/.cargo/env" 2>/dev/null; \
	  command -v cargo >/dev/null || { echo "WSL 中未找到 cargo，请先在 WSL 里安装 Rust: https://rustup.rs" >&2; exit 1; }; \
	  export CARGO_TARGET_DIR="$(WSL_TARGET_DIR)"; \
	  if [ "$$(rustc -vV | sed -n "s/^host: //p")" = $(TARGET_LINUX_X64) ]; then target=; out=release; \
	  else target="--target $(TARGET_LINUX_X64)"; out=$(TARGET_LINUX_X64)/release; fi; \
	  cargo build --release -p weflow-cli $$target && cp "$$CARGO_TARGET_DIR/$$out/weflow" weflow-linux-x64'
	@printf "$(GREEN)✓ → weflow-linux-x64 ($$(ls -lh weflow-linux-x64 | awk '{print $$5}'))$(RESET)\n"
else
cross-linux: check-rust _ensure-target-linux
	@printf "$(BOLD)▶ 编译 Linux x64$(RESET)\n"
	$(CROSS_LINUX_ENV) cargo build --release -p weflow-cli --target $(TARGET_LINUX_X64)
	@cp target/$(TARGET_LINUX_X64)/release/weflow weflow-linux-x64
	@printf "$(GREEN)✓ → weflow-linux-x64 ($$(ls -lh weflow-linux-x64 | awk '{print $$5}'))$(RESET)\n"
endif

cross-windows: check-rust _ensure-target-windows
	@printf "$(BOLD)▶ 编译 Windows x64 ($(WIN_TARGET))$(RESET)\n"
	$(CROSS_WINDOWS_ENV) cargo build --release -p weflow-cli --target $(WIN_TARGET)
	@cp target/$(WIN_TARGET)/release/weflow.exe weflow-windows-x64.exe
	@printf "$(GREEN)✓ → weflow-windows-x64.exe ($$(ls -lh weflow-windows-x64.exe | awk '{print $$5}'))$(RESET)\n"

cross-all: cross-macos cross-linux cross-windows
	@printf "\n$(GREEN)$(BOLD)所有平台构建完成:$(RESET)\n"
	@ls -lh weflow-macos-arm64 weflow-linux-x64 weflow-windows-x64.exe 2>/dev/null | awk '{printf "  %s  %s\n", $$5, $$9}'

# ─────────────────────────────────────────────────────────────────────────────
# CI / 环境信息
# ─────────────────────────────────────────────────────────────────────────────
.PHONY: ci env

ci: fmt-check lint rustdoc test
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
