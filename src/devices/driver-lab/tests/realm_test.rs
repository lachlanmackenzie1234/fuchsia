// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Driver realm tests for the driver-lab driver.
//!
//! A DriverTestRealm boots the lab_root test driver, which adds one
//! node marked as a proxy target and offers it a fake platform device
//! with a single VMO-backed MMIO. The proxy under test binds to that
//! node through its real bind rules, maps the MMIO, and serves the
//! `fuchsia.driver.lab` wire contract, which these tests exercise
//! through real FIDL: identity, policy enforcement, reads, and audit.

use anyhow::{Context, Result};
use fidl::endpoints::create_proxy;
use fidl_fuchsia_driver_lab as flab;
use fidl_fuchsia_driver_test::RealmArgs;
use fuchsia_component::client::Service;
use fuchsia_component_test::{Capability, RealmBuilder};
use fuchsia_driver_test::{DriverTestRealmBuilder, DriverTestRealmInstance};

// Seeded by the lab_root test driver.
const MMIO_SIZE: u64 = 0x1000;
const SEED_VALUE: u32 = 0xFEED_FACE;
const SEED_OFFSET: u64 = 0x10;

async fn start_realm() -> Result<(fuchsia_component_test::RealmInstance, flab::Proxy_Proxy)> {
    let builder = RealmBuilder::new().await?;
    builder.driver_test_realm_setup().await?;
    let dtr_exposes = vec![Capability::service::<flab::ServiceMarker>().into()];
    builder.driver_test_realm_add_dtr_exposes(&dtr_exposes).await?;
    let instance = builder.build().await?;
    instance
        .driver_test_realm_start(RealmArgs { dtr_exposes: Some(dtr_exposes), ..Default::default() })
        .await?;
    let device = Service::open_from_dir(instance.root.get_exposed_dir(), flab::ServiceMarker)
        .context("failed to open fuchsia.driver.lab service")?
        .watch_for_any()
        .await
        .context("failed to find a proxy instance")?;
    let proxy = device.connect_to_proxy()?;
    Ok((instance, proxy))
}

fn expectations(description: &flab::ProxyDescription) -> flab::Expectations {
    flab::Expectations {
        boot_id: description.boot_id.clone(),
        proxy_generation: description.proxy_generation,
        resource_digest: description.resource_digest.clone(),
        policy_digest: description.policy_digest.clone(),
        ..Default::default()
    }
}

fn run_context() -> flab::RunContext {
    flab::RunContext {
        run_id: Some("realm-run".to_string()),
        case_id: Some("realm-case".to_string()),
        plan_digest: Some("sha256:realm".to_string()),
        host_tool_version: Some("realm-test".to_string()),
        ..Default::default()
    }
}

fn read_rule(offset: u64) -> flab::AccessRule {
    flab::AccessRule { resource: 0, offset, width: 4, class: flab::AccessClass::ReadOnce }
}

async fn open_session(
    proxy: &flab::Proxy_Proxy,
    description: &flab::ProxyDescription,
    allowlist: Vec<flab::AccessRule>,
) -> Result<flab::SessionProxy> {
    let (session, server) = create_proxy::<flab::SessionMarker>();
    proxy
        .open_session(
            &run_context(),
            flab::SessionMode::ReadOnly,
            &expectations(description),
            &allowlist,
            server,
        )
        .await?
        .map_err(|error| anyhow::anyhow!("open_session rejected: {error:?}"))?;
    Ok(session)
}

#[fuchsia::test]
async fn describe_reports_real_identity_and_resources() -> Result<()> {
    let (_instance, proxy) = start_realm().await?;
    let description = proxy.describe().await?;

    assert_eq!(description.protocol_major, Some(1));
    // Real per-boot identity: 16 random bytes as hex, never a stub.
    let boot_id = description.boot_id.clone().expect("boot_id present");
    assert_eq!(boot_id.len(), 32);
    assert_ne!(boot_id, "unknown");
    assert!(description.proxy_generation.unwrap_or(0) > 0);
    assert_eq!(description.takeover, Some(flab::TakeoverState::NotActive));

    // Exactly the one MMIO the fake platform device offers, with a
    // populated per-resource digest, and distinct instance digests.
    let resources = description.resources.clone().expect("resources present");
    assert_eq!(resources.len(), 1);
    let resource = &resources[0];
    assert_eq!(resource.id, Some(0));
    assert_eq!(resource.name.as_deref(), Some("mmio0"));
    assert_eq!(resource.logical_size, Some(MMIO_SIZE));
    assert_eq!(resource.kind, Some(flab::ResourceKind::Mmio));
    assert!(resource.digest.as_deref().unwrap_or("").starts_with("sha256:"));
    assert_ne!(description.resource_digest, description.policy_digest);
    Ok(())
}

