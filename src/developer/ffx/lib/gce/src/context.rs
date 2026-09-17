// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::client::GceClient;
use crate::{GceInstanceData, GceTunnel};
use anyhow::{Result, bail};
use credentials::Credentials;
use discovery::gce_watcher::Instance;
use ffx_config::EnvironmentContext;
use gcs::auth::new_access_token;

/// Execution context for GCE commands, encapsulating project, zone, and an authenticated GCE API client.
#[derive(Debug)]
pub struct GceContext {
    pub env_context: EnvironmentContext,
    pub project: String,
    pub zone: String,
    pub client: GceClient,
}

impl GceContext {
    /// Initializes a `GceContext` by resolving project and zone from CLI flags or ffx config,
    /// and obtaining an OAuth2 access token.
    pub async fn new(
        env_context: EnvironmentContext,
        project_flag: Option<String>,
        zone_flag: Option<String>,
    ) -> Result<Self> {
        let project = resolve_setting(&env_context, project_flag, "GCP project", "project")?;
        let zone = resolve_setting(&env_context, zone_flag, "GCE zone", "zone")?;

        let creds = Credentials::load_or_new().await;
        Self::new_with_credentials(env_context, project, zone, creds).await
    }

    /// Initializes a `GceContext` for a resolved project and zone using explicit `Credentials`.
    pub async fn new_with_credentials(
        env_context: EnvironmentContext,
        project: String,
        zone: String,
        creds: Credentials,
    ) -> Result<Self> {
        if creds.oauth2.refresh_token.is_empty() {
            bail!("No Google Cloud credentials found. Run `ffx auth generate`.");
        }
        let access_token = new_access_token(&creds.gcs_credentials()).await?;
        let client = GceClient::new(access_token);

        Ok(Self { env_context, project, zone, client })
    }

    /// Derives the GCE SSH serial port gateway endpoint for this context's zone.
    /// e.g. "us-central1-a" -> "us-central1-ssh-serialport.googleapis.com:9600"
    pub fn serial_endpoint(&self) -> String {
        get_serial_endpoint(&self.zone)
    }

    /// Reads instance state data for an instance in this context's project and zone.
    pub fn read_instance_data(&self, instance_name: &str) -> Result<Option<GceInstanceData>> {
        let instance = Instance::new(&self.project, &self.zone, instance_name)?;
        Ok(instance.read(&self.env_context)?)
    }

    /// Starts a background SSH tunnel to an instance in this context's project and zone.
    pub async fn start_tunnel(&self, instance_name: &str) -> Result<GceInstanceData> {
        GceTunnel::start_tunnel(&self.env_context, &self.project, &self.zone, instance_name).await
    }

    /// Stops the background SSH tunnel for an instance in this context's project and zone.
    pub fn stop_tunnel(&self, instance_name: &str) -> Result<()> {
        GceTunnel::stop_tunnel(&self.env_context, &self.project, &self.zone, instance_name)
    }
}

/// Derives the GCE SSH serial port gateway endpoint for a given zone.
/// e.g. "us-central1-a" -> "us-central1-ssh-serialport.googleapis.com:9600"
pub fn get_serial_endpoint(zone: &str) -> String {
    let parts: Vec<&str> = zone.split('-').collect();
    let region =
        if parts.len() > 1 { parts[..parts.len() - 1].join("-") } else { zone.to_string() };
    format!("{}-ssh-serialport.googleapis.com:9600", region.to_lowercase())
}

fn resolve_setting(
    context: &EnvironmentContext,
    flag: Option<String>,
    name: &str,
    param: &str,
) -> Result<String> {
    let config_key = format!("gce.{param}");
    let flag_name = format!("--{param}");
    flag.filter(|s| !s.is_empty())
        .or_else(|| context.get(&config_key).ok().filter(|s: &String| !s.is_empty()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No {name} specified. Provide {flag_name} or configure via `ffx config set {config_key} <{param}>`."
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[fuchsia::test]
    async fn test_gce_context_unauthenticated() {
        let env = ffx_config::test_init().expect("test env");
        let res = GceContext::new_with_credentials(
            env.context.clone(),
            "test-project".to_string(),
            "test-zone".to_string(),
            Credentials::new(),
        )
        .await;
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();
        assert!(err.contains("No Google Cloud credentials found"));
        assert!(err.contains("ffx auth generate"));
    }

    #[fuchsia::test]
    async fn test_gce_context_missing_zone() {
        let env = ffx_config::test_init().expect("test env");
        let res =
            GceContext::new(env.context.clone(), Some("test-project".to_string()), None).await;
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();
        assert!(err.contains("No GCE zone specified"));
        assert!(err.contains("ffx config set gce.zone"));
    }

    #[fuchsia::test]
    async fn test_gce_context_missing_project() {
        let env = ffx_config::test_init().expect("test env");
        let res = GceContext::new(env.context.clone(), None, Some("test-zone".to_string())).await;
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();
        assert!(err.contains("No GCP project specified"));
        assert!(err.contains("ffx config set gce.project"));
    }

    #[fuchsia::test]
    async fn test_gce_context_struct() {
        let env = ffx_config::test_init().expect("test env");
        let ctx = GceContext {
            env_context: env.context.clone(),
            project: "test-proj".to_string(),
            zone: "test-zone".to_string(),
            client: GceClient::new("token123".to_string()),
        };
        assert_eq!(ctx.project, "test-proj");
        assert_eq!(ctx.zone, "test-zone");
        assert_eq!(ctx.env_context.get::<String, _>("gce.project").ok(), Some("".to_string()));
    }

    #[test]
    fn test_serial_endpoint() {
        assert_eq!(
            get_serial_endpoint("us-central1-a"),
            "us-central1-ssh-serialport.googleapis.com:9600"
        );
        assert_eq!(
            get_serial_endpoint("europe-west1-b"),
            "europe-west1-ssh-serialport.googleapis.com:9600"
        );
    }
}
