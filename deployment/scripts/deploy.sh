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
Usage: ./deploy.sh [--platform=minikube|gke|kata] [--dry-run] [--verbose] [--help]
EOF
}

script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
platform="minikube"
args=()

for arg in "$@"
do
    case $arg in
        --platform=*)
        platform="${arg#--platform=}"
        ;;
        --help|-h)
        usage
        exit 0
        ;;
        *)
        args+=("$arg")
        ;;
    esac
done

case "$platform" in
    minikube)
    target_script="$script_dir/deploy-minikube.sh"
    ;;
    gke)
    target_script="$script_dir/deploy-gke.sh"
    ;;
    kata)
    target_script="$script_dir/deploy-kata.sh"
    ;;
    *)
    echo "Error: platform must be 'minikube', 'gke', or 'kata', got '$platform'" >&2
    exit 1
    ;;
esac

if [ ! -x "$target_script" ]; then
    echo "Error: deploy script is missing or not executable: $target_script" >&2
    exit 1
fi

echo "Selected deployment target: $platform ($target_script)"
exec "$target_script" "${args[@]}"
