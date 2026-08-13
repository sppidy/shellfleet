#!/usr/bin/env bash
# The release-workflow assertion intentionally matches literal shell syntax.
# shellcheck disable=SC2016
set -euo pipefail

chart_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
repo_dir=$(cd "$chart_dir/../.." && pwd)
test_dir=$(mktemp -d)
trap 'rm -rf -- "$test_dir"' EXIT

agent_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_dir/agent/Cargo.toml" | head -n 1)
chart_version=$(sed -n 's/^appVersion: *//p' "$chart_dir/Chart.yaml" | head -n 1)
test -n "$agent_version"
test "$chart_version" = "$agent_version"
grep -Fq 'DEST+=("${REGISTRY}/${IMAGE_NAME}:${VER#v}")' \
  "$repo_dir/.github/workflows/agent-k8s-image.yml"

helm lint "$chart_dir"

helm template default "$chart_dir" >"$test_dir/default.yaml"
grep -Fq 'kind: PersistentVolumeClaim' "$test_dir/default.yaml"
grep -Fq 'mountPath: /var/lib/shellfleet-agent' "$test_dir/default.yaml"
grep -Fq -- '- --pair-if-needed' "$test_dir/default.yaml"
grep -Fq 'claimName: default-shellfleet-agent-state' "$test_dir/default.yaml"
grep -Fq "image: \"ghcr.io/sppidy/shellfleet/agent-k8s:${agent_version}\"" "$test_dir/default.yaml"
grep -Fq 'helm.sh/resource-policy: keep' "$test_dir/default.yaml"
grep -Fq 'runAsUser: 10001' "$test_dir/default.yaml"

long_release=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
helm template "$long_release" "$chart_dir" >"$test_dir/long-name.yaml"
long_claim=$(sed -n 's/^ *claimName: *//p' "$test_dir/long-name.yaml" | head -n 1)
test -n "$long_claim"
test "${#long_claim}" -le 63

helm template ephemeral "$chart_dir" \
  --set persistence.enabled=false >"$test_dir/ephemeral.yaml"
if grep -Fq 'kind: PersistentVolumeClaim' "$test_dir/ephemeral.yaml"; then
  echo 'ephemeral render unexpectedly created a PVC' >&2
  exit 1
fi
grep -Fq 'emptyDir: {}' "$test_dir/ephemeral.yaml"

helm template supplied "$chart_dir" \
  --set persistence.existingClaim=operator-state \
  --set token.existingSecret=bootstrap-token \
  --set mtls.existingSecret=agent-mtls >"$test_dir/supplied.yaml"
if grep -Fq 'kind: PersistentVolumeClaim' "$test_dir/supplied.yaml"; then
  echo 'existingClaim render unexpectedly created a PVC' >&2
  exit 1
fi
grep -Fq 'claimName: operator-state' "$test_dir/supplied.yaml"
grep -Fq 'name: bootstrap-credentials' "$test_dir/supplied.yaml"
grep -Fq 'secretName: bootstrap-token' "$test_dir/supplied.yaml"
grep -Fq 'secretName: agent-mtls' "$test_dir/supplied.yaml"
grep -Fq 'defaultMode: 0440' "$test_dir/supplied.yaml"
test "$(grep -Fc 'mode: 0440' "$test_dir/supplied.yaml")" -eq 3

helm template no-retain "$chart_dir" \
  --set persistence.retain=false \
  --set persistence.storageClass=- >"$test_dir/no-retain.yaml"
grep -Fq 'storageClassName: ""' "$test_dir/no-retain.yaml"
if grep -Fq 'helm.sh/resource-policy: keep' "$test_dir/no-retain.yaml"; then
  echo 'retain=false render unexpectedly kept the PVC' >&2
  exit 1
fi

if helm template invalid "$chart_dir" --set replicaCount=2 \
  >"$test_dir/invalid.yaml" 2>"$test_dir/invalid.err"; then
  echo 'replicaCount=2 unexpectedly passed schema validation' >&2
  exit 1
fi
grep -Fq 'replicaCount' "$test_dir/invalid.err"

if helm template typo "$chart_dir" --set persistnce.enabled=false \
  >"$test_dir/typo.yaml" 2>"$test_dir/typo.err"; then
  echo 'unknown top-level value unexpectedly passed schema validation' >&2
  exit 1
fi
grep -Fq 'persistnce' "$test_dir/typo.err"

echo 'Helm render tests passed.'
