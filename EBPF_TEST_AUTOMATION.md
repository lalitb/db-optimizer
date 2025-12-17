# eBPF Test Automation Guide

## 🎯 Problem: eBPF Requires Linux Kernel

eBPF programs run in the Linux kernel, so testing on macOS requires a Linux VM or CI environment.

---

## ✅ Solution Matrix

| Method | Speed | Setup | CI/CD | Cost | Recommended For |
|--------|-------|-------|-------|------|----------------|
| **GitHub Actions** | ⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ✅ | FREE | **Production CI** |
| **Lima VM** | ⭐⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ❌ | FREE | **macOS Local Dev** |
| **Multipass** | ⭐⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ❌ | FREE | **Quick Testing** |
| **Docker + Act** | ⭐⭐⭐ | ⭐⭐⭐ | ✅ | FREE | **Local CI Testing** |
| **Vagrant** | ⭐⭐ | ⭐⭐ | ❌ | FREE | **Legacy Projects** |
| **AWS EC2** | ⭐⭐⭐⭐ | ⭐⭐ | ✅ | $$ | **Production Tests** |

---

## 🚀 Option 1: GitHub Actions (RECOMMENDED)

### **Setup: 2 minutes**

**File created:** `.github/workflows/ebpf-tests.yml`

### **Features:**
- ✅ Runs on every push/PR
- ✅ Real Linux kernel (Ubuntu 22.04, kernel 5.15+)
- ✅ Free for public repos (2,000 minutes/month)
- ✅ Automatic PostgreSQL setup
- ✅ eBPF tools pre-installed
- ✅ Sudo access for eBPF programs

### **Usage:**

```bash
# Just push to GitHub!
git add .github/workflows/ebpf-tests.yml
git commit -m "Add eBPF CI tests"
git push origin main

# Tests run automatically
# View results at: https://github.com/YOUR_USERNAME/vaanyze/actions
```

### **What It Tests:**
1. ✅ Kernel eBPF support verification
2. ✅ Load 4.5M row test database
3. ✅ Run integration tests
4. ✅ eBPF-enabled analysis with sudo
5. ✅ pgbench workload generation
6. ✅ I/O breakdown validation
7. ✅ Auto-apply dry-run test
8. ✅ Kernel panic detection

### **Expected Runtime:** 8-12 minutes

### **Cost:** FREE (2,000 minutes/month for public repos)

---

## 🖥️ Option 2: Lima VM (BEST for macOS)

### **Setup: 5 minutes**

**File created:** `lima-vaanyze.yaml`

### **Install Lima:**

```bash
# Install via Homebrew
brew install lima

# Start Vaanyze VM
limactl start lima-vaanyze.yaml

# Wait 2-3 minutes for provisioning...

# Enter the VM
limactl shell vaanyze
```

### **What You Get:**

- ✅ Ubuntu 22.04 with kernel 5.15+
- ✅ 4 CPUs, 8GB RAM
- ✅ PostgreSQL 15 pre-installed
- ✅ eBPF tools (bpftool, libbpf-dev)
- ✅ Rust toolchain
- ✅ Port forwarding (PostgreSQL :5432)
- ✅ Your macOS home directory mounted

### **Usage:**

```bash
# Start VM
limactl start vaanyze

# Enter VM
limactl shell vaanyze

# Inside VM:
cd ~/personal/db-optimizer
cargo build --release

# Load test data
psql -U test -d test_vaanyze < scripts/test-data.sql

# Run eBPF tests
sudo ./target/release/db-optimizer \
  --database-url postgresql://test:test@localhost/test_vaanyze \
  --ebpf-enabled analyze

# Run integration tests
TEST_DATABASE_URL=postgresql://test:test@localhost/test_vaanyze \
  cargo test --ignored

# Exit VM
exit

# Stop VM (saves state)
limactl stop vaanyze

# Delete VM
limactl delete vaanyze
```

### **Automation Script:**

Create `scripts/test-ebpf-lima.sh`:

```bash
#!/bin/bash
set -e

echo "🚀 Starting Lima VM..."
limactl start vaanyze 2>/dev/null || echo "VM already running"

echo "📦 Building Vaanyze..."
limactl shell vaanyze bash -c "cd ~/personal/db-optimizer && cargo build --release"

echo "📊 Loading test data..."
limactl shell vaanyze bash -c "psql -U test -d test_vaanyze < ~/personal/db-optimizer/scripts/test-data.sql"

echo "🔬 Running eBPF tests..."
limactl shell vaanyze bash -c "cd ~/personal/db-optimizer && sudo ./target/release/db-optimizer --database-url postgresql://test:test@localhost/test_vaanyze --ebpf-enabled analyze"

echo "✅ eBPF tests completed!"
```

**Usage:**
```bash
chmod +x scripts/test-ebpf-lima.sh
./scripts/test-ebpf-lima.sh
```

