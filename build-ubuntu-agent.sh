#!/bin/bash
set -euo pipefail

# Cross-compile the agent for Ubuntu x86_64 using the cross-toolchain
# inside Dockerfile.agent.ubuntu-x86_64. The Dockerfile runs natively on
# whatever arch the build host is and emits an amd64 binary via
# g++-x86-64-linux-gnu, so we deliberately do NOT pass --platform here:
# QEMU emulation is not needed and just slows the build down.

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

echo "Building Agent for Ubuntu x86_64..."
docker buildx build \
  --output "type=local,dest=${OUT_DIR}" \
  -f Dockerfile.agent.ubuntu-x86_64 \
  .

cp "${OUT_DIR}/agent-x86_64-linux" ./agent-x86_64-linux
chmod +x ./agent-x86_64-linux
echo "Success. Binary: ./agent-x86_64-linux"
file ./agent-x86_64-linux
