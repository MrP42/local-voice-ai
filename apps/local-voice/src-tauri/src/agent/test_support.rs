//! Gemeinsame Test-Bausteine des Agent-Moduls: eine llama-server-Attrappe (HTTP,
//! `/v1/chat/completions`), die jede Anfrage aufzeichnet (nur `cfg(test)`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::runtime::{AgentRuntime, Target};
use crate::managers::meetings::llm_call::test_support::{spawn_llm_mock_with, MockReply};

/// Was die Attrappe antwortet.
#[derive(Clone)]
pub enum R {
    Ok(String),
    Status(u16),
    StatusBody(u16, String),
    Hang,
}

/// Antwortkoerper wie der llama-server: Inhalt, `finish_reason`, Token.
pub fn body(content: &str, finish: &str, prompt: u64, completion: u64) -> String {
    json!({
        "choices": [{
            "message": { "role": "assistant", "content": content },
            "finish_reason": finish
        }],
        "usage": { "prompt_tokens": prompt, "completion_tokens": completion }
    })
    .to_string()
}

/// Eine fertige Antwort (`stop`, 100 Token ein, 20 aus).
pub fn ok(content: &str) -> R {
    R::Ok(body(content, "stop", 100, 20))
}

/// Eine abgeschnittene Antwort (`length`).
pub fn cut(content: &str) -> R {
    R::Ok(body(content, "length", 100, 2048))
}

pub struct Mock {
    pub base_url: String,
    pub seen: Arc<Mutex<Vec<Value>>>,
}

impl Mock {
    pub fn requests(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    pub fn runtime(&self) -> AgentRuntime {
        self.runtime_with_context(8192)
    }

    pub fn runtime_with_context(&self, context_tokens: u32) -> AgentRuntime {
        AgentRuntime::new(Target::Endpoint {
            base_url: self.base_url.clone(),
            model: "llm-test".into(),
            context_tokens,
        })
    }
}

/// Antwortet der Reihe nach (die letzte wiederholt sich).
pub async fn mock(replies: Vec<R>) -> Mock {
    let counter = Arc::new(AtomicUsize::new(0));
    mock_with(move |_| {
        let i = counter.fetch_add(1, Ordering::SeqCst);
        replies[i.min(replies.len() - 1)].clone()
    })
    .await
}

/// Antwortet je nach Anfrage (der JSON-Koerper der Anfrage).
pub async fn mock_with(handler: impl Fn(&Value) -> R + Send + Sync + 'static) -> Mock {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let port = spawn_llm_mock_with(move |request| {
        let value: Value = serde_json::from_str(request).unwrap_or(Value::Null);
        sink.lock().unwrap().push(value.clone());
        match handler(&value) {
            R::Ok(b) => MockReply::Body(b),
            R::Status(c) => MockReply::Status(c),
            R::StatusBody(c, b) => MockReply::StatusBody(c, b),
            R::Hang => MockReply::Hang,
        }
    })
    .await;
    Mock {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        seen,
    }
}

/// Der Nutzertext einer aufgezeichneten Anfrage.
pub fn user_of(request: &Value) -> String {
    request["messages"][1]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Der Systemprompt einer aufgezeichneten Anfrage.
pub fn system_of(request: &Value) -> String {
    request["messages"][0]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Ein Port, an dem niemand lauscht.
pub async fn closed_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

pub fn endpoint(port: u16) -> AgentRuntime {
    AgentRuntime::new(Target::Endpoint {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        model: "llm-test".into(),
        context_tokens: 8192,
    })
}
