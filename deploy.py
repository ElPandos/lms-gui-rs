#!/usr/bin/env python3
"""Build and deploy lms-gui-rs to remote host."""

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

# === Configuration ===
BINARY_NAME = "lms-gui-rs"
_remote_user = os.environ["ENV_USER_JUMP_155_HOST"]
_remote_ip = os.environ["ENV_IP_JUMP_155_HOST"]
REMOTE_HOST = f"{_remote_user}@{_remote_ip}"
REMOTE_DIR = f"/home/{_remote_user}/lms-gui-rs"
REMOTE_PORT = 3000
HEALTH_CHECK_PORT = REMOTE_PORT
HEALTH_CHECK_RETRIES = 5
HEALTH_CHECK_DELAY = 3
IMMEDIATE_CHECK_DELAY = 1.0


def ssh(cmd: str, silent: bool = False) -> tuple[str, int]:
    """Execute command on remote host via SSH.

    Args:
        cmd: Command to execute remotely.
        silent: Suppress output if True.

    Returns:
        Tuple of (stdout, exit_code).
    """
    result = subprocess.run(
        ["ssh", "-o", "ConnectTimeout=5", REMOTE_HOST, cmd],
        capture_output=True, text=True, timeout=30,
    )
    out = result.stdout.strip()
    err = result.stderr.strip()
    if not silent:
        if out:
            print(f"  {out}")
        if err and result.returncode != 0:
            print(f"  stderr: {err}")
    return out, result.returncode


def check_process_running() -> bool:
    """Check if process is running on remote host."""
    out, code = ssh(f"pgrep -f '{BINARY_NAME}'", silent=True)
    return code == 0 and out != ""


def check_port_listening() -> bool:
    """Check if port is listening on remote host."""
    out, code = ssh(f"ss -tuln | grep ':{HEALTH_CHECK_PORT}' || true", silent=True)
    return "LISTEN" in out


def capture_logs(lines: int = 20) -> None:
    """Capture and display recent log entries."""
    print(f"\n📋 Last {lines} lines from log:")
    print("=" * 70)
    ssh(f"tail -n {lines} {REMOTE_DIR}/{BINARY_NAME}.log 2>/dev/null || echo 'No log file'")
    print("=" * 70)


def check_remote_compatibility() -> bool:
    """Verify remote host compatibility."""
    print("🔍 Checking remote system compatibility...")
    out, code = ssh("uname -m", silent=True)
    if code == 0:
        print(f"  ℹ️  Architecture: {out}")
    out, code = ssh("lsb_release -r -s 2>/dev/null || cat /etc/os-release | grep VERSION_ID | cut -d= -f2", silent=True)
    if code == 0:
        print(f"  ℹ️  OS Version: {out}")
    out, code = ssh("ldd --version 2>&1 | head -1", silent=True)
    if code == 0:
        print(f"  ✅ libc: {out}")
    print()
    return True


def run_diagnostics() -> None:
    """Run comprehensive diagnostics on remote host."""
    print("\n" + "=" * 70)
    print("🔬 DIAGNOSTIC MODE")
    print("=" * 70)

    print("\n📊 System Information:")
    ssh("uname -a")
    ssh("cat /etc/os-release | head -3")

    print(f"\n📦 Binary: {REMOTE_DIR}/{BINARY_NAME}")
    ssh(f"ls -lh {REMOTE_DIR}/{BINARY_NAME} 2>/dev/null || echo 'Not found'")
    ssh(f"file {REMOTE_DIR}/{BINARY_NAME} 2>/dev/null || true")

    print("\n📚 Library Dependencies:")
    out, _ = ssh(f"ldd {REMOTE_DIR}/{BINARY_NAME} 2>&1 | grep 'not found' || echo 'All dependencies satisfied'", silent=True)
    print(f"  {out}")

    print("\n🧠 Memory:")
    ssh("free -h | head -2")

    print("\n🎮 GPU:")
    ssh("nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader 2>/dev/null || echo 'No nvidia-smi'")

    print(f"\n🌐 Port {HEALTH_CHECK_PORT} Status:")
    ssh(f"ss -tuln | grep {HEALTH_CHECK_PORT} || echo 'Port available'")

    print("\n🔍 Process Status:")
    ssh(f"pgrep -fa {BINARY_NAME} || echo 'No process running'")

    print("=" * 70 + "\n")