### **Cost:** FREE

---

## 🐳 Option 3: Multipass (Simplest)

### **Setup: 3 minutes**

```bash
# Install Multipass
brew install multipass

# Launch Ubuntu VM
multipass launch 22.04 \
  --name vaanyze \
  --cpus 4 \
  --memory 8G \
  --disk 30G

# Install dependencies
multipass exec vaanyze -- bash -c '
  sudo apt-get update
  sudo apt-get install -y postgresql clang llvm libelf-dev libbpf-dev bpftool
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
'

# Mount your project
multipass mount ~/personal/db-optimizer vaanyze:/vaanyze

# Enter VM
multipass shell vaanyze
```

### **Usage:**

```bash
# Inside VM
cd /vaanyze
cargo build --release

# Setup PostgreSQL
sudo -u postgres createdb test_vaanyze
psql test_vaanyze < scripts/test-data.sql

# Run eBPF tests
sudo ./target/release/db-optimizer \
  --database-url postgresql://postgres@localhost/test_vaanyze \
  --ebpf-enabled analyze
```

### **Automation:**

```bash
# scripts/test-ebpf-multipass.sh
multipass exec vaanyze -- bash -c '
  cd /vaanyze
  cargo build --release
  psql test_vaanyze < scripts/test-data.sql
  sudo ./target/release/db-optimizer --ebpf-enabled analyze
'
```

### **Cost:** FREE

---

## 🎭 Option 4: Docker + Act (Local GitHub Actions)

**Run GitHub Actions locally on macOS!**

### **Setup:**

```bash
# Install Act
brew install act

# Run GitHub Actions locally
act -j ebpf-tests \
  --container-architecture linux/amd64 \
  -P ubuntu-22.04=catthehacker/ubuntu:act-22.04
```

### **Limitations:**
- ⚠️ Docker on macOS uses a Linux VM under the hood
- ⚠️ eBPF support may be limited
- ⚠️ Requires privileged mode

### **Usage:**

```bash
# Test your GitHub Actions workflow locally
act push

# Run specific job
act -j ebpf-tests

# With secrets
act -s DATABASE_URL=postgresql://test:test@localhost/test
```

### **Cost:** FREE

---

## 🏗️ Option 5: Cloud VMs (Production Testing)

### **AWS EC2 Spot Instances** (Cheapest)

```bash
# Launch Ubuntu 22.04 instance
aws ec2 run-instances \
  --image-id ami-0c7217cdde317cfec \
  --instance-type t3.medium \
  --key-name your-key \
  --instance-market-options MarketType=spot \
  --user-data '#!/bin/bash
    apt-get update
    apt-get install -y postgresql clang llvm libelf-dev libbpf-dev
    curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  '

# SSH into instance
ssh -i your-key.pem ubuntu@ec2-instance

# Run tests
git clone https://github.com/yourname/vaanyze
cd vaanyze
cargo build --release
sudo ./target/release/db-optimizer --ebpf-enabled analyze
```

### **Cost:** $0.01-0.05/hour (spot pricing)

### **Azure VMs** (Since you're at Microsoft)

```bash
# Create VM
az vm create \
  --resource-group vaanyze-tests \
  --name vaanyze-test-vm \
  --image Ubuntu2204 \
  --size Standard_D2s_v3 \
  --admin-username azureuser \
  --generate-ssh-keys

# SSH
ssh azureuser@<public-ip>

# Install dependencies
sudo apt-get update
sudo apt-get install -y postgresql clang llvm libelf-dev libbpf-dev bpftool
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# Run tests
git clone https://github.com/yourname/vaanyze
cd vaanyze
cargo build --release
sudo ./target/release/db-optimizer --ebpf-enabled analyze
```

### **Cost:** ~$70/month (or use Azure credits)

---

## 📊 Comparison: Which to Use?

### **For CI/CD (Production):**
1. ✅ **GitHub Actions** - Free, automated, real kernel
2. ✅ **GitLab CI** - Similar to GitHub Actions
3. ✅ **Azure Pipelines** - Since you're at Microsoft

### **For Local Development (macOS):**
1. ✅ **Lima** - Lightweight, fast, native
2. ✅ **Multipass** - Simple, Ubuntu-official
3. ✅ **Docker Desktop** - If you're already using it

### **For Production Testing:**
1. ✅ **AWS EC2 Spot** - Cheapest
2. ✅ **Azure VMs** - Microsoft credits
3. ✅ **Kubernetes** - For k8s support later

---

## 🎯 Recommended Setup

### **Phase 1: Quick Start (Today)**

Use **GitHub Actions** for automated CI:

```bash
# 1. Add workflow file (already created)
git add .github/workflows/ebpf-tests.yml
git commit -m "Add eBPF CI"
git push

# 2. Watch tests run
open https://github.com/YOUR_USERNAME/vaanyze/actions
```

