// Copyright 2025 TikTok Inc. and/or its affiliates
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};

pub fn read_text_file(path: &Path) -> Result<String> {
    if !path.exists() {
        return Err(anyhow!("file does not exist: {}", path.display()));
    }
    if !path.is_file() {
        return Err(anyhow!("path is not a file: {}", path.display()));
    }
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

pub fn normalize_input_path(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty() {
        return Err(anyhow!("file path is empty"));
    }
    Ok(path)
}
