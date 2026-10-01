// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Provider-neutral bundle of acquired resources.
//!
//! A resource provider (for example the platform-device adapter in the
//! driver crate) discovers the resources offered to the bound node and
//! produces one [`ProvidedResources`]: descriptions, the immutable
//! ceiling entries that apply to them, and a backend per resource. The
//! bundle is validated before the proxy serves anything, so a provider
//! bug fails driver start instead of producing a resource without policy
//! or a policy without a resource.

use crate::access_policy::{MmioResource, ResourceCeiling, ResourceId};
use crate::digest::{Sha256Digest, combined_digest, per_resource_digests};
use std::collections::BTreeMap;

/// Everything one provider acquired for the bound node.
#[derive(Debug)]
pub struct ProvidedResources<B> {
    /// Provider kind, for example `"platform"` or `"fake"`. Feeds the
    /// per-resource digests.
    pub provider: String,
    /// Stable node identity criteria (for example the node moniker),
    /// never a per-boot numeric ID. Feeds the per-resource digests.
    pub node_identity: String,
    /// Logical resource descriptions, keyed by resource ordinal.
    pub resources: BTreeMap<ResourceId, MmioResource>,
    /// Immutable ceiling entries. A resource with no entry permits
    /// nothing.
    pub ceiling: BTreeMap<ResourceId, ResourceCeiling>,
    /// One backend per offered resource.
    pub backends: BTreeMap<ResourceId, B>,
}

/// Why a provider bundle was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    /// A resource has no backend, or a backend has no resource.
    ResourceBackendMismatch {
        /// Resource ordinals present on exactly one side.
        missing: Vec<ResourceId>,
    },
    /// A ceiling entry names a resource that is not offered. The reverse
    /// (an offered resource without a ceiling entry) is legal and simply
    /// permits nothing.
    CeilingWithoutResource {
        /// The dangling ceiling ordinals.
        missing: Vec<ResourceId>,
    },
    /// A resource's mapped size is smaller than its logical size, so
    /// reads the description advertises could fault.
    MappingSmallerThanLogical {
        /// The offending resource ordinal.
        resource: ResourceId,
    },
}

impl<B> ProvidedResources<B> {
    /// An empty bundle: no resources, nothing reachable.
    pub fn empty(provider: &str, node_identity: &str) -> Self {
        Self {
            provider: provider.to_string(),
            node_identity: node_identity.to_string(),
            resources: BTreeMap::new(),
            ceiling: BTreeMap::new(),
            backends: BTreeMap::new(),
        }
    }

    /// Validates internal consistency. Driver start fails on error.
    pub fn validate(&self) -> Result<(), ProviderError> {
        let resource_ids: Vec<ResourceId> = self.resources.keys().copied().collect();
        let backend_ids: Vec<ResourceId> = self.backends.keys().copied().collect();
        if resource_ids != backend_ids {
            let missing = resource_ids
                .iter()
                .filter(|id| !self.backends.contains_key(id))
                .chain(backend_ids.iter().filter(|id| !self.resources.contains_key(id)))
                .copied()
                .collect();
            return Err(ProviderError::ResourceBackendMismatch { missing });
        }
        let dangling: Vec<ResourceId> =
            self.ceiling.keys().filter(|id| !self.resources.contains_key(id)).copied().collect();
        if !dangling.is_empty() {
            return Err(ProviderError::CeilingWithoutResource { missing: dangling });
        }
        for (id, resource) in &self.resources {
            if resource.mapped_size < resource.logical_size {
                return Err(ProviderError::MappingSmallerThanLogical { resource: *id });
            }
        }
        Ok(())
    }

    /// Per-resource digests for this bundle, in ascending resource ID
    /// order.
    pub fn digests(&self) -> BTreeMap<ResourceId, Sha256Digest> {
        per_resource_digests(&self.node_identity, &self.provider, &self.resources, &self.ceiling)
    }

    /// The whole-instance resource digest for this bundle.
    pub fn combined_digest(&self) -> Sha256Digest {
        combined_digest(self.digests().values().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware_backend::FakeMmio;

    fn resource(logical: u64, mapped: u64) -> MmioResource {
        MmioResource { name: "ctrl".to_string(), logical_size: logical, mapped_size: mapped }
    }

    fn bundle() -> ProvidedResources<FakeMmio> {
        let mut bundle = ProvidedResources::empty("fake", "node");
        bundle.resources.insert(1, resource(0x100, 0x1000));
        bundle.ceiling.insert(
            1,
            ResourceCeiling {
                hard_denied: vec![],
                allow_unknown_reads: true,
                allow_poll: false,
                writable_registers: vec![],
            },
        );
        bundle.backends.insert(1, FakeMmio::new());
        bundle
    }

    #[test]
    fn valid_bundle_passes() {
        assert_eq!(bundle().validate(), Ok(()));
        assert_eq!(ProvidedResources::<FakeMmio>::empty("fake", "node").validate(), Ok(()));
    }

    #[test]
    fn missing_backend_is_rejected() {
        let mut bundle = bundle();
        bundle.backends.clear();
        assert_eq!(
            bundle.validate(),
            Err(ProviderError::ResourceBackendMismatch { missing: vec![1] })
        );
    }

    #[test]
    fn dangling_ceiling_is_rejected() {
        let mut bundle = bundle();
        bundle.ceiling.insert(
            9,
            ResourceCeiling {
                hard_denied: vec![],
                allow_unknown_reads: false,
                allow_poll: false,
                writable_registers: vec![],
            },
        );
        assert_eq!(
            bundle.validate(),
            Err(ProviderError::CeilingWithoutResource { missing: vec![9] })
        );
    }

    #[test]
    fn resource_without_ceiling_is_legal() {
        let mut bundle = bundle();
        bundle.ceiling.clear();
        assert_eq!(bundle.validate(), Ok(()));
    }

    #[test]
    fn short_mapping_is_rejected() {
        let mut bundle = bundle();
        bundle.resources.insert(1, resource(0x1000, 0x100));
        assert_eq!(
            bundle.validate(),
            Err(ProviderError::MappingSmallerThanLogical { resource: 1 })
        );
    }

    #[test]
    fn digests_are_ordered_and_combined() {
        let bundle = bundle();
        let digests = bundle.digests();
        assert_eq!(digests.len(), 1);
        assert_eq!(bundle.combined_digest(), combined_digest(digests.values().copied()));
    }
}
