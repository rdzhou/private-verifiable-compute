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
use crate::session::StoredSession;
use anyhow::{Result, anyhow};
use futures::StreamExt;
use pvc_client_core::PvcClient;
use serde_json::{Value, json};
use std::io::{self, Read, Write};

#[derive(Debug, Clone)]
pub struct ChatCommand {
    pub prompt: Option<String>,
    pub prompt_stdin: bool,
    pub model: Option<String>,
    pub output: OutputMode,
}

#[derive(Debug, Clone)]
enum ChatMode {
    OneShot(String),
    Interactive,
}

pub async fn run(command: ChatCommand) -> Result<()> {
    let session = StoredSession::load()?.ok_or_else(|| anyhow!("run `pvc-cli login` first"))?;
    let (mut client, refreshed) = session.bootstrap_and_refresh("chat").await?;
    refreshed.save()?;

    match resolve_mode(&command)? {
        ChatMode::OneShot(prompt) => {
            stream_chat_turn(
                &mut client,
                &[],
                &prompt,
                command.model.as_deref(),
                command.output,
            )
            .await?;
        }
        ChatMode::Interactive => {
            run_interactive_loop(&mut client, &command).await?;
        }
    }

    Ok(())
}

fn resolve_mode(command: &ChatCommand) -> Result<ChatMode> {
    if let Some(prompt) = &command.prompt {
        if !prompt.trim().is_empty() {
            return Ok(ChatMode::OneShot(prompt.clone()));
        }
    }

    if command.prompt_stdin {
        let prompt = read_prompt_from_stdin()?;
        if prompt.is_empty() {
            return Err(anyhow!("stdin prompt was empty"));
        }
        return Ok(ChatMode::OneShot(prompt));
    }

    Ok(ChatMode::Interactive)
}

async fn run_interactive_loop(client: &mut PvcClient, command: &ChatCommand) -> Result<()> {
    command.output.print_interactive_start();

    let mut history = Vec::new();
    loop {
        let Some(prompt) = read_interactive_prompt(command.output)? else {
            break;
        };

        if is_exit_command(&prompt) {
            break;
        }
        if prompt.is_empty() {
            continue;
        }

        match stream_chat_turn(
            client,
            &history,
            &prompt,
            command.model.as_deref(),
            command.output,
        )
        .await
        {
            Ok(reply) => {
                history.push(message("user", &prompt));
                history.push(message("assistant", &reply));
            }
            Err(error) => {
                command.output.print_chat_error(&error.to_string());
            }
        }
    }

    command.output.print_interactive_exit();

    Ok(())
}

fn read_prompt_from_stdin() -> Result<String> {
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer.trim().to_string())
}

fn read_interactive_prompt(output: OutputMode) -> Result<Option<String>> {
    match output {
        OutputMode::Human => {
            print!("You> ");
            io::stdout().flush()?;
        }
        OutputMode::Json => {
            eprint!("> ");
            io::stderr().flush()?;
        }
    }

    let mut buffer = String::new();
    let read = io::stdin().read_line(&mut buffer)?;
    if read == 0 {
        return Ok(None);
    }

    Ok(Some(buffer.trim().to_string()))
}

async fn stream_chat_turn(
    client: &mut PvcClient,
    history: &[Value],
    prompt: &str,
    model: Option<&str>,
    output: OutputMode,
) -> Result<String> {
    let payload = build_payload(history, prompt, model)?;
    let mut stream = client.chat_completions(None, &payload).await?;
    let mut content = String::new();
    while let Some(item) = stream.next().await {
        let chunk = item?;
        consume_chunk(&chunk, &mut content, output)?;
    }
    output.print_chat_summary(&content);
    Ok(content)
}

fn build_payload(history: &[Value], prompt: &str, model: Option<&str>) -> Result<Vec<u8>> {
    let mut messages = Vec::with_capacity(history.len() + 1);
    messages.extend(history.iter().cloned());
    messages.push(message("user", prompt));

    let mut body = json!({
        "messages": messages,
        "stream": true,
    });
    if let Some(model) = model {
        body["model"] = Value::String(model.to_string());
    }

    Ok(serde_json::to_vec(&body)?)
}

fn message(role: &str, content: &str) -> Value {
    json!({
        "role": role,
        "content": content,
    })
}

fn is_exit_command(prompt: &str) -> bool {
    matches!(prompt.trim(), "exit" | "quit" | "/exit" | "/quit")
}

fn consume_chunk(chunk: &str, content: &mut String, output: OutputMode) -> Result<()> {
    let trimmed_chunk = chunk.trim();
    if trimmed_chunk.starts_with('{') {
        if let Ok(json) = serde_json::from_str::<Value>(trimmed_chunk) {
            handle_payload(&json, content, output)?;
        }
    }

    for line in chunk.split('\n') {
        let trimmed = line.trim();
        if !trimmed.starts_with("data: ") {
            continue;
        }
        let data = &trimmed[6..];
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        let json: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        handle_payload(&json, content, output)?;
    }
    Ok(())
}

fn handle_payload(json: &Value, content: &mut String, output: OutputMode) -> Result<()> {
    if let Some(message) = json["error"]["message"].as_str() {
        return Err(anyhow!(message.to_string()));
    }

    let delta = &json["choices"][0]["delta"];
    if let Some(text) = delta["content"].as_str() {
        content.push_str(text);
        output.print_chat_chunk(text);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consume_chunk_appends_content() {
        let mut content = String::new();
        consume_chunk(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n",
            &mut content,
            OutputMode::Json,
        )
        .unwrap();

        assert_eq!(content, "hello");
    }

    #[test]
    fn consume_chunk_ignores_reasoning_content() {
        let mut content = String::new();
        consume_chunk(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n",
            &mut content,
            OutputMode::Json,
        )
        .unwrap();

        assert_eq!(content, "");
    }

    #[test]
    fn consume_chunk_ignores_malformed_json() {
        let mut content = String::new();
        consume_chunk("data: not-json\n", &mut content, OutputMode::Json).unwrap();

        assert!(content.is_empty());
    }

    #[test]
    fn consume_chunk_returns_sse_error() {
        let mut content = String::new();
        let error = consume_chunk(
            "data: {\"error\":{\"message\":\"boom\"}}\n",
            &mut content,
            OutputMode::Json,
        )
        .unwrap_err();

        assert!(error.to_string().contains("boom"));
    }

    #[test]
    fn consume_chunk_returns_plain_json_error() {
        let mut content = String::new();
        let error = consume_chunk(
            "{\"error\":{\"message\":\"plain boom\"}}",
            &mut content,
            OutputMode::Json,
        )
        .unwrap_err();

        assert!(error.to_string().contains("plain boom"));
    }

    #[test]
    fn build_payload_includes_history_and_model() {
        let history = vec![message("assistant", "hello")];
        let payload = build_payload(&history, "hi", Some("demo-model")).unwrap();
        let json: Value = serde_json::from_slice(&payload).unwrap();

        assert_eq!(json["messages"][0]["role"], "assistant");
        assert_eq!(json["messages"][1]["content"], "hi");
        assert_eq!(json["model"], "demo-model");
    }

    #[test]
    fn exit_commands_are_detected() {
        assert!(is_exit_command("exit"));
        assert!(is_exit_command("/quit"));
        assert!(!is_exit_command("hello"));
    }

    #[test]
    fn no_prompt_defaults_to_interactive_mode() {
        let command = ChatCommand {
            prompt: None,
            prompt_stdin: false,
            model: None,
            output: OutputMode::Human,
        };

        assert!(matches!(
            resolve_mode(&command).unwrap(),
            ChatMode::Interactive
        ));
    }
}