def print_manual_commands() -> None:
    """Print manual process management commands."""
    remote_path = f"{REMOTE_DIR}/{BINARY_NAME}"
    remote_log = f"{REMOTE_DIR}/{BINARY_NAME}.log"
    print("\n" + "=" * 70)
    print("MANUAL PROCESS MANAGEMENT")
    print("=" * 70)
    print(f"\n📍 SSH Connection:\n   ssh {REMOTE_HOST}")
    print(f"\n🔍 Check Status:\n   pgrep -fa {BINARY_NAME}")
    print(f"\n🛑 Stop:\n   pkill -f {BINARY_NAME}")
    print(f"\n▶️  Start:\n   cd {REMOTE_DIR} && setsid env LMS_LOCAL=1 ./{BINARY_NAME} > {remote_log} 2>&1 < /dev/null &")
    print(f"\n📋 Logs:\n   tail -f {remote_log}")
    print(f"\n🌐 Health:\n   curl http://localhost:{HEALTH_CHECK_PORT}/")
    print("=" * 70 + "\n")


# === Build ===


def build() -> bool:
    """Build release binary with cargo.

    Returns:
        True if build succeeded.
    """
    print("🔨 Building release binary...", flush=True)
    proc = subprocess.Popen(
        ["cargo", "build", "--release"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
    )
    # Show a spinner while building
    spinner = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
    i = 0
    while proc.poll() is None:
        print(f"\r  {spinner[i % len(spinner)]} Compiling...", end="", flush=True)
        i += 1
        time.sleep(0.1)
    print("\r  ", end="")

    if proc.returncode != 0:
        out = proc.stdout.read() if proc.stdout else ""
        print(f"❌ Build failed:\n{out[-500:]}")
        return False
    size = Path(f"target/release/{BINARY_NAME}").stat().st_size / 1024 / 1024
    print(f"✅ Build complete ({size:.1f} MB)")
    return True


# === Deploy ===


def deploy(diagnostics: bool = False, skip_build: bool = False) -> bool:
    """Deploy binary to remote host with full lifecycle management.

    Args:
        diagnostics: Run diagnostics only.
        skip_build: Skip the build step.

    Returns:
        True if deployment succeeded.
    """
    binary = Path(f"target/release/{BINARY_NAME}")
    remote_path = f"{REMOTE_DIR}/{BINARY_NAME}"
    remote_log = f"{REMOTE_DIR}/{BINARY_NAME}.log"

    print(f"\n{'='*70}")
    print(f"🚀 DEPLOYING {BINARY_NAME}")
    print(f"{'='*70}")
    print(f"Target: {REMOTE_HOST}:{REMOTE_DIR}")
    print(f"{'='*70}\n")

    print("🔌 Connecting to remote host...")
    out, code = ssh("echo ok", silent=True)
    if code != 0:
        print(f"❌ Cannot connect to {REMOTE_HOST}")
        return False
    print("✅ Connected\n")

    if not check_remote_compatibility():
        return False

    if diagnostics:
        run_diagnostics()
        return True

    # Show changelog
    _show_deploy_changelog()

    if not skip_build:
        answer = input("🔨 Run build before deploying? [Y/n]: ").strip().lower()
        if answer != "n":
            if not build():
                return False
            print()

    if not binary.exists():
        print(f"❌ Binary not found: {binary}")
        return False

    # [1/5] Stop existing
    print("[1/5] 🛑 Stopping existing process...")
    if check_process_running():
        ssh(f"pkill -f '{BINARY_NAME}'", silent=True)
        time.sleep(2)
        if check_process_running():
            ssh(f"pkill -9 -f '{BINARY_NAME}'", silent=True)
            time.sleep(1)
        print("  ✅ Process stopped")
    else:
        print("  ℹ️  No process running")
    print()

    # [2/5] Upload
    print("[2/5] 📦 Uploading binary...")
    ssh(f"mkdir -p {REMOTE_DIR}", silent=True)

    # Backup
    timestamp, _ = ssh("date +%Y%m%d_%H%M%S", silent=True)
    backup_path = f"{remote_path}.{timestamp}"
    ssh(f"test -f {remote_path} && mv {remote_path} {backup_path} && echo 'backed up' || true", silent=True)

    size_mb = binary.stat().st_size / 1024 / 1024
    print(f"  Uploading {size_mb:.1f} MB...", end="", flush=True)
    proc = subprocess.Popen(
        ["scp", "-o", "ConnectTimeout=5", str(binary), f"{REMOTE_HOST}:{remote_path}"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    spinner = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
    i = 0
    while proc.poll() is None:
        print(f"\r  {spinner[i % len(spinner)]} Uploading {size_mb:.1f} MB...", end="", flush=True)
        i += 1
        time.sleep(0.1)
    print("\r  ", end="")

    if proc.returncode != 0:
        print(f"❌ Upload failed: {proc.stderr.read().decode()}")
        return False
    ssh(f"chmod +x {remote_path}", silent=True)
    print(f"✅ Uploaded ({size_mb:.1f} MB)")
    print()

    # [3/5] Verify
    print("[3/5] 🔍 Pre-deployment verification...")
    out, code = ssh(f"ldd {remote_path} 2>&1 | grep 'not found'", silent=True)
    if "not found" in out:
        print(f"  ❌ Missing libraries: {out[:200]}")
        return False
    print("  ✅ Binary executable, dependencies satisfied")
    print()

    # [4/5] Start
    print("[4/5] ▶️  Starting process...")
    # Fire-and-forget: start the process without waiting for SSH to close
    # SSH hangs because the Rust binary inherits stdout fd; use Popen + short wait
    proc = subprocess.Popen(
        ["ssh", "-o", "ConnectTimeout=5", REMOTE_HOST,
         f"cd {REMOTE_DIR} && LMS_LOCAL=1 nohup ./{BINARY_NAME} > {remote_log} 2>&1 & disown; sleep 1; echo started"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()  # Kill the local SSH process — remote process is already running
    time.sleep(IMMEDIATE_CHECK_DELAY)

    if not check_process_running():
        print("  ❌ Process died immediately!")
        capture_logs(30)
        return False
    out, _ = ssh(f"pgrep -f '{BINARY_NAME}'", silent=True)
    print(f"  ✅ Started (PID: {out})")
    print()

    # [5/5] Health check
    print(f"[5/5] 🏥 Health checks (port {HEALTH_CHECK_PORT})...")
    healthy = False
    for attempt in range(1, HEALTH_CHECK_RETRIES + 1):
        print(f"  Attempt {attempt}/{HEALTH_CHECK_RETRIES}...", end=" ", flush=True)

        if not check_process_running():
            print("❌ Process died")
            capture_logs()
            break

        if check_port_listening():
            print(f"✅ Port {HEALTH_CHECK_PORT} listening")
            healthy = True
            break

        print("⏳ Waiting...")
        time.sleep(HEALTH_CHECK_DELAY)

    print()
    if healthy:
        host = REMOTE_HOST.split("@")[-1] if "@" in REMOTE_HOST else REMOTE_HOST
        print("✅ DEPLOYMENT SUCCESSFUL")
        print(f"   URL: http://{host}:{REMOTE_PORT}")
        print(f"   Log: {remote_log}")
        _save_deployed_hash()
    else:
        print("⚠️  DEPLOYMENT FAILED")
        capture_logs(30)
        if backup_path:
            print(f"\n   To rollback: ssh {REMOTE_HOST} 'mv {backup_path} {remote_path}'")

    print_manual_commands()
    return healthy


def _show_deploy_changelog() -> None:
    """Show commits since last deployment."""
    out, code = ssh(f"cat {REMOTE_DIR}/.deployed_hash 2>/dev/null", silent=True)
    if code != 0 or not out:
        print("📋 First deployment\n")
        return

    try:
        result = subprocess.run(
            ["git", "log", "--oneline", f"{out}..HEAD"],
            capture_output=True, text=True, timeout=5,
        )
        if result.returncode == 0 and result.stdout.strip():
            lines = result.stdout.strip().splitlines()
            print(f"📋 Changes since last deploy ({len(lines)} commits):")
            for line in lines[:15]:
                print(f"   {line}")
            print()
    except (subprocess.SubprocessError, OSError):
        pass


def _save_deployed_hash() -> None:
    """Save current git hash to remote."""
    result = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        capture_output=True, text=True,
    )
    if result.returncode == 0:
        ssh(f"echo '{result.stdout.strip()}' > {REMOTE_DIR}/.deployed_hash", silent=True)


# === CLI ===


def main() -> None:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description=f"Build and deploy {BINARY_NAME}")
    parser.add_argument("--build-only", action="store_true", help="Build only (no deployment)")
    parser.add_argument("--skip-build", action="store_true", help="Skip cargo build")
    parser.add_argument("--diagnostics", action="store_true", help="Run diagnostics only")
    args = parser.parse_args()

    if args.build_only:
        sys.exit(0 if build() else 1)

    success = deploy(diagnostics=args.diagnostics, skip_build=args.skip_build)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()
