//! Cloud LLM engine (Google Gemini & OpenAI-compatible) and Hybrid fallback engine.

use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::chat::{ChatHistory, Role};
use crate::engine::LlmEngine;
use crate::grammar::ToolSpec;

#[derive(Debug, Clone)]
pub struct CloudConfig {
    pub provider: String, // "gemini" or "openai"
    pub model: String,
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            provider: "gemini".to_string(),
            model: "gemini-2.0-flash".to_string(),
            api_key: None,
            endpoint: None,
        }
    }
}

pub struct CloudEngine {
    config: CloudConfig,
}

impl CloudEngine {
    pub fn new(config: CloudConfig) -> Self {
        Self { config }
    }

    fn resolve_api_key(&self) -> Option<String> {
        self.config
            .api_key
            .clone()
            .or_else(|| match self.config.provider.as_str() {
                "openai" => std::env::var("OPENAI_API_KEY").ok(),
                _ => std::env::var("GEMINI_API_KEY").ok(),
            })
    }

    async fn call_gemini(
        &self,
        api_key: &str,
        history: &ChatHistory,
        tools: &[ToolSpec],
    ) -> anyhow::Result<String> {
        let endpoint = self.config.endpoint.clone().unwrap_or_else(|| {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                self.config.model, api_key
            )
        });

        // Convert ChatHistory to Gemini contents format
        let mut contents = Vec::new();
        let mut system_text = String::new();

        for msg in history.messages() {
            match msg.role {
                Role::System => {
                    if !system_text.is_empty() {
                        system_text.push('\n');
                    }
                    system_text.push_str(&msg.content);
                }
                Role::User => {
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{ "text": msg.content }]
                    }));
                }
                Role::Assistant => {
                    contents.push(serde_json::json!({
                        "role": "model",
                        "parts": [{ "text": msg.content }]
                    }));
                }
                Role::Tool => {
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{ "text": format!("Tool response: {}", msg.content) }]
                    }));
                }
            }
        }

        // Add tools description to system prompt if any
        if !tools.is_empty() {
            system_text = crate::chat::format_system_with_tools(&system_text, tools);
        }

        let mut payload = serde_json::json!({
            "contents": contents,
        });

        if !system_text.is_empty() {
            payload["systemInstruction"] = serde_json::json!({
                "parts": [{ "text": system_text }]
            });
        }

        let payload_str = serde_json::to_string(&payload)?;

        let mut child = Command::new("curl")
            .args([
                "-s",
                "-S",
                "--max-time",
                "30",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                &endpoint,
                "-d",
                "@-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(payload_str.as_bytes()).await?;
        }

        let output = child.wait_with_output().await?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Gemini API curl failed: {err}");
        }

        let resp_val: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        if let Some(candidates) = resp_val.get("candidates").and_then(|c| c.as_array()) {
            if let Some(first) = candidates.first() {
                if let Some(parts) = first
                    .get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.as_array())
                {
                    let mut full_text = String::new();
                    for part in parts {
                        if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                            full_text.push_str(t);
                        }
                    }
                    if !full_text.is_empty() {
                        return Ok(full_text);
                    }
                }
            }
        }

        if let Some(err) = resp_val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            anyhow::bail!("Gemini API error: {err}");
        }

        anyhow::bail!(
            "Unexpected Gemini response: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    }

    async fn call_openai(
        &self,
        api_key: &str,
        history: &ChatHistory,
        tools: &[ToolSpec],
    ) -> anyhow::Result<String> {
        let endpoint = self.config.endpoint.clone().unwrap_or_else(|| {
            "https://api.openai.com/v1/chat/completions".to_string()
        });

        let mut messages = Vec::new();
        for msg in history.messages() {
            let role_str = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            let content = if msg.role == Role::System && !tools.is_empty() {
                crate::chat::format_system_with_tools(&msg.content, tools)
            } else {
                msg.content.clone()
            };
            messages.push(serde_json::json!({
                "role": role_str,
                "content": content
            }));
        }

        let payload = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "temperature": 0.3,
        });

        let payload_str = serde_json::to_string(&payload)?;

        let mut child = Command::new("curl")
            .args([
                "-s",
                "-S",
                "--max-time",
                "30",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-H",
                &format!("Authorization: Bearer {}", api_key),
                &endpoint,
                "-d",
                "@-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(payload_str.as_bytes()).await?;
        }

        let output = child.wait_with_output().await?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("OpenAI API curl failed: {err}");
        }

        let resp_val: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        if let Some(choices) = resp_val.get("choices").and_then(|c| c.as_array()) {
            if let Some(first) = choices.first() {
                if let Some(content) = first
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                {
                    return Ok(content.to_string());
                }
            }
        }

        if let Some(err) = resp_val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            anyhow::bail!("OpenAI API error: {err}");
        }

        anyhow::bail!(
            "Unexpected OpenAI response: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    }
}

