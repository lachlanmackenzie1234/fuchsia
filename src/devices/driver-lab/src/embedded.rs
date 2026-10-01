// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Embedded in-situ driver library (`driver_lab_rust::embedded`) for DFv2
//! Rust drivers (Spec Phase 2 Sections 2 & 3.1).
//!
//! Enables an active, bound driver to register its mapped MMIO banks (via
//! VMO duplication), protocol resources, and interrupt channels with an
//! embedded `fuchsia.driver.lab` server published on the driver's outgoing
//! [`ServiceFs`].

use crate::fuchsia_backends::{
    FuchsiaClock, FuchsiaGpio, FuchsiaI2c, FuchsiaReset, FuchsiaSerial, FuchsiaSpi, LiveBackend,
};
use crate::platform_provider::MappedMmio;
use crate::server::{self, ProxyState, SharedState, SharedStateData, ZxClock};
use fidl_fuchsia_driver_lab as flab;
use fidl_fuchsia_hardware_clock as fclock;
use fidl_fuchsia_hardware_gpio as fgpio;
use fidl_fuchsia_hardware_i2c as fi2c;
use fidl_fuchsia_hardware_reset as freset;
use fidl_fuchsia_hardware_serial as fserial;
use fidl_fuchsia_hardware_spi as fspi;
use fuchsia_async as fasync;
use fuchsia_component::server::{ServiceFs, ServiceObj, ServiceObjLocal};
use lab_proxy_core::access_policy::{ProtocolCeiling, ResourceCeiling, ResourceId, ResourceKind};
use lab_proxy_core::audit_ring::{AuditRecord, AuditRing};
use lab_proxy_core::config::{AccessLimitEnforcer, ProxyConfig};
use lab_proxy_core::executor::{ExecLimits, Executor};
use lab_proxy_core::interrupt::InterruptManager;
use lab_proxy_core::provider::{ProvidedResources, ProviderError};
use lab_proxy_core::session::{ProxyIdentity, SessionManager};
use lab_proxy_core::target_policy::TargetPolicyManifest;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn now_ns() -> i64 {
    zx::MonotonicInstant::get().into_nanos()
}

/// Builder for configuring and instantiating an [`EmbeddedLabServer`] inside
/// an existing DFv2 Rust driver.
pub struct DriverLabBuilder {
    bundle: ProvidedResources<LiveBackend>,
    next_id: ResourceId,
    allow_mutating_sessions: bool,
    config: ProxyConfig,
}

impl DriverLabBuilder {
    /// Creates a new builder for the given stable node/driver identity.
    pub fn new(node_identity: impl Into<String>) -> Self {
        Self {
            bundle: ProvidedResources::embedded(&node_identity.into()),
            next_id: 0,
            allow_mutating_sessions: true,
            config: ProxyConfig::default(),
        }
    }

    /// Creates a new builder extracting the node identity from `DriverContext`.
    pub fn from_context(context: &fdf_component::DriverContext) -> Self {
        let identity = context
            .start_args
            .node_name
            .clone()
            .unwrap_or_else(|| "driver-lab.embedded".to_string());
        Self::new(identity)
    }

    /// Sets whether mutating sessions are permitted on this embedded server.
    pub fn with_allow_mutating_sessions(mut self, allow: bool) -> Self {
        self.allow_mutating_sessions = allow;
        self
    }

    /// Overrides the proxy configuration bounds (narrowing against baseline).
    pub fn with_config(mut self, config: ProxyConfig) -> Self {
        if let Ok(narrowed) = ProxyConfig::default().narrow_with(&config) {
            self.config = narrowed;
        }
        self
    }

    /// Ergonomic helper to register a driver MMIO VMO by byte size.
    pub fn with_mmio(
        &mut self,
        name: impl Into<String>,
        vmo: &zx::Vmo,
        offset: usize,
        size: usize,
    ) -> Result<ResourceId, zx::Status> {
        self.add_mmio_vmo(name, vmo, offset, size as u64)
    }

