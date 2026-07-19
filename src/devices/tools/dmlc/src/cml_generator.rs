// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::parser::DmlProgram;
use anyhow::Context;
use serde_json::Value;

pub fn de_duplicate_use_entries(use_entries: &[Value]) -> Vec<Value> {
    let mut grouped: Vec<(serde_json::Map<String, Value>, Vec<&serde_json::Map<String, Value>>)> =
        Vec::new();
    let mut result = Vec::new();

    for entry in use_entries {
        if let Some(obj) = entry.as_object() {
            let mut base = obj.clone();
            base.remove("availability");

            if let Some(pos) = grouped.iter().position(|(k, _)| k == &base) {
                grouped[pos].1.push(obj);
            } else {
                grouped.push((base, vec![obj]));
            }
        } else {
            if !result.contains(entry) {
                result.push(entry.clone());
            }
        }
    }

    for (mut base, objs) in grouped {
        let first_availability = objs[0].get("availability").and_then(|v| v.as_str());
        let all_same = objs
            .iter()
            .all(|obj| obj.get("availability").and_then(|v| v.as_str()) == first_availability);

        if all_same {
            if let Some(av) = first_availability {
                base.insert("availability".to_string(), Value::String(av.to_string()));
            } else {
                base.remove("availability");
            }
        } else {
            base.remove("availability");
        }
        result.push(Value::Object(base));
    }

    result.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
    result
}

pub fn json_to_json5(json_str: &str) -> String {
    let mut result = String::new();
    for line in json_str.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('"') {
            if let Some(quote_end) = trimmed[1..].find('"') {
                let key = &trimmed[1..quote_end + 1];
                let rest = &trimmed[quote_end + 2..];
                let rest_trimmed = rest.trim_start();
                if rest_trimmed.starts_with(':') {
                    let is_valid_id = !key.is_empty()
                        && key.chars().next().unwrap().is_alphabetic()
                        && key.chars().all(|c| c.is_alphanumeric() || c == '_');
                    if is_valid_id {
                        let indent = line.len() - trimmed.len();
                        result.push_str(&line[..indent]);
                        result.push_str(key);
                        result.push_str(rest);
                        result.push('\n');
                        continue;
                    }
                }
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

pub fn generate_board_cml_file(name: &str, program: &DmlProgram) -> Result<String, anyhow::Error> {
    let driver_name = program.driver_name.as_deref().unwrap_or(name);
    let binary = format!("driver/{}.so", driver_name);
    let bind = format!("meta/bind/{}.bindbc", driver_name);

    let cml = serde_json::json!({
        "include": [
            "inspect/client.shard.cml",
            "syslog/client.shard.cml"
        ],
        "program": {
            "runner": "driver",
            "binary": binary,
            "bind": bind,
            "default_dispatcher_opts": [ "allow_sync_calls" ],
            "colocate": "true"
        },
        "use": [
            { "service": "fuchsia.hardware.platform.bus.Service" },
            { "protocol": "fuchsia.driver.framework.CompositeNodeManager" }
        ]
    });

    let cml_code = serde_json::to_string_pretty(&cml).context("Failed to serialize CML")?;
    Ok(json_to_json5(&cml_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_de_duplicate_use_entries() {
        let entries = vec![
            serde_json::json!({ "service": "fuchsia.hardware.gpio.Service" }),
            serde_json::json!({ "protocol": "fuchsia.hardware.i2c.Device" }),
            serde_json::json!({ "service": "fuchsia.hardware.gpio.Service", "availability": "optional" }),
            serde_json::json!({ "service": "fuchsia.hardware.power.Service", "availability": "optional" }),
            serde_json::json!({ "service": "fuchsia.hardware.power.Service", "availability": "optional" }),
            serde_json::json!({ "config": "fuchsia.power.SuspendEnabled", "key": "suspend_enabled", "type": "bool" }),
        ];
        let cleaned = de_duplicate_use_entries(&entries);
        assert_eq!(cleaned.len(), 4);
        assert_eq!(
            cleaned[0],
            serde_json::json!({ "config": "fuchsia.power.SuspendEnabled", "key": "suspend_enabled", "type": "bool" })
        );
        assert_eq!(cleaned[1], serde_json::json!({ "protocol": "fuchsia.hardware.i2c.Device" }));
        assert_eq!(cleaned[2], serde_json::json!({ "service": "fuchsia.hardware.gpio.Service" }));
        assert_eq!(
            cleaned[3],
            serde_json::json!({ "service": "fuchsia.hardware.power.Service", "availability": "optional" })
        );
    }

    #[test]
    fn test_de_duplicate_use_entries_directory() {
        let entries = vec![
            serde_json::json!({
                "directory": "dev-class",
                "rights": ["r*"],
                "path": "/dev/class/gpio"
            }),
            serde_json::json!({
                "directory": "dev-class",
                "rights": ["r*"],
                "path": "/dev/class/gpio",
                "availability": "optional"
            }),
            serde_json::json!({
                "directory": "dev-class",
                "rights": ["rw*"],
                "path": "/dev/class/i2c"
            }),
        ];
        let cleaned = de_duplicate_use_entries(&entries);
        assert_eq!(cleaned.len(), 2);
        assert_eq!(
            cleaned[0],
            serde_json::json!({
                "directory": "dev-class",
                "rights": ["r*"],
                "path": "/dev/class/gpio"
            })
        );
        assert_eq!(
            cleaned[1],
            serde_json::json!({
                "directory": "dev-class",
                "rights": ["rw*"],
                "path": "/dev/class/i2c"
            })
        );
    }

    #[test]
    fn test_de_duplicate_use_entries_transitional_config() {
        let entries = vec![serde_json::json!({
            "config": "fuchsia.power.SuspendEnabled",
            "key": "suspend_enabled",
            "type": "bool",
            "availability": "transitional",
            "default": false
        })];
        let cleaned = de_duplicate_use_entries(&entries);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(
            cleaned[0],
            serde_json::json!({
                "config": "fuchsia.power.SuspendEnabled",
                "key": "suspend_enabled",
                "type": "bool",
                "availability": "transitional",
                "default": false
            })
        );
    }
}
