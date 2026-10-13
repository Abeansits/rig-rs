//! Local spec checks. These never open the daemon socket.
//!
//! `spec_validate_agent` checks that the YAML is a mapping with a non-empty
//! `name`. It is not a full `AgentSpec` schema.

use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use crate::args::YamlArgs;
use crate::result::map_body;

pub(crate) fn spec_validate_rig(args: YamlArgs) -> CallToolResult {
    let yaml = args.yaml;
    let value = match crewmux_spec::parse(&yaml) {
        Ok(_) => json!({"valid": true, "errors": []}),
        Err(err) => json!({"valid": false, "errors": [err.to_string()]}),
    };
    map_body(200, &value.to_string(), |_| false)
}

pub(crate) fn spec_validate_agent(args: YamlArgs) -> CallToolResult {
    let yaml = args.yaml;
    let value = agent_value(&yaml);
    map_body(200, &value.to_string(), |_| false)
}

fn agent_value(yaml: &str) -> Value {
    match crewmux_spec::yaml::parse_yaml::<Value>(yaml) {
        Ok(Value::Object(map)) => {
            let name = map.get("name").and_then(Value::as_str).unwrap_or("");
            if name.is_empty() {
                json!({"valid": false, "errors": ["name is required"]})
            } else {
                json!({"valid": true, "errors": []})
            }
        }
        Ok(_) => json!({"valid": false, "errors": ["agent spec must be a mapping"]}),
        Err(err) => json!({"valid": false, "errors": [err.to_string()]}),
    }
}
