#!/bin/bash
set -e

echo "Building Agent for Ubuntu x86_64 using Docker Buildx..."

# 1. Build the Docker image targeting linux/amd64
docker buildx build --platform linux/amd64 -t sys-manager-agent-x86_64 -f Dockerfile.agent.ubuntu-x86_64 .

# 2. Create a temporary container to extract the compiled binary
docker create --name temp-agent sys-manager-agent-x86_64

# 3. Copy the compiled binary out of the container to the host machine
docker cp temp-agent:/app/agent ./agent-x86_64-linux

# 4. Clean up the temporary container
docker rm temp-agent

echo "Success! The compiled binary is available at ./agent-x86_64-linux"
echo "You can copy this file to your Ubuntu x86_64 servers."
