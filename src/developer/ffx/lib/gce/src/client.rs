// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::models::{Instance, InstanceList, Operation, SerialPortOutput};
use anyhow::{Context, Result, bail};
use fuchsia_hyper::{HttpsClient, new_https_client};
use http_body_util::BodyExt;
use hyper::{Method, Request};
use serde::de::DeserializeOwned;
use std::time::Duration;
use url::Url;

const COMPUTE_BASE: &str = "https://compute.googleapis.com/compute/v1";

pub type Body = http_body_util::Full<hyper::body::Bytes>;

#[derive(Debug)]
pub struct GceClient {
    base_url: Url,
    access_token: String,
    https_client: HttpsClient,
}

impl GceClient {
    pub fn new(access_token: String) -> Self {
        Self {
            base_url: Url::parse(COMPUTE_BASE).expect("valid compute base URL"),
            access_token,
            https_client: new_https_client(),
        }
    }

    async fn send_request<T: DeserializeOwned>(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
    ) -> Result<T> {
        let mut builder = Request::builder().method(&method).uri(url.as_str());

        if !self.access_token.is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", self.access_token));
        }

        let body_bytes = body.unwrap_or_default();
        if method == Method::POST || !body_bytes.is_empty() {
            builder = builder
                .header("Content-Type", "application/json")
                .header("Content-Length", body_bytes.len().to_string());
        }

        let req = builder.body(Body::from(body_bytes)).context("Failed to build request")?;

        let res = self.https_client.request(req).await.context("HTTP request failed")?;
        let status = res.status();

        let collected = res.into_body().collect().await.context("Failed to read response body")?;
        let bytes = collected.to_bytes();

        if !status.is_success() {
            let error_text = String::from_utf8_lossy(&bytes);
            bail!("GCE API returned error status {}: {}", status, error_text);
        }

