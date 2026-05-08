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

use anyhow::Result;
use pvc_client_core::{PvcClientConfig, pvc_home_dir, read_json_file, write_private_json_file};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliProfile {
    pub identity_server_url: String,
    pub gateway_url: String,
    pub relay_url: String,
    pub target_url: String,
}

impl Default for CliProfile {
    fn default() -> Self {
        Self {
            identity_server_url: String::from("http://localhost:8000"),
            gateway_url: String::from("http://localhost:8082"),
            relay_url: String::from("http://localhost:8787"),
            target_url: String::from("localhost:9000"),
        }
    }
}

impl CliProfile {
    pub fn load() -> Result<Option<Self>> {
        read_json_file(&profile_path()?)
    }

    pub fn save(&self) -> Result<()> {
        write_private_json_file(&profile_path()?, self)
    }

    pub fn merged_with(&self, other: &ProfileOverrides) -> Self {
        Self {
            identity_server_url: other
                .identity_server_url
                .clone()
                .unwrap_or_else(|| self.identity_server_url.clone()),
            gateway_url: other
                .gateway_url
                .clone()
                .unwrap_or_else(|| self.gateway_url.clone()),
            relay_url: other
                .relay_url
                .clone()
                .unwrap_or_else(|| self.relay_url.clone()),
            target_url: other
                .target_url
                .clone()
                .unwrap_or_else(|| self.target_url.clone()),
        }
    }

    pub fn to_client_config(&self) -> PvcClientConfig {
        PvcClientConfig {
            identity_server_url: self.identity_server_url.clone(),
            ohttp_gateway_url: self.gateway_url.clone(),
            relay_url: self.relay_url.clone(),
            target_url: self.target_url.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProfileOverrides {
    pub identity_server_url: Option<String>,
    pub gateway_url: Option<String>,
    pub relay_url: Option<String>,
    pub target_url: Option<String>,
}

pub fn profile_path() -> Result<PathBuf> {
    Ok(pvc_home_dir()?.join("cli").join("profile.json"))
}
