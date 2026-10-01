// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

mod platform_provider;
mod server;

use fdf_component::{Driver, DriverContext, DriverError, Node, driver_register};
use fidl_fuchsia_driver_lab as flab;
use fuchsia_async as fasync;
use fuchsia_component::server::ServiceFs;
use futures::StreamExt;
use lab_proxy_core::audit_ring::{AuditRecord, AuditRing};
use lab_proxy_core::executor::{ExecLimits, Executor};
use lab_proxy_core::session::{ProxyIdentity, SessionManager};
use log::{info, warn};
use server::{ProxyState, SharedState, ZxClock};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Number of audit entries retained before the ring wraps. This moves into
/// the immutable target ceiling once policy loading exists.
const AUDIT_CAPACITY: usize = 1024;

/// Maximum snapshot items accepted, within the wire-contract bound.
const MAX_SNAPSHOT_ITEMS: usize = 64;

/// Maximum sequence items accepted, within the wire-contract bound.
const MAX_SEQUENCE_ITEMS: usize = 64;

/// Maximum delay accepted in a single sequence step (1 second).
const MAX_DELAY_NS: i64 = 1_000_000_000;

struct LabProxy {
    _node: Node,
    _scope: fasync::Scope,
    state: SharedState,
}

driver_register!(LabProxy);

fn now_ns() -> i64 {
    zx::MonotonicInstant::get().into_nanos()
}

impl Driver for LabProxy {
    const NAME: &str = "lab_proxy";

    async fn start(mut context: DriverContext) -> Result<Self, DriverError> {
        let node = context.take_node()?;
        let node_identity = context
            .start_args
            .node_name
            .clone()
            .unwrap_or_else(|| "driver-lab.unnamed".to_string());

        // Acquire whatever the bound node offers. A node without a
        // platform device yields zero resources; a node whose MMIOs
        // cannot all be mapped fails start rather than serving a
        // partially acquired identity.
        let bundle =
            platform_provider::acquire(&context, &node_identity).await.map_err(|error| {
                warn!("resource acquisition failed: {error}");
                DriverError::Status(zx::Status::INTERNAL)
            })?;
        if let Err(error) = bundle.validate() {
            warn!("provider bundle is inconsistent: {error:?}");
            return Err(DriverError::Status(zx::Status::INTERNAL));
        }

        // Kernel randomness is the per-boot identity source: there is no
        // kernel boot UUID, and this is the same mechanism RCS uses for
        // ffx's reboot detection. A driver-host restart also regenerates
        // it, which is conservatively correct for staleness -- the
        // boot-timeline generation below distinguishes restart from
        // reboot (it keeps increasing across restarts within one boot).
        let mut boot_id_bytes = [0u8; 16];
        zx::cprng_draw(&mut boot_id_bytes);
        let boot_id: String = boot_id_bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let proxy_generation = zx::BootInstant::get().into_nanos() as u64;

        let manifest = lab_proxy_core::target_policy::TargetPolicyManifest::engineering_default(
            &bundle.resources,
        );
        let policy_digest = manifest.policy_digest().to_string();
        let digests = bundle.digests();
        let identity = ProxyIdentity {
            boot_id,
            proxy_generation,
            resource_digest: bundle.combined_digest().to_string(),
            policy_digest,
        };
        let resource_digests: BTreeMap<u32, String> =
            digests.iter().map(|(id, digest)| (*id, digest.to_string())).collect();
        let sessions = SessionManager::new(
            identity,
            bundle.resources,
            manifest.to_ceiling_map(),
            manifest.allow_mutating_sessions,
        );
        let executor = Executor::new(
            bundle.backends,
            ZxClock,
            ExecLimits {
                max_snapshot_items: MAX_SNAPSHOT_ITEMS,
                max_sequence_items: MAX_SEQUENCE_ITEMS,
                max_delay_ns: MAX_DELAY_NS,
            },
        );
        let mut audit = AuditRing::new(AUDIT_CAPACITY);
        audit.append(AuditRecord::lifecycle("driver_start", now_ns()));
        let state: SharedState =
            Arc::new(Mutex::new(ProxyState { sessions, executor, audit, resource_digests }));

        let scope = fasync::Scope::new_with_name(Self::NAME);
        let mut outgoing = ServiceFs::new();
        outgoing.dir("svc").add_fidl_service_instance(
            "default",
            |request: flab::ServiceRequest| {
                let flab::ServiceRequest::Proxy(stream) = request;
                stream
            },
        );
        context.serve_outgoing(&mut outgoing)?;

        {
            let state = state.clone();
            let handle = scope.to_handle();
            scope.spawn(async move {
                let mut outgoing = outgoing;
                while let Some(stream) = outgoing.next().await {
                    handle.spawn(server::serve_proxy(state.clone(), handle.clone(), stream));
                }
            });
        }

        info!("LabProxy started; serving fuchsia.driver.lab");
        Ok(Self { _node: node, _scope: scope, state })
    }

    async fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.sessions.reject_new_sessions();
        let seq = state.audit.append(AuditRecord::lifecycle("driver_stop", now_ns()));
        info!("LabProxy::stop() audit seq {seq}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fdf_component::testing::harness::TestHarness;

    #[fuchsia::test]
    async fn test_driver_start() {
        let mut harness = TestHarness::<LabProxy>::new();
        let started_driver = harness.start_driver().await.unwrap();

        // Verify driver started successfully
        assert!(true);

        started_driver.stop_driver().await;
    }
}
