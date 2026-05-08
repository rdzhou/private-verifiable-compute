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

use crate::output::OutputMode;
use crate::session::{StoredSession, session_path};
use anyhow::Result;

pub async fn show(output: OutputMode) -> Result<()> {
    let path = session_path()?;
    let payload = match StoredSession::load()? {
        Some(session) => serde_json::json!({
            "session_path": path,
            "session": session.redacted_json(),
        }),
        None => serde_json::json!({
            "session_path": path,
            "session": serde_json::Value::Null,
        }),
    };
    output.print_session(&payload);
    Ok(())
}

pub async fn clear() -> Result<()> {
    StoredSession::clear()?;
    println!("Cleared saved CLI session state");
    Ok(())
}
