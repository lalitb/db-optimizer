#!/bin/bash
# install-ebpf-daemon.sh
# Install and configure the DB Optimizer eBPF daemon with privilege separation
#
# Usage: sudo ./install-ebpf-daemon.sh

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if running as root
if [[ $EUID -ne 0 ]]; then
    log_error "This script must be run as root"
    exit 1
fi

# Check kernel version for eBPF support
KERNEL_VERSION=$(uname -r | cut -d. -f1-2)
KERNEL_MAJOR=$(echo $KERNEL_VERSION | cut -d. -f1)
KERNEL_MINOR=$(echo $KERNEL_VERSION | cut -d. -f2)

if [[ $KERNEL_MAJOR -lt 5 ]] || ([[ $KERNEL_MAJOR -eq 5 ]] && [[ $KERNEL_MINOR -lt 8 ]]); then
    log_warn "Kernel version $KERNEL_VERSION detected. eBPF features work best with kernel 5.8+"
    log_warn "Some features may require CAP_SYS_ADMIN instead of CAP_BPF"
fi

log_info "Installing DB Optimizer eBPF Daemon..."

# Create dedicated user and group for the daemon
if ! getent group db-optimizer-ebpf >/dev/null 2>&1; then
    log_info "Creating group: db-optimizer-ebpf"
    groupadd --system db-optimizer-ebpf
fi

if ! getent passwd db-optimizer-ebpf >/dev/null 2>&1; then
    log_info "Creating user: db-optimizer-ebpf"
    useradd --system \
        --gid db-optimizer-ebpf \
        --home-dir /var/lib/db-optimizer \
        --shell /usr/sbin/nologin \
        --comment "DB Optimizer eBPF Daemon" \
        db-optimizer-ebpf
fi

# Create configuration directory
log_info "Creating configuration directory..."
mkdir -p /etc/db-optimizer
chmod 755 /etc/db-optimizer

# Create default config if it doesn't exist
if [[ ! -f /etc/db-optimizer/config.yml ]]; then
    log_info "Creating default configuration..."
    cat > /etc/db-optimizer/config.yml << 'EOF'
# DB Optimizer eBPF Daemon Configuration

ebpf:
  # Enable eBPF data collection
  enabled: true
  
  # Unix socket path for IPC
  socket_path: /run/db-optimizer/ebpf.sock
  
  # Sampling configuration
  sampling:
    # Initial sampling rate (1 = no sampling, N = sample 1-in-N events)
    initial_rate: 1
    # Enable adaptive sampling based on load
    adaptive: true
    # Target events per second (adaptive sampling adjusts to maintain this)
    target_events_per_sec: 10000
    # Maximum sampling rate (to prevent too aggressive sampling)
    max_rate: 100
  
  # Aggregation window
  aggregation_window_secs: 1
  
  # pg_stat_activity polling for PID correlation
  pg_stat:
    # How often to poll pg_stat_activity
    poll_interval_ms: 1000
    # Connection string for PostgreSQL
    connection_string: "host=/var/run/postgresql dbname=postgres"
  
  # Perf buffer sizes (per-CPU)
  perf_buffer:
    # Size in pages (each page is typically 4KB)
    pages: 64
  
  # BPF programs to load
  programs:
    - name: events
      type: tracepoint
      enabled: true
    - name: postgres
      type: uprobe
      enabled: true
      # Path to PostgreSQL binary (auto-detected if not specified)
      # binary_path: /usr/lib/postgresql/15/bin/postgres

# Logging
logging:
  level: info
  # Log to stderr (also goes to journald when running as systemd service)
  format: json
EOF
    chmod 644 /etc/db-optimizer/config.yml
    log_info "Default configuration created at /etc/db-optimizer/config.yml"
fi

# Create state directory
log_info "Creating state directory..."
mkdir -p /var/lib/db-optimizer
chown db-optimizer-ebpf:db-optimizer-ebpf /var/lib/db-optimizer
chmod 750 /var/lib/db-optimizer

# Install systemd service file
log_info "Installing systemd service..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cp "$SCRIPT_DIR/db-optimizer-ebpf.service" /etc/systemd/system/
chmod 644 /etc/systemd/system/db-optimizer-ebpf.service

# Reload systemd
log_info "Reloading systemd..."
systemctl daemon-reload

# Check if binary exists
if [[ ! -f /usr/local/bin/db-optimizer-ebpf-daemon ]]; then
    log_warn "Binary not found at /usr/local/bin/db-optimizer-ebpf-daemon"
    log_warn "Please build and install the binary:"
    log_warn "  cargo build --release --features ebpf"
    log_warn "  sudo cp target/release/db-optimizer-ebpf-daemon /usr/local/bin/"
else
    log_info "Binary found at /usr/local/bin/db-optimizer-ebpf-daemon"
fi

# Display kernel capabilities info
log_info ""
log_info "Installation complete!"
log_info ""
log_info "The daemon requires the following capabilities:"
log_info "  - CAP_BPF: Load and attach eBPF programs"
log_info "  - CAP_PERFMON: Access perf events"
log_info "  - CAP_SYS_PTRACE: Attach uprobes to PostgreSQL"
log_info ""
log_info "These capabilities are configured in the systemd service file."
log_info ""
log_info "To start the service:"
log_info "  sudo systemctl start db-optimizer-ebpf"
log_info ""
log_info "To enable auto-start on boot:"
log_info "  sudo systemctl enable db-optimizer-ebpf"
log_info ""
log_info "To view logs:"
log_info "  journalctl -u db-optimizer-ebpf -f"
log_info ""
log_info "Unprivileged clients can connect via:"
log_info "  /run/db-optimizer/ebpf.sock"

# Verify eBPF support
log_info ""
log_info "Checking eBPF support..."

if [[ -f /proc/config.gz ]]; then
    if zcat /proc/config.gz | grep -q "CONFIG_BPF=y"; then
        log_info "✓ CONFIG_BPF is enabled"
    else
        log_error "✗ CONFIG_BPF is not enabled"
    fi
    
    if zcat /proc/config.gz | grep -q "CONFIG_BPF_SYSCALL=y"; then
        log_info "✓ CONFIG_BPF_SYSCALL is enabled"
    else
        log_error "✗ CONFIG_BPF_SYSCALL is not enabled"
    fi
    
    if zcat /proc/config.gz | grep -q "CONFIG_HAVE_EBPF_JIT=y"; then
        log_info "✓ CONFIG_HAVE_EBPF_JIT is enabled"
    else
        log_warn "? CONFIG_HAVE_EBPF_JIT status unknown"
    fi
elif [[ -f /boot/config-$(uname -r) ]]; then
    if grep -q "CONFIG_BPF=y" /boot/config-$(uname -r); then
        log_info "✓ CONFIG_BPF is enabled"
    else
        log_error "✗ CONFIG_BPF is not enabled"
    fi
else
    log_warn "Could not verify kernel configuration"
fi

# Check BTF support (required for CO-RE)
if [[ -f /sys/kernel/btf/vmlinux ]]; then
    log_info "✓ BTF is available (CO-RE compatible)"
else
    log_warn "? BTF not found at /sys/kernel/btf/vmlinux"
    log_warn "  CO-RE may not work. Consider installing kernel debug info."
fi