        let parsed: T = serde_json::from_slice(&bytes).context("Failed to parse JSON response")?;
        Ok(parsed)
    }

    fn instance_url(&self, project: &str, zone: &str, instance_name: &str) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("Invalid base URL"))?.extend(&[
            "projects",
            project,
            "zones",
            zone,
            "instances",
            instance_name,
        ]);
        Ok(url)
    }

    fn serial_port_output_url(
        &self,
        project: &str,
        zone: &str,
        instance_name: &str,
        port: u32,
        start: Option<i64>,
    ) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("Invalid base URL"))?.extend(&[
            "projects",
            project,
            "zones",
            zone,
            "instances",
            instance_name,
            "serialPort",
        ]);
        url.query_pairs_mut().append_pair("port", &port.to_string());
        if let Some(s) = start {
            url.query_pairs_mut().append_pair("start", &s.to_string());
        }
        Ok(url)
    }

    fn list_instances_url(&self, project: &str, zone: &str) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("Invalid base URL"))?.extend(&[
            "projects",
            project,
            "zones",
            zone,
            "instances",
        ]);
        Ok(url)
    }

    pub async fn get_instance(
        &self,
        project: &str,
        zone: &str,
        instance_name: &str,
    ) -> Result<Instance> {
        let url = self.instance_url(project, zone, instance_name)?;
        self.send_request(Method::GET, url, None).await
    }

    pub async fn get_serial_port_output(
        &self,
        project: &str,
        zone: &str,
        instance_name: &str,
        port: u32,
        start: Option<i64>,
    ) -> Result<SerialPortOutput> {
        let url = self.serial_port_output_url(project, zone, instance_name, port, start)?;
        self.send_request(Method::GET, url, None).await
    }

    pub async fn list_instances(&self, project: &str, zone: &str) -> Result<Vec<Instance>> {
        let url = self.list_instances_url(project, zone)?;
        let list: InstanceList = self.send_request(Method::GET, url, None).await?;
        Ok(list.items)
    }

    fn stop_instance_url(&self, project: &str, zone: &str, instance_name: &str) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("Invalid base URL"))?.extend(&[
            "projects",
            project,
            "zones",
            zone,
            "instances",
            instance_name,
            "stop",
        ]);
        Ok(url)
    }

    fn zone_operation_url(&self, project: &str, zone: &str, op_name: &str) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("Invalid base URL"))?.extend(&[
            "projects",
            project,
            "zones",
            zone,
            "operations",
            op_name,
        ]);
        Ok(url)
    }

    pub async fn delete_instance(
        &self,
        project: &str,
        zone: &str,
        instance_name: &str,
    ) -> Result<Operation> {
        let url = self.instance_url(project, zone, instance_name)?;
        self.send_request(Method::DELETE, url, None).await
    }

    pub async fn stop_instance(
        &self,
        project: &str,
        zone: &str,
        instance_name: &str,
    ) -> Result<Operation> {
        let url = self.stop_instance_url(project, zone, instance_name)?;
        self.send_request(Method::POST, url, None).await
    }

    pub async fn get_zone_operation(
        &self,
        project: &str,
        zone: &str,
        op_name: &str,
    ) -> Result<Operation> {
        let url = self.zone_operation_url(project, zone, op_name)?;
        self.send_request(Method::GET, url, None).await
    }

    async fn wait_for_operation<F, Fut>(&self, mut get_op: F) -> Result<()>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Operation>>,
    {
        loop {
            let op = get_op().await?;

            if let Some(err) = op.error {
                let err_msg = err
                    .errors
                    .into_iter()
                    .map(|e| {
                        format!("{}: {}", e.code.unwrap_or_default(), e.message.unwrap_or_default())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("Operation failed: {}", err_msg);
            }

            if op.status.as_deref() == Some("DONE") {
                return Ok(());
            }

            fuchsia_async::Timer::new(Duration::from_secs(2)).await;
        }
    }

    pub async fn wait_for_zone_operation(
        &self,
        project: &str,
        zone: &str,
        op_name: &str,
    ) -> Result<()> {
        self.wait_for_operation(|| self.get_zone_operation(project, zone, op_name)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_instances_url_construction() {
        let client = GceClient::new("token123".to_string());
        let url = client.list_instances_url("test-p", "test-z").unwrap();
        assert_eq!(
            url.as_str(),
            "https://compute.googleapis.com/compute/v1/projects/test-p/zones/test-z/instances"
        );
    }

    #[test]
    fn test_get_instance_url_construction() {
        let client = GceClient::new("token123".to_string());
        let url = client.instance_url("test-p", "test-z", "test-inst").unwrap();
        assert_eq!(
            url.as_str(),
            "https://compute.googleapis.com/compute/v1/projects/test-p/zones/test-z/instances/test-inst"
        );
    }

    #[test]
    fn test_get_serial_port_output_url_construction() {
        let client = GceClient::new("token123".to_string());
        let url =
            client.serial_port_output_url("test-p", "test-z", "test-inst", 1, Some(100)).unwrap();
        assert_eq!(
            url.as_str(),
            "https://compute.googleapis.com/compute/v1/projects/test-p/zones/test-z/instances/test-inst/serialPort?port=1&start=100"
        );

        let url_no_start =
            client.serial_port_output_url("test-p", "test-z", "test-inst", 1, None).unwrap();
        assert_eq!(
            url_no_start.as_str(),
            "https://compute.googleapis.com/compute/v1/projects/test-p/zones/test-z/instances/test-inst/serialPort?port=1"
        );
    }

    #[test]
    fn test_stop_instance_url_construction() {
        let client = GceClient::new("token123".to_string());
        let url = client.stop_instance_url("test-p", "test-z", "test-inst").unwrap();
        assert_eq!(
            url.as_str(),
            "https://compute.googleapis.com/compute/v1/projects/test-p/zones/test-z/instances/test-inst/stop"
        );
    }

    #[test]
    fn test_delete_instance_url_construction() {
        let client = GceClient::new("token123".to_string());
        let url = client.instance_url("test-p", "test-z", "test-inst").unwrap();
        assert_eq!(
            url.as_str(),
            "https://compute.googleapis.com/compute/v1/projects/test-p/zones/test-z/instances/test-inst"
        );
    }

    #[test]
    fn test_get_zone_operation_url_construction() {
        let client = GceClient::new("token123".to_string());
        let url = client.zone_operation_url("test-p", "test-z", "operation-123").unwrap();
        assert_eq!(
            url.as_str(),
            "https://compute.googleapis.com/compute/v1/projects/test-p/zones/test-z/operations/operation-123"
        );
    }
}
