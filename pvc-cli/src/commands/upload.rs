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

use crate::file_input::{normalize_input_path, read_text_file};
use crate::session::StoredSession;
use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub struct UploadCommand {
    pub path: String,
}

pub async fn run(command: UploadCommand) -> Result<()> {
    let session = StoredSession::load()?.ok_or_else(|| anyhow!("run `pvc-cli login` first"))?;
    let file_path = normalize_input_path(&command.path)?;
    let content = read_text_file(&file_path)?;
    let filename = file_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("file name is invalid UTF-8"))?;

    let (mut client, refreshed) = session.bootstrap_and_refresh("upload").await?;
    client.upload_knowledge_document(filename, &content).await?;
    refreshed.save()?;

    println!("Uploaded {filename}");
    Ok(())
}