    /// Configures 32-bit writable register offsets on a registered MMIO resource.
    pub fn with_writable_registers(&mut self, id: ResourceId, offsets: Vec<u64>) -> &mut Self {
        if let Some(ceiling) = self.bundle.ceiling.get_mut(&id) {
            ceiling.writable_registers = offsets
                .into_iter()
                .map(|offset| lab_proxy_core::access_policy::WritableRegister {
                    offset,
                    width: 4,
                    allow_mask: 0xFFFF_FFFF,
                    allow_rmw: true,
                    require_precondition: false,
                    precondition_mask: 0,
                    readback: true,
                })
                .collect();
        }
        self
    }

    /// Configures hard-denied byte offset ranges `(start, end)` on a registered MMIO resource.
    pub fn with_hard_denied_ranges(
        &mut self,
        id: ResourceId,
        ranges: Vec<(u64, u64)>,
    ) -> &mut Self {
        if let Some(ceiling) = self.bundle.ceiling.get_mut(&id) {
            ceiling.hard_denied = ranges.into_iter().map(|(start, end)| start..end).collect();
        }
        self
    }

    /// Ergonomic alias for [`Self::add_interrupt`].
    pub fn with_interrupt(&mut self, name: impl Into<String>) -> ResourceId {
        self.add_interrupt(name)
    }

    /// Shares an existing driver MMIO VMO with the embedded server using a
    /// default read/poll-permissive ceiling.
    ///
    /// Duplicates `vmo` (`zx::Rights::SAME_RIGHTS`) and creates an independent
    /// local mapping in the driver host address space.
    pub fn add_mmio_vmo(
        &mut self,
        name: impl Into<String>,
        vmo: &zx::Vmo,
        offset: usize,
        logical_size: u64,
    ) -> Result<ResourceId, zx::Status> {
        let ceiling = ResourceCeiling {
            hard_denied: vec![],
            allow_unknown_reads: true,
            allow_poll: true,
            writable_registers: vec![],
            protocol: None,
            allow_interrupt: false,
        };
        self.add_mmio_vmo_with_ceiling(name, vmo, offset, logical_size, ceiling)
    }

    /// Shares an existing driver MMIO VMO with an explicit [`ResourceCeiling`]
    /// (such as `hard_denied` ranges for clear-on-read/FIFO registers and
    /// `writable_registers` for permitted mutations).
    pub fn add_mmio_vmo_with_ceiling(
        &mut self,
        name: impl Into<String>,
        vmo: &zx::Vmo,
        offset: usize,
        logical_size: u64,
        ceiling: ResourceCeiling,
    ) -> Result<ResourceId, zx::Status> {
        let map_len = usize::try_from(logical_size).map_err(|_| zx::Status::OUT_OF_RANGE)?;
        let mmio = MappedMmio::from_vmo(vmo, offset, map_len)?;
        Ok(self.add_mapped_mmio(name, mmio, logical_size, ceiling))
    }

