//! Shared test scaffolding for the team modules: a recording TeamBridge
//! mock and a canned TeamConfig. Test-only (cfg(test)).

use std::sync::Mutex;

use serde_json::Value;

use crate::server::TeamBridge;

use super::TeamConfig;

pub(super) struct RecordingBridge {
    pub(super) seeded: Mutex<Vec<(String, Value)>>,
    pub(super) existing: Vec<(String, Value, String)>,
}
impl TeamBridge for RecordingBridge {
    fn history(&self, _room: &str, _l: i64) -> Value { serde_json::json!({}) }
    fn roster(&self, _room: &str) -> Value { serde_json::json!({ "roster": [] }) }
    fn post(&self, _room: &str, _f: &str, _b: &str, _r: bool) -> Result<Value, String> { Ok(Value::Null) }
    fn set_agent_status(&self, _room: &str, _agent: &str, _status: &str) -> Result<(), String> { Ok(()) }
    fn employees(&self, _room: &str) -> Value { serde_json::json!({}) }
    fn seed_employee(&self, _room: &str, name: &str, spec: &Value) -> Result<(), String> {
        self.seeded.lock().unwrap().push((name.to_string(), spec.clone()));
        Ok(())
    }
    fn employee_specs(&self, _room: &str) -> Vec<(String, Value, String)> { self.existing.clone() }
    fn room_exists(&self, _room: &str) -> bool { true }
    fn start_team(&self, _workspace: &str, _template: &str) -> Value { serde_json::json!({ "started": false }) }
    fn close_team(&self, _room: &str) -> bool { false }
    fn teams(&self) -> Value { serde_json::json!({ "teams": [] }) }
    fn templates(&self) -> Value { serde_json::json!({ "templates": [] }) }
    fn save_template(&self, _name: &str, _agents: &Value) -> Result<(), String> { Ok(()) }
    fn delete_template(&self, _name: &str) -> Result<(), String> { Ok(()) }
    fn system_prompt(&self) -> String { String::new() }
    fn save_system_prompt(&self, _text: &str) -> Result<(), String> { Ok(()) }
    fn default_workspace(&self) -> String { "/tmp/ws".into() }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
        tokio::sync::broadcast::channel(1).1
    }
}

pub(super) fn cfg() -> TeamConfig {
    TeamConfig { url: "http://127.0.0.1:8787".into(), model: "claude-sonnet-4.6".into(), system_prompt: String::new(), team_rules: String::new(), team_kick: "kick".into() }
}