**Time:** 2 minutes to setup, 10 minutes per test run
**Cost:** FREE

---

### **Phase 2: Local Testing (This Week)**

Use **Lima** for fast local testing:

```bash
# 1. Install Lima
brew install lima

# 2. Start VM
limactl start lima-vaanyze.yaml

# 3. Run tests
limactl shell vaanyze
cd ~/personal/db-optimizer
sudo ./target/release/db-optimizer --ebpf-enabled analyze
```

**Time:** 5 minutes setup, instant subsequent runs
**Cost:** FREE

---

### **Phase 3: Cross-Kernel Testing (Month 2)**

Test on multiple kernel versions:

**GitHub Actions Matrix:**

```yaml
strategy:
  matrix:
    kernel:
      - ubuntu-20.04  # Kernel 5.4
      - ubuntu-22.04  # Kernel 5.15
      - ubuntu-24.04  # Kernel 6.8
```

Add to `.github/workflows/ebpf-tests.yml`:

```yaml
jobs:
  ebpf-tests:
    runs-on: ${{ matrix.kernel }}
    strategy:
      matrix:
        kernel: [ubuntu-20.04, ubuntu-22.04, ubuntu-24.04]
```

---

## 🧪 Test Automation Scripts

### **1. Full Test Suite** (`scripts/run-all-tests.sh`)

```bash
#!/bin/bash
set -e

echo "🧪 Running Vaanyze Full Test Suite"

echo "1️⃣ Unit Tests..."
cargo test

echo "2️⃣ Integration Tests..."
TEST_DATABASE_URL=postgresql://test:test@localhost/test_vaanyze \
  cargo test --ignored

echo "3️⃣ eBPF Tests (Lima)..."
limactl shell vaanyze bash -c "
  cd ~/personal/db-optimizer
  sudo ./target/release/db-optimizer \
    --database-url postgresql://test:test@localhost/test_vaanyze \
    --ebpf-enabled analyze
"

echo "✅ All tests passed!"
```

### **2. Pre-commit Hook** (`.git/hooks/pre-push`)

```bash
#!/bin/bash
# Run tests before pushing

echo "🧪 Running pre-push tests..."

# Unit tests
cargo test || exit 1

# eBPF tests in Lima VM
if command -v limactl &> /dev/null; then
  limactl shell vaanyze bash -c "
    cd ~/personal/db-optimizer
    cargo test
  " || exit 1
fi

echo "✅ Pre-push tests passed!"
```

### **3. Continuous Testing** (`scripts/watch-and-test.sh`)

```bash
#!/bin/bash
# Watch for file changes and run tests

cargo watch -x test -x 'test --ignored'
```

---

## 📈 Performance Benchmarks

| Method | Boot Time | Test Time | Total |
|--------|-----------|-----------|-------|
| GitHub Actions | ~30s | 8-10min | **8-11min** |
| Lima | ~2min (first) | 2-3min | **2-5min** |
| Multipass | ~3min (first) | 2-3min | **3-6min** |
| Docker + Act | ~1min | 5-7min | **6-8min** |
| AWS EC2 | ~2min | 2-3min | **4-5min** |

**Winner:** Lima (after first boot, instant subsequent runs)

---

## ✅ Final Recommendation

### **For Launch (Next Week):**

1. ✅ **GitHub Actions** - Set up today for CI/CD
   ```bash
   git add .github/workflows/ebpf-tests.yml
   git push
   ```

2. ✅ **Lima VM** - Set up for local testing
   ```bash
   brew install lima
   limactl start lima-vaanyze.yaml
   ```

3. ✅ **Manual verification** - Run once before launch
   ```bash
   limactl shell vaanyze
   sudo ./target/release/db-optimizer --ebpf-enabled analyze
   ```

### **Total Setup Time:** 30 minutes
### **Total Cost:** FREE

---

## 🚨 Pre-Launch Checklist

- [ ] **GitHub Actions** set up (`.github/workflows/ebpf-tests.yml`)
- [ ] **Lima VM** running (`limactl start`)
- [ ] **Test data** loaded (`psql < test-data.sql`)
- [ ] **eBPF analysis** verified (I/O breakdown present)
- [ ] **Auto-apply** tested (dry-run mode)
- [ ] **Kernel compatibility** tested (5.4, 5.15, 6.x)
- [ ] **No kernel panics** (`dmesg` clean)
- [ ] **Memory leaks** checked (run for 1 hour)

---

## 🎯 Next Steps

**TODAY:**
1. Push GitHub Actions workflow
2. Install Lima
3. Run first automated test

**TOMORROW:**
1. Verify tests pass on GitHub
2. Test locally with Lima
3. Ready to launch!

Want me to help you set up GitHub Actions or Lima VM now?
