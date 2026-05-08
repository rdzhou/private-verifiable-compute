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

use crate::config::{CliProfile, ProfileOverrides};
use crate::session::{StoredAuth, StoredSession, now_unix_seconds};
use anyhow::{Context, Result, anyhow};
use std::io::{self, Read};

#[derive(Debug, Clone)]
pub struct LoginCommand {
    pub overrides: ProfileOverrides,
    pub token: Option<String>,
    pub token_env_var: Option<String>,
    pub token_stdin: bool,
    pub interactive: bool,
}

pub async fn run(command: LoginCommand) -> Result<()> {
    let base_profile = CliProfile::load()?.unwrap_or_default();
    let profile = base_profile.merged_with(&command.overrides);
    let auth = resolve_auth(&command)?;

    profile.save()?;
    let (_, refreshed) = StoredSession {
        profile,
        auth,
        session_id: None,
        last_bootstrap_unix_seconds: now_unix_seconds(),
        last_command: String::from("login"),
    }
    .bootstrap_and_refresh("login")
    .await
    .context("failed to bootstrap PVC session")?;
    refreshed.save()?;

    if let Some(session_id) = refreshed.session_id.as_deref() {
        println!("Login succeeded. Session ID: {session_id}");
    } else {
        println!("Login succeeded.");
    }
    Ok(())
}

fn resolve_auth(command: &LoginCommand) -> Result<StoredAuth> {
    if command.interactive {
        return Err(anyhow!(
            "interactive login is not supported yet; use --token, --token-env-var, or --token-stdin"
        ));
    }

    if let Some(token) = &command.token {
        return Ok(StoredAuth::Token {
            value: token.clone(),
        });
    }

    if let Some(env_var) = &command.token_env_var {
        let value = std::env::var(env_var)
            .with_context(|| format!("environment variable {env_var} is not set"))?;
        return Ok(StoredAuth::Token { value });
    }

    if command.token_stdin {
        let value = read_token_from_stdin()?;
        return Ok(StoredAuth::Token { value });
    }

    Err(anyhow!(
        "provide --token, --token-env-var, --token-stdin, or --interactive"
    ))
}

fn read_token_from_stdin() -> Result<String> {
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;
    let token = buffer.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("stdin token input is empty"));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_command() -> LoginCommand {
        LoginCommand {
            overrides: ProfileOverrides::default(),
            token: None,
            token_env_var: None,
            token_stdin: false,
            interactive: false,
        }
    }

    #[test]
    fn resolve_auth_rejects_interactive() {
        let mut command = base_command();
        command.interactive = true;

        let error = resolve_auth(&command).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("interactive login is not supported yet")
        );
    }

    #[test]
    fn resolve_auth_prefers_explicit_token() {
        let mut command = base_command();
        command.token = Some("token-value".to_string());

        let auth = resolve_auth(&command).unwrap();
        match auth {
            StoredAuth::Token { value } => assert_eq!(value, "token-value"),
            StoredAuth::InteractivePending => panic!("expected token auth"),
        }
    }
}