#[fuchsia::test]
async fn exact_allowed_read_reaches_real_mmio() -> Result<()> {
    let (_instance, proxy) = start_realm().await?;
    let description = proxy.describe().await?;
    let session = open_session(&proxy, &description, vec![read_rule(SEED_OFFSET)]).await?;

    // The allowed read returns the value lab_root seeded in the VMO.
    let (value, _audit_seq, _timestamp_ns) = session
        .read32(0, SEED_OFFSET)
        .await?
        .map_err(|error| anyhow::anyhow!("read32 denied: {error:?}"))?;
    assert_eq!(value, SEED_VALUE);

    // An offset outside the exact allowlist is denied, not read.
    let denied = session.read32(0, 0x20).await?;
    assert_eq!(denied.unwrap_err(), flab::OperationError::NotInAllowlist);

    // A read-once rule never authorizes the snapshot class.
    let snapshot =
        session.snapshot(&[flab::SnapshotItem { resource: 0, offset: SEED_OFFSET }]).await?;
    assert_eq!(snapshot.unwrap_err(), flab::OperationError::NotInAllowlist);

    // The audit trail covers the session open, the allowed read, and
    // both rejections, stamped with instance identity.
    let page = session.read_audit(0, 64).await?;
    let entries = page.0;
    let operations: Vec<String> =
        entries.iter().filter_map(|entry| entry.operation.clone()).collect();
    assert!(operations.iter().any(|operation| operation == "open_session"));
    assert!(operations.iter().any(|operation| operation == "read32"));
    let denied_count =
        entries.iter().filter(|entry| entry.decision == Some(flab::AuditDecision::Denied)).count();
    assert_eq!(denied_count, 2);
    for entry in &entries {
        assert_eq!(entry.boot_id, description.boot_id);
        assert_eq!(entry.proxy_generation, description.proxy_generation);
    }
    let open_entry = entries
        .iter()
        .find(|entry| entry.operation.as_deref() == Some("open_session"))
        .expect("open_session audit entry");
    assert_eq!(open_entry.run_id.as_deref(), Some("realm-run"));
    Ok(())
}

#[fuchsia::test]
async fn stale_and_reserved_expectations_are_rejected() -> Result<()> {
    let (_instance, proxy) = start_realm().await?;
    let description = proxy.describe().await?;

    // A stale boot id creates no session.
    let (_, server) = create_proxy::<flab::SessionMarker>();
    let mut stale = expectations(&description);
    stale.boot_id = Some("boot-stale".to_string());
    let result = proxy
        .open_session(&run_context(), flab::SessionMode::ReadOnly, &stale, &[], server)
        .await?;
    assert_eq!(result.unwrap_err(), flab::OpenSessionError::StaleBootId);

    // A reserved phase 2 expectation fails closed.
    let (_, server) = create_proxy::<flab::SessionMarker>();
    let mut reserved = expectations(&description);
    reserved.topology_generation = Some(7);
    let result = proxy
        .open_session(&run_context(), flab::SessionMode::ReadOnly, &reserved, &[], server)
        .await?;
    assert_eq!(result.unwrap_err(), flab::OpenSessionError::UnsupportedExpectation);

    // A rule outside the offered resources rejects the whole allowlist.
    let (_, server) = create_proxy::<flab::SessionMarker>();
    let result = proxy
        .open_session(
            &run_context(),
            flab::SessionMode::ReadOnly,
            &expectations(&description),
            &[flab::AccessRule {
                resource: 9,
                offset: 0,
                width: 4,
                class: flab::AccessClass::ReadOnce,
            }],
            server,
        )
        .await?;
    assert_eq!(result.unwrap_err(), flab::OpenSessionError::RejectedAllowlist);
    Ok(())
}