#[async_trait]
impl LlmEngine for CloudEngine {
    async fn generate(&self, history: &ChatHistory, tools: &[ToolSpec]) -> anyhow::Result<String> {
        let Some(api_key) = self.resolve_api_key() else {
            anyhow::bail!(
                "No API key configured for cloud provider '{}'",
                self.config.provider
            );
        };

        if self.config.provider == "openai" {
            self.call_openai(&api_key, history, tools).await
        } else {
            self.call_gemini(&api_key, history, tools).await
        }
    }

    async fn generate_stream<'a>(
        &'a self,
        history: &'a ChatHistory,
        tools: &'a [ToolSpec],
    ) -> anyhow::Result<BoxStream<'a, anyhow::Result<String>>> {
        let full = self.generate(history, tools).await?;
        Ok(stream::iter(std::iter::once(Ok(full))).boxed())
    }
}

/// Hybrid engine: tries local first or routes to cloud, with seamless automatic fallback.
pub struct HybridEngine {
    local: Arc<dyn LlmEngine>,
    cloud: Option<Arc<dyn LlmEngine>>,
    prefer_cloud: bool,
}

impl HybridEngine {
    pub fn new(
        local: Arc<dyn LlmEngine>,
        cloud: Option<Arc<dyn LlmEngine>>,
        prefer_cloud: bool,
    ) -> Self {
        Self {
            local,
            cloud,
            prefer_cloud,
        }
    }
}

#[async_trait]
impl LlmEngine for HybridEngine {
    async fn generate(&self, history: &ChatHistory, tools: &[ToolSpec]) -> anyhow::Result<String> {
        if self.prefer_cloud {
            if let Some(ref cloud) = self.cloud {
                match cloud.generate(history, tools).await {
                    Ok(r) => return Ok(r),
                    Err(e) => {
                        tracing::warn!("Cloud LLM error: {e:#} — falling back to local model");
                    }
                }
            }
        }

        // Try local model
        match self.local.generate(history, tools).await {
            Ok(r) => Ok(r),
            Err(e) => {
                if !self.prefer_cloud {
                    if let Some(ref cloud) = self.cloud {
                        tracing::info!("Local LLM error: {e:#} — attempting cloud fallback");
                        if let Ok(r) = cloud.generate(history, tools).await {
                            return Ok(r);
                        }
                    }
                }
                Err(e)
            }
        }
    }

    async fn generate_stream<'a>(
        &'a self,
        history: &'a ChatHistory,
        tools: &'a [ToolSpec],
    ) -> anyhow::Result<BoxStream<'a, anyhow::Result<String>>> {
        if self.prefer_cloud {
            if let Some(ref cloud) = self.cloud {
                if let Ok(st) = cloud.generate_stream(history, tools).await {
                    return Ok(st);
                }
            }
        }

        match self.local.generate_stream(history, tools).await {
            Ok(st) => Ok(st),
            Err(e) => {
                if let Some(ref cloud) = self.cloud {
                    tracing::info!("Local stream failed ({e:#}) — attempting cloud stream");
                    if let Ok(st) = cloud.generate_stream(history, tools).await {
                        return Ok(st);
                    }
                }
                Err(e)
            }
        }
    }
}
