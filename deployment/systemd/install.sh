#!/bin/bash
# DB Optimizer Systemd Installation Script
# 
# This script installs db-optimizer as a systemd service on Linux systems.
# It should be run with sudo/root privileges.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
BINARY_PATH="/usr/local/bin/db-optimizer"
CONFIG_DIR="/etc/db-optimizer"
DATA_DIR="/var/lib/db-optimizer"
LOG_DIR="/var/log/db-optimizer"
SERVICE_FILE="/etc/systemd/system/db-optimizer.service"
USER="db-optimizer"
GROUP="db-optimizer"

echo -e "${GREEN}DB Optimizer Systemd Installation${NC}"
echo "===================================="

# Check if running as root
if [ "$EUID" -ne 0 ]; then 
    echo -e "${RED}Error: This script must be run as root${NC}"
    exit 1
fi

# Check if db-optimizer binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo -e "${YELLOW}Warning: db-optimizer binary not found at $BINARY_PATH${NC}"
    echo "Please install the binary first:"
    echo "  cargo build --release"
    echo "  sudo cp target/release/db-optimizer /usr/local/bin/"
    exit 1
fi

echo "✓ Found db-optimizer binary at $BINARY_PATH"

# Create user and group
if ! id -u "$USER" > /dev/null 2>&1; then
    echo "Creating user and group: $USER"
    useradd --system --no-create-home --shell /bin/false "$USER"
    echo "✓ Created user: $USER"
else
    echo "✓ User $USER already exists"
fi

# Create directories
echo "Creating directories..."
mkdir -p "$CONFIG_DIR"
mkdir -p "$DATA_DIR"
mkdir -p "$LOG_DIR"
mkdir -p /opt/db-optimizer

# Set permissions
chown -R "$USER:$GROUP" "$DATA_DIR"
chown -R "$USER:$GROUP" "$LOG_DIR"
chown -R "$USER:$GROUP" /opt/db-optimizer
chmod 755 "$CONFIG_DIR"
chmod 750 "$DATA_DIR"
chmod 750 "$LOG_DIR"

echo "✓ Created and configured directories"

# Copy config file if it doesn't exist
if [ ! -f "$CONFIG_DIR/config.yml" ]; then
    if [ -f "config.example.yml" ]; then
        echo "Copying example config to $CONFIG_DIR/config.yml"
        cp config.example.yml "$CONFIG_DIR/config.yml"
        chmod 640 "$CONFIG_DIR/config.yml"
        chown root:$GROUP "$CONFIG_DIR/config.yml"
        echo -e "${YELLOW}⚠ Please edit $CONFIG_DIR/config.yml with your database connection${NC}"
    else
        echo -e "${YELLOW}Warning: config.example.yml not found in current directory${NC}"
        echo "You'll need to create $CONFIG_DIR/config.yml manually"
    fi
else
    echo "✓ Config file already exists at $CONFIG_DIR/config.yml"
fi

# Copy systemd service file
echo "Installing systemd service..."
cp deployment/systemd/db-optimizer.service "$SERVICE_FILE"
chmod 644 "$SERVICE_FILE"
echo "✓ Installed systemd service file"

# Reload systemd
echo "Reloading systemd..."
systemctl daemon-reload
echo "✓ Systemd reloaded"

# Print next steps
echo ""
echo -e "${GREEN}Installation complete!${NC}"
echo ""
echo "Next steps:"
echo "  1. Edit configuration:  sudo nano $CONFIG_DIR/config.yml"
echo "  2. Set database URL:    sudo nano $CONFIG_DIR/environment"
echo "     Example content:     DBO_DATABASE_URL=postgresql://user:pass@localhost/db"
echo "  3. Enable service:      sudo systemctl enable db-optimizer"
echo "  4. Start service:       sudo systemctl start db-optimizer"
echo "  5. Check status:        sudo systemctl status db-optimizer"
echo "  6. View logs:           sudo journalctl -u db-optimizer -f"
echo ""
echo "Environment variables (optional):"
echo "  Create $CONFIG_DIR/environment with DBO_* variables:"
echo "    DBO_DATABASE_URL=postgresql://..."
echo "    DBO_OPERATION_MODE=observe_only"
echo "    DBO_GUARDRAILS_MEMORY_THRESHOLD=90"
echo "    DBO_HTTP_ENABLED=true"
echo "    DBO_HTTP_PORT=9090"
echo ""
