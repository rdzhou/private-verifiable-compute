#!/bin/bash

# Copyright 2025 TikTok Inc. and/or its affiliates
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

set -euo pipefail

usage() {
    cat <<'EOF'
Usage: ./deploy-kata.sh --namespace=<namespace> [--dry-run] [--verbose] [--help]
EOF
}

require_command() {
    local cmd="$1"
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "Error: $cmd is not installed." >&2
        exit 1
    fi
}

require_file() {
    local path="$1"
    if [ ! -f "$path" ]; then
        echo "Error: required file not found: $path" >&2
        exit 1
    fi
}

require_dir() {
    local path="$1"
    if [ ! -d "$path" ]; then
        echo "Error: required directory not found: $path" >&2
        exit 1
    fi
}

script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(cd -- "$script_dir/../.." && pwd)
var_file="$repo_root/.env"
chart_dir="$script_dir/../chart"
env_values_file="$script_dir/../envs/kata.yaml"
platform="kata"
namespace=""
tag="latest"
dry_run=false
verbose=false
helm_name="private-verifiable-compute"

for arg in "$@"
do
    case $arg in
        --namespace=*)
        namespace="${arg#--namespace=}"
        ;;
        --dry-run)
        dry_run=true
        ;;
        --verbose)
        verbose=true
        ;;
        --help|-h)
        usage
        exit 0
        ;;
        --*)
        echo "Error: unknown option '$arg'" >&2
        usage >&2
        exit 1
        ;;
        *)
        echo "Error: unexpected argument '$arg'" >&2
        usage >&2
        exit 1
        ;;
    esac
done

if [ "$verbose" = true ]; then
    set -x
fi

if [ -z "$namespace" ]; then
    echo "Error: the namespace parameter is required, run the script again like ./deploy-kata.sh --namespace=<namespace>" >&2
    exit 1
fi

require_command helm
require_dir "$chart_dir"
require_file "$env_values_file"
require_file "$var_file"

source "$var_file"

if [ -z "${project_id:-}" ]; then
    echo "Error: project_id is not set in $var_file" >&2
    exit 1
fi

docker_repo="pvc-${namespace}-images"
privacy_gateway_reference="us-docker.pkg.dev/${project_id}/${docker_repo}/pvc-ohttp-gateway"
identity_server_reference="us-docker.pkg.dev/${project_id}/${docker_repo}/pvc-identity-server"
relay_reference="us-docker.pkg.dev/${project_id}/${docker_repo}/pvc-ohttp-relay"
tee_llm_reference="us-docker.pkg.dev/${project_id}/${docker_repo}/pvc-tee-llm"
pvc_client="us-docker.pkg.dev/${project_id}/${docker_repo}/pvc-client"

helm_args=(
    upgrade
    --cleanup-on-fail
    -f "$env_values_file"
    --set "platform=${platform}"
    --set "privacyGateway.image.repository=${privacy_gateway_reference}"
    --set "privacyGateway.image.tag=${tag}"
    --set "global.namespace=${namespace}"
    --set "relay.image.repository=${relay_reference}"
    --set "relay.image.tag=${tag}"
    --set "identityServer.image.repository=${identity_server_reference}"
    --set "identityServer.image.tag=${tag}"
    --set "teeLlm.image.repository=${tee_llm_reference}"
    --set "teeLlm.image.tag=${tag}"
    --set "client.image.repository=${pvc_client}"
    --set "client.image.tag=${tag}"
    --namespace "$namespace"
    --install "$helm_name" "$chart_dir"
)

if [ "$dry_run" = true ]; then
    helm_args+=(--dry-run=client --debug)
fi

echo "Deploying to platform: $platform"
helm "${helm_args[@]}"
