# eBPF Build Pipeline for db-optimizer
# Compiles .bpf.c files to .bpf.o bytecode using clang/LLVM
#
# Requirements:
#   - clang (>= 11.0)
#   - llvm (for llvm-strip)
#   - libbpf-dev (headers)
#   - linux-headers (for BTF)
#
# Usage:
#   make              - Build all eBPF programs
#   make clean        - Remove compiled objects
#   make check        - Verify toolchain
#   make install      - Install to /usr/local/lib/db-optimizer/

# Toolchain
CLANG ?= clang
LLC ?= llc
LLVM_STRIP ?= llvm-strip
BPFTOOL ?= bpftool

# Directories
BPF_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
OUTPUT_DIR := $(BPF_DIR)/.output
LIBBPF_DIR := /usr/include/bpf
INSTALL_DIR := /usr/local/lib/db-optimizer

# Compiler flags
ARCH := $(shell uname -m | sed 's/x86_64/x86/' | sed 's/aarch64/arm64/')
CFLAGS := -g -O2 -target bpf -D__TARGET_ARCH_$(ARCH)
CFLAGS += -Wall -Werror
CFLAGS += -I$(LIBBPF_DIR)
CFLAGS += -I/usr/include
CFLAGS += -I$(OUTPUT_DIR)

# CO-RE flags for portability
CFLAGS += -mcpu=v3  # BPF instruction set v3 for better compatibility

# Debug flags (optional)
ifdef DEBUG
CFLAGS += -DDEBUG -g
endif

# Source files
BPF_SRCS := $(wildcard $(BPF_DIR)*.bpf.c)
BPF_OBJS := $(patsubst %.bpf.c,$(OUTPUT_DIR)/%.bpf.o,$(notdir $(BPF_SRCS)))

# VMLinux header (for BTF/CO-RE)
VMLINUX_H := $(OUTPUT_DIR)/vmlinux.h

# Phony targets
.PHONY: all clean check install vmlinux help

# Default target
all: $(OUTPUT_DIR) $(BPF_OBJS)
	@echo "✓ Built $(words $(BPF_OBJS)) eBPF programs"

# Create output directory
$(OUTPUT_DIR):
	@mkdir -p $(OUTPUT_DIR)

# Generate vmlinux.h from kernel BTF (optional, for full CO-RE)
vmlinux: $(VMLINUX_H)

$(VMLINUX_H): $(OUTPUT_DIR)
	@echo "Generating vmlinux.h from kernel BTF..."
	@if [ -f /sys/kernel/btf/vmlinux ]; then \
		$(BPFTOOL) btf dump file /sys/kernel/btf/vmlinux format c > $@; \
		echo "✓ Generated $(VMLINUX_H)"; \
	else \
		echo "⚠ BTF not available, using minimal headers"; \
		touch $@; \
	fi

# Compile eBPF programs
$(OUTPUT_DIR)/%.bpf.o: $(BPF_DIR)%.bpf.c $(OUTPUT_DIR)
	@echo "Compiling $<..."
	$(CLANG) $(CFLAGS) -c $< -o $@
	@if command -v $(LLVM_STRIP) >/dev/null 2>&1; then \
		$(LLVM_STRIP) -g $@; \
	fi
	@echo "✓ $@"

# Check toolchain
check:
	@echo "Checking eBPF build toolchain..."
	@echo -n "  clang: "; $(CLANG) --version | head -1 || echo "NOT FOUND"
	@echo -n "  llvm-strip: "; $(LLVM_STRIP) --version 2>/dev/null | head -1 || echo "NOT FOUND (optional)"
	@echo -n "  bpftool: "; $(BPFTOOL) version 2>/dev/null || echo "NOT FOUND (optional)"
	@echo -n "  libbpf headers: "; test -f $(LIBBPF_DIR)/bpf_helpers.h && echo "OK" || echo "NOT FOUND"
	@echo -n "  kernel BTF: "; test -f /sys/kernel/btf/vmlinux && echo "OK" || echo "NOT AVAILABLE"
	@echo -n "  architecture: "; echo $(ARCH)

# Install compiled programs
install: all
	@echo "Installing eBPF programs to $(INSTALL_DIR)..."
	@mkdir -p $(INSTALL_DIR)
	@cp $(OUTPUT_DIR)/*.bpf.o $(INSTALL_DIR)/
	@echo "✓ Installed to $(INSTALL_DIR)"

# Clean build artifacts
clean:
	@echo "Cleaning eBPF build artifacts..."
	@rm -rf $(OUTPUT_DIR)
	@echo "✓ Cleaned"

# Help
help:
	@echo "eBPF Build Pipeline for db-optimizer"
	@echo ""
	@echo "Targets:"
	@echo "  all      - Build all eBPF programs (default)"
	@echo "  clean    - Remove compiled objects"
	@echo "  check    - Verify toolchain availability"
	@echo "  install  - Install to $(INSTALL_DIR)"
	@echo "  vmlinux  - Generate vmlinux.h from kernel BTF"
	@echo "  help     - Show this help"
	@echo ""
	@echo "Environment:"
	@echo "  CLANG=$(CLANG)"
	@echo "  DEBUG=1 for debug builds"
	@echo ""
	@echo "Programs:"
	@for f in $(BPF_SRCS); do echo "  - $$(basename $$f)"; done