    /// Registers a pre-mapped [`MappedMmio`] region with an explicit [`ResourceCeiling`].
    pub fn add_mapped_mmio(
        &mut self,
        name: impl Into<String>,
        mmio: MappedMmio,
        logical_size: u64,
        ceiling: ResourceCeiling,
    ) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        let mapped_size = mmio.len() as u64;
        self.bundle.add_mmio(
            id,
            name,
            logical_size,
            mapped_size,
            ceiling,
            LiveBackend::Mmio(mmio),
        );
        id
    }

    /// Registers a GPIO protocol resource.
    pub fn add_gpio(
        &mut self,
        name: impl Into<String>,
        proxy: fgpio::GpioSynchronousProxy,
    ) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.bundle.add_protocol(
            id,
            name,
            ResourceKind::Gpio,
            ProtocolCeiling::default_for(ResourceKind::Gpio),
            LiveBackend::Gpio(FuchsiaGpio::new(proxy)),
        );
        id
    }

    /// Registers an I2C protocol resource.
    pub fn add_i2c(
        &mut self,
        name: impl Into<String>,
        proxy: fi2c::DeviceSynchronousProxy,
    ) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.bundle.add_protocol(
            id,
            name,
            ResourceKind::I2c,
            ProtocolCeiling::default_for(ResourceKind::I2c),
            LiveBackend::I2c(FuchsiaI2c::new(proxy)),
        );
        id
    }

    /// Registers a SPI protocol resource.
    pub fn add_spi(
        &mut self,
        name: impl Into<String>,
        proxy: fspi::DeviceSynchronousProxy,
    ) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.bundle.add_protocol(
            id,
            name,
            ResourceKind::Spi,
            ProtocolCeiling::default_for(ResourceKind::Spi),
            LiveBackend::Spi(FuchsiaSpi::new(proxy)),
        );
        id
    }

    /// Registers a Clock protocol resource.
    pub fn add_clock(
        &mut self,
        name: impl Into<String>,
        proxy: fclock::ClockSynchronousProxy,
    ) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.bundle.add_protocol(
            id,
            name,
            ResourceKind::Clock,
            ProtocolCeiling::default_for(ResourceKind::Clock),
            LiveBackend::Clock(FuchsiaClock::new(proxy)),
        );
        id
    }

    /// Registers a Reset protocol resource.
    pub fn add_reset(
        &mut self,
        name: impl Into<String>,
        proxy: freset::ResetSynchronousProxy,
    ) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.bundle.add_protocol(
            id,
            name,
            ResourceKind::Reset,
            ProtocolCeiling::default_for(ResourceKind::Reset),
            LiveBackend::Reset(FuchsiaReset::new(proxy)),
        );
        id
    }

    /// Registers a Serial protocol resource.
    pub fn add_serial(
        &mut self,
        name: impl Into<String>,
        proxy: fserial::DeviceSynchronousProxy,
    ) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.bundle.add_protocol(
            id,
            name,
            ResourceKind::Serial,
            ProtocolCeiling::default_for(ResourceKind::Serial),
            LiveBackend::Serial(FuchsiaSerial::new(proxy)),
        );
        id
    }

    /// Registers an interrupt observation resource for event tapping.
    pub fn add_interrupt(&mut self, name: impl Into<String>) -> ResourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.bundle.add_interrupt(id, name, LiveBackend::Interrupt);
        id
    }

    /// Validates all registered resources and builds the [`EmbeddedLabServer`].
    pub fn build(self) -> Result<EmbeddedLabServer, ProviderError> {
        self.bundle.validate()?;

        let mut boot_id_bytes = [0u8; 16];
        zx::cprng_draw(&mut boot_id_bytes);
        let boot_id: String = boot_id_bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let proxy_generation = zx::BootInstant::get().into_nanos() as u64;

        let manifest = TargetPolicyManifest::from_bundle_ceiling(
            &self.bundle.resources,
            &self.bundle.ceiling,
            self.allow_mutating_sessions,
        );
        let policy_digest = manifest.policy_digest().to_string();
        let digests = self.bundle.digests();
        let identity = ProxyIdentity {
            boot_id,
            proxy_generation,
            resource_digest: self.bundle.combined_digest().to_string(),
            policy_digest,
        };
        let resource_digests: BTreeMap<u32, String> =
            digests.iter().map(|(id, digest)| (*id, digest.to_string())).collect();

        let mut interrupts = InterruptManager::new();
        for (id, res) in &self.bundle.resources {
            if res.kind == ResourceKind::Interrupt {
                interrupts.register(*id);
            }
        }

        let limit_enforcer = AccessLimitEnforcer::new(
            self.config.max_ops_per_second,
            self.config.max_deadline_ns,
        );
        let mut sessions = SessionManager::with_limits(
            identity,
            self.bundle.resources,
            self.bundle.ceiling,
            self.allow_mutating_sessions,
            limit_enforcer,
        );
        if !self.config.enabled {
            sessions.reject_new_sessions();
        }

        let executor = Executor::new(
            self.bundle.backends,
            ZxClock,
            ExecLimits {
                max_snapshot_items: self.config.max_snapshot_items as usize,
                max_sequence_items: self.config.max_sequence_items as usize,
                max_delay_ns: self.config.max_delay_ns as i64,
                max_sequence_duration_ns: self.config.max_sequence_duration_ns as i64,
            },
        );
        let mut audit = AuditRing::new(self.config.audit_capacity as usize);
        audit.append(AuditRecord::lifecycle("embedded_driver_start", now_ns()));

        let state: SharedState = Arc::new(SharedStateData {
            inner: Mutex::new(ProxyState {
                sessions,
                executor,
                audit,
                resource_digests,
                interrupts,
                config: self.config,
            }),
            abort_token: std::sync::atomic::AtomicBool::new(false),
        });

        Ok(EmbeddedLabServer { state })
    }
}

