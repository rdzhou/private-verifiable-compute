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
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default)]
pub enum OutputMode {
    #[default]
    Human,
    Json,
}

impl OutputMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "human" => Ok(Self::Human),
            "json" => Ok(Self::Json),
            other => anyhow::bail!("unsupported output mode: {other}"),
        }
    }

    pub fn print_chat_chunk(self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        match self {
            Self::Human => print!("{chunk}"),
            Self::Json => {
                let payload = serde_json::json!({ "type": "chunk", "content": chunk });
                println!("{}", payload);
            }
        }
    }

    pub fn print_chat_summary(self, full_text: &str) {
        match self {
            Self::Human => {
                if !full_text.ends_with('\n') {
                    println!();
                }
            }
            Self::Json => {
                let payload = serde_json::json!({ "type": "complete", "content": full_text });
                println!("{}", payload);
            }
        }
    }

    pub fn print_interactive_start(self) {
        match self {
            Self::Human => println!("Interactive chat started. Type `exit` or `quit` to leave."),
            Self::Json => {
                let payload = serde_json::json!({ "type": "interactive_start" });
                println!("{}", payload);
            }
        }
    }

    pub fn print_interactive_exit(self) {
        match self {
            Self::Human => println!("Leaving interactive chat."),
            Self::Json => {
                let payload = serde_json::json!({ "type": "interactive_exit" });
                println!("{}", payload);
            }
        }
    }

    pub fn print_chat_error(self, message: &str) {
        match self {
            Self::Human => eprintln!("Error: {message}"),
            Self::Json => {
                let payload = serde_json::json!({ "type": "error", "message": message });
                println!("{}", payload);
            }
        }
    }

    pub fn print_session(self, session: &Value) {
        match self {
            Self::Human | Self::Json => {
                println!("{}", serde_json::to_string_pretty(session).unwrap())
            }
        }
    }
}
