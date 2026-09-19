// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use serde::{Deserialize, Serialize};

/// Represents a GCE custom image object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_disk: Option<RawDisk>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guest_os_features: Vec<GuestOsFeature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RawDisk {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GuestOsFeature {
    #[serde(rename = "type")]
    pub feature_type: String,
}

/// Represents an operation returned by asynchronous GCE API calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OperationError {
    #[serde(default)]
    pub errors: Vec<OperationErrorItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OperationErrorItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Represents a GCE virtual machine instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_interfaces: Vec<NetworkInterface>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_link: Option<String>,
}

impl Instance {
    /// Extracts the internal primary IPv4 address, if assigned.
    pub fn internal_ip(&self) -> Option<&str> {
        self.network_interfaces.first().and_then(|nic| nic.network_ip.as_deref())
    }

    /// Extracts the external public IPv4 address, if assigned.
    pub fn external_ip(&self) -> Option<&str> {
        self.network_interfaces
            .first()
            .and_then(|nic| nic.access_configs.first())
            .and_then(|cfg| cfg.nat_ip.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    #[serde(rename = "networkIP", alias = "networkIp", skip_serializing_if = "Option::is_none")]
    pub network_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access_configs: Vec<AccessConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccessConfig {
    #[serde(rename = "natIP", alias = "natIp", skip_serializing_if = "Option::is_none")]
    pub nat_ip: Option<String>,
}

/// Represents the list response from GET /compute/v1/projects/{project}/zones/{zone}/instances.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstanceList {
    #[serde(default)]
    pub items: Vec<Instance>,
}

/// Represents serial port output returned by GCE REST API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortOutput {
    #[serde(default)]
    pub contents: String,
    #[serde(default, deserialize_with = "deserialize_string_as_i64")]
    pub start: i64,
    #[serde(default, deserialize_with = "deserialize_string_as_i64")]
    pub next: i64,
}

/// Deserializes an int64 field encoded as a decimal JSON string.
///
/// In the Google Compute Engine REST API, 64-bit integers (`start` and `next`)
/// are returned as JSON decimal strings (e.g. `"1024"`) per protobuf-to-JSON mapping
/// rules to prevent precision loss in JavaScript clients.
fn deserialize_string_as_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<i64>().map_err(serde::de::Error::custom)
}

/// Result returned by `ffx gce stop` in machine-readable output format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StopResult {
    pub name: String,
    pub project: String,
    pub zone: String,
    pub action: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[fuchsia::test]
    fn test_deserialize_instance_list() {
        let json_str = r#"{
            "items": [
                {
                    "name": "fuchsia-vm-1",
                    "machineType": "zones/us-central1-a/machineTypes/n2-standard-4",
                    "status": "RUNNING",
                    "networkInterfaces": [
                        {
                            "networkIP": "10.128.0.2",
                            "accessConfigs": [
                                {
                                    "natIP": "35.200.100.50"
                                }
                            ]
                        }
                    ]
                }
            ]
        }"#;

        let list: InstanceList = serde_json::from_str(json_str).expect("parsed instance list");
        assert_eq!(list.items.len(), 1);
        let inst = &list.items[0];
        assert_eq!(inst.name.as_deref(), Some("fuchsia-vm-1"));
        assert_eq!(inst.status.as_deref(), Some("RUNNING"));
        assert_eq!(inst.internal_ip(), Some("10.128.0.2"));
        assert_eq!(inst.external_ip(), Some("35.200.100.50"));
    }

    #[fuchsia::test]
    fn test_empty_instance_list() {
        let json_str = r#"{}"#;
        let list: InstanceList = serde_json::from_str(json_str).expect("parsed empty list");
        assert!(list.items.is_empty());
    }

    #[fuchsia::test]
    fn test_serial_port_output_deserialization() {
        let json_str = r#"{"contents": "world", "start": "10", "next": "20"}"#;
        let spo: SerialPortOutput = serde_json::from_str(json_str).unwrap();
        assert_eq!(spo.contents, "world");
        assert_eq!(spo.start, 10);
        assert_eq!(spo.next, 20);
    }

    #[fuchsia::test]
    fn test_serial_port_output_rejects_numeric_literal() {
        let json_int = r#"{"contents": "hello", "start": 0, "next": 10}"#;
        assert!(serde_json::from_str::<SerialPortOutput>(json_int).is_err());
    }

    #[fuchsia::test]
    fn test_operation_deserialization() {
        let json = r#"{
            "id": "123456789",
            "name": "operation-123",
            "status": "DONE",
            "targetLink": "https://www.googleapis.com/compute/v1/projects/my-proj/zones/us-central1-a/instances/test-vm"
        }"#;

        let op: Operation = serde_json::from_str(json).expect("deserialize operation");
        assert_eq!(op.name.as_deref(), Some("operation-123"));
        assert_eq!(op.status.as_deref(), Some("DONE"));
    }

    #[fuchsia::test]
    fn test_stop_result_serialization() {
        let result = StopResult {
            name: "test-vm".to_string(),
            project: "test-proj".to_string(),
            zone: "us-central1-a".to_string(),
            action: "stopped".to_string(),
        };

        let json = serde_json::to_string(&result).expect("serialize stop result");
        assert!(json.contains("\"action\":\"stopped\""));
        let parsed: StopResult = serde_json::from_str(&json).expect("deserialize stop result");
        assert_eq!(parsed, result);
    }

    #[fuchsia::test]
    fn test_image_serialization_roundtrip() {
        let image = Image {
            name: Some("fuchsia-test-img".to_string()),
            raw_disk: Some(RawDisk {
                source: "https://storage.googleapis.com/b/disk.tar.gz".to_string(),
            }),
            guest_os_features: vec![GuestOsFeature {
                feature_type: "VIRTIO_SCSI_MULTIQUEUE".to_string(),
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&image).expect("serialize image");
        assert!(json.contains("\"rawDisk\":{\"source\":"));
        assert!(json.contains("\"guestOsFeatures\":[{\"type\":\"VIRTIO_SCSI_MULTIQUEUE\"}]"));
        let parsed: Image = serde_json::from_str(&json).expect("deserialize image");
        assert_eq!(parsed, image);
    }

    #[fuchsia::test]
    fn test_operation_deserialization_with_error() {
        let json = r#"{
            "id": "123",
            "status": "DONE",
            "error": {
                "errors": [
                    {"code": "RESOURCE_ALREADY_EXISTS", "message": "Image already exists"}
                ]
            }
        }"#;
        let op: Operation = serde_json::from_str(json).expect("deserialize operation with error");
        assert!(op.error.is_some());
        assert_eq!(op.error.unwrap().errors[0].code.as_deref(), Some("RESOURCE_ALREADY_EXISTS"));
    }
}