/// Handle to the embedded `fuchsia.driver.lab` server running inside an
/// existing DFv2 driver.
#[derive(Clone)]
pub struct EmbeddedLabServer {
    state: SharedState,
}

impl EmbeddedLabServer {
    /// Publishes `fuchsia.driver.lab.Service` (`default` instance) onto a
    /// thread-safe driver [`ServiceFs`], spawning connection handlers onto
    /// `scope_handle`.
    pub fn publish(
        &self,
        outgoing: &mut ServiceFs<ServiceObj<'static, ()>>,
        scope_handle: fasync::ScopeHandle,
    ) {
        let state = self.state.clone();
        outgoing.dir("svc").add_fidl_service_instance(
            "default",
            move |request: flab::ServiceRequest| {
                let flab::ServiceRequest::Proxy(stream) = request;
                scope_handle.spawn(server::serve_proxy(
                    state.clone(),
                    scope_handle.clone(),
                    stream,
                ));
            },
        );
    }

    /// Publishes `fuchsia.driver.lab.Service` (`default` instance) onto a
    /// local driver [`ServiceFs`], spawning connection handlers onto
    /// `scope_handle`.
    pub fn publish_local(
        &self,
        outgoing: &mut ServiceFs<ServiceObjLocal<'static, ()>>,
        scope_handle: fasync::ScopeHandle,
    ) {
        let state = self.state.clone();
        outgoing.dir("svc").add_fidl_service_instance(
            "default",
            move |request: flab::ServiceRequest| {
                let flab::ServiceRequest::Proxy(stream) = request;
                scope_handle.spawn(server::serve_proxy(
                    state.clone(),
                    scope_handle.clone(),
                    stream,
                ));
            },
        );
    }

    /// Spawns a single `fuchsia.driver.lab.Proxy` stream on `scope_handle`.
    pub fn serve_proxy_stream(
        &self,
        scope_handle: fasync::ScopeHandle,
        stream: flab::Proxy_RequestStream,
    ) {
        let state = self.state.clone();
        scope_handle.spawn(server::serve_proxy(state, scope_handle.clone(), stream));
    }

    /// Returns a reference to the underlying shared proxy state.
    pub fn state(&self) -> &SharedState {
        &self.state
    }

    /// Stops accepting new sessions, cancels pending interrupt waiters, and
    /// appends a stop lifecycle audit record.
    pub fn stop(&self) {
        self.state.abort_token.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut state = self.state.inner.lock().unwrap();
        state.sessions.reject_new_sessions();
        state.interrupts.cancel_waiters();
        state.audit.append(AuditRecord::lifecycle("embedded_driver_stop", now_ns()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lab_proxy_core::access_policy::WritableRegister;
    use mmio::Mmio as _;
    use mmio::vmo::VmoMapping;
    use zx::CachePolicy;

    #[fuchsia::test]
    async fn embedded_server_describes_and_reads_live_mmio_over_fidl() {
        const SIZE: usize = 4096;
        let vmo = zx::Vmo::create(SIZE as u64).unwrap();
        let driver_vmo = vmo.duplicate_handle(zx::Rights::SAME_RIGHTS).unwrap();

        let mut driver_mmio =
            VmoMapping::map_with_cache_policy(0, SIZE, driver_vmo, CachePolicy::Cached).unwrap();
        // Simulated live registers:
        // 0x00 = DEVICE_ID (0xD1A6_0001)
        // 0x04 = STATUS (0x0000_0007)
        // 0x40 = FIFO_DATA (clear-on-read, marked hard_denied)
        driver_mmio.try_store32(0x00, 0xD1A6_0001).unwrap();
        driver_mmio.try_store32(0x04, 0x0000_0007).unwrap();
        driver_mmio.try_store32(0x40, 0xDEAD_BEEF).unwrap();

        let mut builder = DriverLabBuilder::new("sample-driver");
        let mmio_id = builder
            .add_mmio_vmo_with_ceiling(
                "regs",
                &vmo,
                0,
                0x100,
                ResourceCeiling {
                    hard_denied: vec![0x40..0x44],
                    allow_unknown_reads: true,
                    allow_poll: true,
                    writable_registers: vec![WritableRegister {
                        offset: 0x08,
                        width: 4,
                        allow_mask: 0xFFFF_FFFF,
                        allow_rmw: true,
                        require_precondition: false,
                        precondition_mask: 0,
                        readback: true,
                    }],
                    protocol: None,
                    allow_interrupt: false,
                },
            )
            .unwrap();
        assert_eq!(mmio_id, 0);

        let server = builder.build().unwrap();
        let scope = fasync::Scope::new_with_name("embedded-test");
        let (proxy, stream) = fidl::endpoints::create_proxy_and_stream::<flab::Proxy_Marker>();
        server.serve_proxy_stream(scope.to_handle(), stream);

        // 1. Describe the embedded server
        let desc = proxy.describe().await.unwrap();
        assert_eq!(desc.protocol_major, Some(1));
        let resources = desc.resources.as_ref().unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name.as_deref(), Some("regs"));
        assert_eq!(resources[0].logical_size, Some(0x100));

        // 2. Open a read-only session allowing 0x00 and 0x04
        let (session_proxy, session_server) =
            fidl::endpoints::create_proxy::<flab::SessionMarker>();
        let open_res = proxy
            .open_session(
                &flab::RunContext {
                    run_id: Some("run-cs25".to_string()),
                    case_id: Some("case-cs25".to_string()),
                    plan_digest: Some("sha256:test".to_string()),
                    host_tool_version: Some("0.1.0".to_string()),
                    ..Default::default()
                },
                flab::SessionMode::ReadOnly,
                &flab::Expectations {
                    boot_id: desc.boot_id.clone(),
                    proxy_generation: desc.proxy_generation,
                    resource_digest: desc.resource_digest.clone(),
                    policy_digest: desc.policy_digest.clone(),
                    ..Default::default()
                },
                &[
                    flab::AccessRule {
                        resource: 0,
                        offset: 0x00,
                        width: 4,
                        class: flab::AccessClass::ReadOnce,
                    },
                    flab::AccessRule {
                        resource: 0,
                        offset: 0x04,
                        width: 4,
                        class: flab::AccessClass::Snapshot,
                    },
                ],
                session_server,
            )
            .await
            .unwrap();
        assert!(open_res.is_ok());

        // 3. Read32 at 0x00 reflects live driver state
        let (read_val, _, _) = session_proxy.read32(0, 0x00).await.unwrap().unwrap();
        assert_eq!(read_val, 0xD1A6_0001);

        // 4. Snapshot at 0x04 reflects live driver state
        let (snap_results, snap_complete) = session_proxy
            .snapshot(&[flab::SnapshotItem { resource: 0, offset: 0x04 }])
            .await
            .unwrap()
            .unwrap();
        assert!(snap_complete);
        assert_eq!(snap_results.len(), 1);
        assert_eq!(snap_results[0].value, 0x0000_0007);

        // 5. Hard-denied destructive FIFO register at 0x40 is rejected when opening a session
        let (_bad_session_proxy, bad_session_server) =
            fidl::endpoints::create_proxy::<flab::SessionMarker>();
        let bad_open = proxy
            .open_session(
                &flab::RunContext::default(),
                flab::SessionMode::ReadOnly,
                &flab::Expectations {
                    boot_id: desc.boot_id.clone(),
                    proxy_generation: desc.proxy_generation,
                    resource_digest: desc.resource_digest.clone(),
                    policy_digest: desc.policy_digest.clone(),
                    ..Default::default()
                },
                &[flab::AccessRule {
                    resource: 0,
                    offset: 0x40,
                    width: 4,
                    class: flab::AccessClass::ReadOnce,
                }],
                bad_session_server,
            )
            .await
            .unwrap();
        assert_eq!(bad_open, Err(flab::OpenSessionError::RejectedAllowlist));

        // 6. Stop rejects new sessions
        server.stop();
    }
}
