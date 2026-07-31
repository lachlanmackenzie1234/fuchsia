// Copyright 2025 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::digest::Digest;
use crate::{
    GlobalPrincipalIdentifier, InflatedPrincipal, InflatedResource, PrincipalType,
    ResourceReference, ZXName, fplugin_serde,
};
use bstr::ByteSlice;
use core::default::Default;
use fidl_fuchsia_memory_attribution_plugin_common as fplugin;
use fplugin::Vmo;
#[cfg(target_os = "fuchsia")]
use fuchsia_trace::duration;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
/// Consider that two floats are equals if they differ less than [FLOAT_COMPARISON_EPSILON].
const FLOAT_COMPARISON_EPSILON: f64 = 1e-10;

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct ComponentSummaryProfileResult {
    pub kernel: fplugin_serde::KernelStatistics,
    pub principals: Vec<PrincipalSummary>,
    /// Amount, in bytes, of memory that is known but remained unclaimed. Should be equal to zero.
    pub unclaimed: u64,
    #[serde(with = "fplugin_serde::PerformanceImpactMetricsDef")]
    pub performance: fplugin::PerformanceImpactMetrics,
    pub digest: Option<Digest>,
}

/// Summary view of the memory usage on a device.
///
/// This view aggregates the memory usage for each Principal, and, for each Principal, for VMOs
/// sharing the same name or belonging to the same logical group. This is a view appropriate to
/// display to developers who want to understand the memory usage of their Principal.
#[derive(Debug, PartialEq, Serialize)]
pub struct MemorySummary {
    pub principals: Vec<PrincipalSummary>,
    /// Amount, in bytes, of memory that is known but remained unclaimed. Should be equal to zero.
    pub unclaimed: u64,
}

impl MemorySummary {
    pub(crate) fn build(
        principals: &HashMap<GlobalPrincipalIdentifier, InflatedPrincipal>,
        resources: &HashMap<u64, InflatedResource>,
        resource_names: &Vec<ZXName>,
    ) -> MemorySummary {
        #[cfg(target_os = "fuchsia")]
        duration!(crate::CATEGORY_MEMORY_CAPTURE, c"MemorySummary::build");
        let mut output = MemorySummary { principals: Default::default(), unclaimed: 0 };
        for principal in principals.values() {
            output.principals.push(MemorySummary::build_one_principal(
                &principal,
                &principals,
                &resources,
                &resource_names,
            ));
        }

        output.principals.sort_unstable_by_key(|p| -(p.populated_total as i64));

        let mut unclaimed = 0;
        for (_, resource) in resources {
            if resource.claims.is_empty() {
                match &resource.resource.resource_type {
                    fplugin::ResourceType::Job(_) | fplugin::ResourceType::Process(_) => {}
                    fplugin::ResourceType::Vmo(vmo) => {
                        unclaimed += vmo.scaled_populated_bytes.unwrap();
                    }
                    _ => todo!(),
                }
            }
        }
        output.unclaimed = unclaimed;
        output
    }

    fn build_one_principal(
        principal: &InflatedPrincipal,
        principals: &HashMap<GlobalPrincipalIdentifier, InflatedPrincipal>,
        resources: &HashMap<u64, InflatedResource>,
        resource_names: &Vec<ZXName>,
    ) -> PrincipalSummary {
        let mut output = PrincipalSummary {
            name: principal.name().to_owned(),
            id: principal.principal.identifier.0.into(),
            principal_type: match &principal.principal.principal_type {
                PrincipalType::Runnable => "R",
                PrincipalType::Part => "P",
            }
            .to_owned(),
            committed_private: 0,
            committed_scaled: 0.0,
            committed_total: 0,
            populated_private: 0,
            populated_scaled: 0.0,
            populated_total: 0,
            attributor: principal
                .principal
                .parent
                .as_ref()
                .and_then(|p| principals.get(p))
                .map(|p| p.name().to_owned()),
            processes: Vec::new(),
            vmos: HashMap::new(),
        };

        for resource_id in &principal.resources {
            if !resources.contains_key(resource_id) {
                continue;
            }

            let resource = resources.get(resource_id).unwrap();
            let share_count = resource
                .claims
                .iter()
                .map(|c| c.subject)
                .collect::<HashSet<GlobalPrincipalIdentifier>>()
                .len();
            match &resource.resource.resource_type {
                fplugin::ResourceType::Job(_) => todo!(),
                fplugin::ResourceType::Process(_) => {
                    output.processes.push(format!(
                        "{} ({})",
                        resource_names.get(resource.resource.name_index).unwrap().clone(),
                        resource.resource.koid
                    ));
                }
                fplugin::ResourceType::Vmo(vmo_info) => {
                    output.committed_total += vmo_info.total_committed_bytes.unwrap();
                    output.populated_total += vmo_info.total_populated_bytes.unwrap();
                    output.committed_scaled +=
                        vmo_info.scaled_committed_bytes.unwrap() as f64 / share_count as f64;
                    output.populated_scaled +=
                        vmo_info.scaled_populated_bytes.unwrap() as f64 / share_count as f64;
                    if share_count == 1 {
                        output.committed_private += vmo_info.private_committed_bytes.unwrap();
                        output.populated_private += vmo_info.private_populated_bytes.unwrap();
                    }
                    output
                        .vmos
                        .entry(
                            vmo_name_to_digest_zxname(
                                &resource_names.get(resource.resource.name_index).unwrap(),
                            )
                            .clone(),
                        )
                        .or_default()
                        .merge(vmo_info, share_count);
                }
                _ => todo!(),
            }
        }

        for (_source, attribution) in &principal.attribution_claims {
            for resource in &attribution.resources {
                if let ResourceReference::ProcessMapped {
                    process: process_mapped,
                    base: _,
                    len: _,
                    hint_skip_handle_table: _,
                } = resource
                {
                    if let Some(process) = resources.get(&process_mapped) {
                        output.processes.push(format!(
                            "{} ({})",
                            resource_names.get(process.resource.name_index).unwrap().clone(),
                            process.resource.koid
                        ));
                    }
                }
            }
        }

        output.processes.sort();
        output
    }
}

impl Display for MemorySummary {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

/// Summary of a Principal memory usage, and its breakdown per VMO group.
#[derive(Debug, Serialize)]
pub struct PrincipalSummary {
    /// Identifier for the Principal. This number is not meaningful outside of the memory
    /// attribution system.
    pub id: u64,
    /// Display name of the Principal.
    pub name: String,
    /// Type of the Principal.
    pub principal_type: String,
    /// Number of committed private bytes of the Principal.
    pub committed_private: u64,
    /// Number of committed bytes of all VMOs accessible to the Principal, scaled by the number of
    /// Principals that can access them.
    pub committed_scaled: f64,
    /// Total number of committed bytes of all the VMOs accessible to the Principal.
    pub committed_total: u64,
    /// Number of populated private bytes of the Principal.
    pub populated_private: u64,
    /// Number of populated bytes of all VMOs accessible to the Principal, scaled by the number of
    /// Principals that can access them.
    pub populated_scaled: f64,
    /// Total number of populated bytes of all the VMOs accessible to the Principal.
    pub populated_total: u64,
    /// Name of the Principal who gave attribution information for this Principal.
    pub attributor: Option<String>,
    /// List of Zircon processes attributed (even partially) to this Principal.
    pub processes: Vec<String>,
    /// Summary of memory usage for the VMOs accessible to this Principal, grouped by VMO name.
    pub vmos: HashMap<ZXName, VmoSummary>,
}

impl PartialEq for PrincipalSummary {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.principal_type == other.principal_type
            && self.committed_private == other.committed_private
            && (self.committed_scaled - other.committed_scaled).abs() < FLOAT_COMPARISON_EPSILON
            && self.committed_total == other.committed_total
            && self.populated_private == other.populated_private
            && (self.populated_scaled - other.populated_scaled).abs() < FLOAT_COMPARISON_EPSILON
            && self.populated_total == other.populated_total
            && self.attributor == other.attributor
            && self.processes == other.processes
            && self.vmos == other.vmos
    }
}

/// Group of VMOs sharing the same name.
#[derive(Default, Debug, Serialize)]
pub struct VmoSummary {
    /// Number of distinct VMOs under the same name.
    pub count: u64,
    /// Number of committed bytes of this VMO group only accessible by the Principal this group
    /// belongs.
    pub committed_private: u64,
    /// Number of committed bytes of this VMO group, scaled by the number of Principals that can
    /// access them.
    pub committed_scaled: f64,
    /// Total number of committed bytes of this VMO group.
    pub committed_total: u64,
    /// Number of populated bytes of this VMO group only accessible by the Principal this group
    /// belongs.
    pub populated_private: u64,
    /// Number of populated bytes of this VMO group, scaled by the number of Principals that can
    /// access them.
    pub populated_scaled: f64,
    /// Total number of populated bytes of this VMO group.
    pub populated_total: u64,
}

impl VmoSummary {
    fn merge(&mut self, vmo_info: &Vmo, share_count: usize) {
        self.count += 1;
        self.committed_total += vmo_info.total_committed_bytes.unwrap();
        self.populated_total += vmo_info.total_populated_bytes.unwrap();
        self.committed_scaled +=
            vmo_info.scaled_committed_bytes.unwrap() as f64 / share_count as f64;
        self.populated_scaled +=
            vmo_info.scaled_populated_bytes.unwrap() as f64 / share_count as f64;
        if share_count == 1 {
            self.committed_private += vmo_info.private_committed_bytes.unwrap();
            self.populated_private += vmo_info.private_populated_bytes.unwrap();
        }
    }
}

impl PartialEq for VmoSummary {
    fn eq(&self, other: &Self) -> bool {
        self.count == other.count
            && self.committed_private == other.committed_private
            && (self.committed_scaled - other.committed_scaled).abs() < FLOAT_COMPARISON_EPSILON
            && self.committed_total == other.committed_total
            && self.populated_private == other.populated_private
            && (self.populated_scaled - other.populated_scaled).abs() < FLOAT_COMPARISON_EPSILON
            && self.populated_total == other.populated_total
    }
}
const VMO_DIGEST_NAME_MAPPING: [(&str, &str); 15] = [
    ("ld\\.so\\.1-internal-heap|(^stack: msg of.*)", "[process-bootstrap]"),
    ("^blob-[0-9a-f]+$", "[blobs]"),
    ("^inactive-blob-[0-9a-f]+$", "[inactive blobs]"),
    ("^thrd_t:0x.*|initial-thread|pthread_(t|create):0x.*$", "[stacks]"),
    ("^data[0-9]*:.*$", "[data]"),
    ("^bss[0-9]*:.*$", "[bss]"),
    ("^relro:.*$", "[relro]"),
    ("^$", "[unnamed]"),
    ("^scudo:.*$", "[scudo]"),
    ("^.*\\.so.*$", "[bootfs-libraries]"),
    ("^stack_and_tls:.*$", "[bionic-stack]"),
    ("^ext4!.*$", "[ext4]"),
    ("^dalvik-.*$", "[dalvik]"),
    ("^bootfs(:.*)?$", "[bootfs]"),
    ("^restricted_state_vmo:[0-9]*$", "[restricted_state_vmo]"),
];

/// Returns the name of a VMO category when the name match on of the rules.
/// This is used for presentation and aggregation.
pub fn vmo_name_to_digest_name(name: &str) -> &str {
    static RULES: std::sync::LazyLock<Vec<(regex_lite::Regex, &'static str)>> =
        std::sync::LazyLock::new(|| {
            VMO_DIGEST_NAME_MAPPING
                .iter()
                .map(|&(pattern, replacement)| {
                    (regex_lite::Regex::new(pattern).unwrap(), replacement)
                })
                .collect()
        });
    RULES.iter().find(|(regex, _)| regex.is_match(name.trim())).map_or(name, |rule| rule.1)
}

pub fn vmo_name_to_digest_zxname(name: &ZXName) -> &ZXName {
    static RULES: std::sync::LazyLock<Vec<(regex_lite::Regex, ZXName)>> =
        std::sync::LazyLock::new(|| {
            VMO_DIGEST_NAME_MAPPING
                .iter()
                .map(|&(pattern, replacement)| {
                    (
                        regex_lite::Regex::new(pattern).unwrap(),
                        ZXName::try_from_bytes(replacement.as_bytes()).unwrap(),
                    )
                })
                .collect()
        });
    if let Ok(name_str) = name.as_bstr().to_str() {
        RULES.iter().find(|(regex, _)| regex.is_match(name_str)).map_or(name, |rule| &rule.1)
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Claim, ClaimType, GlobalPrincipalIdentifier, InflatedPrincipal, InflatedResource};

    #[test]
    fn rename_zx_test() {
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_zxname(&ZXName::from_string_lossy("ld.so.1-internal-heap")),
            &ZXName::from_string_lossy("[process-bootstrap]"),
        );
    }

    #[test]
    fn rename_zx_test_small_name() {
        // Verify that we can match regular expressions anchored at both ends even when the name is
        // not taking the full size of a [ZXName].
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_zxname(&ZXName::from_string_lossy("blob-1234")),
            &ZXName::from_string_lossy("[blobs]"),
        );
    }

    #[test]
    fn rename_test() {
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_name("ld.so.1-internal-heap"),
            "[process-bootstrap]"
        );
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_name("stack: msg of 123"),
            "[process-bootstrap]"
        );
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("blob-123"), "[blobs]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("blob-15e0da8e"), "[blobs]");
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_name("inactive-blob-123"),
            "[inactive blobs]"
        );
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("thrd_t:0x123"), "[stacks]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("initial-thread"), "[stacks]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("pthread_t:0x123"), "[stacks]");
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_name("pthread_create:0xfa124714"),
            "[stacks]"
        );
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("data456:"), "[data]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("bss456:"), "[bss]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("relro:foobar"), "[relro]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name(""), "[unnamed]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("scudo:primary"), "[scudo]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("libfoo.so.1"), "[bootfs-libraries]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("foobar"), "foobar");
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_name("stack_and_tls:2331"),
            "[bionic-stack]"
        );
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("ext4!foobar"), "[ext4]");
        pretty_assertions::assert_eq!(vmo_name_to_digest_name("dalvik-data1234"), "[dalvik]");
        pretty_assertions::assert_eq!(
            vmo_name_to_digest_name("restricted_state_vmo:119723"),
            "[restricted_state_vmo]"
        );
    }

    fn make_test_principal(id: u64, name: &str) -> InflatedPrincipal {
        InflatedPrincipal::new(
            fplugin::Principal {
                identifier: Some(fplugin::PrincipalIdentifier { id }),
                description: Some(fplugin::Description::Component(name.to_owned())),
                principal_type: Some(fplugin::PrincipalType::Runnable),
                parent: None,
                ..Default::default()
            }
            .into(),
        )
    }

    fn make_test_vmo_resource(
        koid: u64,
        name_index: usize,
        committed: u64,
        populated: u64,
        claims: Vec<(u64, u64)>,
    ) -> InflatedResource {
        let mut res = InflatedResource::new(
            fplugin::Resource {
                koid: Some(koid),
                name_index: Some(name_index as u64),
                resource_type: Some(fplugin::ResourceType::Vmo(fplugin::Vmo {
                    private_committed_bytes: Some(committed),
                    private_populated_bytes: Some(populated),
                    scaled_committed_bytes: Some(committed),
                    scaled_populated_bytes: Some(populated),
                    total_committed_bytes: Some(committed),
                    total_populated_bytes: Some(populated),
                    ..Default::default()
                })),
                ..Default::default()
            }
            .into(),
        );
        for (source, subject) in claims {
            res.claims.insert(Claim {
                source: GlobalPrincipalIdentifier::new_for_test(source),
                subject: GlobalPrincipalIdentifier::new_for_test(subject),
                claim_type: ClaimType::Direct,
            });
        }
        res
    }

    /// What is tested: `MemorySummary::build` sorting of `PrincipalSummary` entries by
    /// `populated_total` in descending order.
    ///
    /// Expectations verified:
    /// - Principals in `summary.principals` are ordered descending by their total populated bytes
    ///   (`1_000_000_000` -> `500_000_000` -> `100_000_000`).
    /// - Verifies that large unsigned byte totals are handled correctly without sign-overflow when
    ///   sorting comparator logic is refactored.
    #[test]
    fn test_memory_summary_build_sorting_and_overflow() {
        let mut principals = HashMap::new();
        let mut p1 = make_test_principal(1, "small_principal");
        p1.resources.insert(101);
        let mut p2 = make_test_principal(2, "large_principal");
        p2.resources.insert(102);
        let mut p3 = make_test_principal(3, "medium_principal");
        p3.resources.insert(103);
        principals.insert(GlobalPrincipalIdentifier::new_for_test(1), p1);
        principals.insert(GlobalPrincipalIdentifier::new_for_test(2), p2);
        principals.insert(GlobalPrincipalIdentifier::new_for_test(3), p3);

        let mut resources = HashMap::new();
        resources
            .insert(101, make_test_vmo_resource(101, 0, 100_000_000, 100_000_000, vec![(1, 1)]));
        resources.insert(
            102,
            make_test_vmo_resource(102, 1, 1_000_000_000, 1_000_000_000, vec![(2, 2)]),
        );
        resources
            .insert(103, make_test_vmo_resource(103, 2, 500_000_000, 500_000_000, vec![(3, 3)]));

        let resource_names = vec![
            ZXName::from_string_lossy("vmo_1"),
            ZXName::from_string_lossy("vmo_2"),
            ZXName::from_string_lossy("vmo_3"),
        ];

        let summary = MemorySummary::build(&principals, &resources, &resource_names);
        assert_eq!(summary.principals.len(), 3);
        assert_eq!(summary.principals[0].name, "large_principal");
        assert_eq!(summary.principals[0].populated_total, 1_000_000_000);
        assert_eq!(summary.principals[1].name, "medium_principal");
        assert_eq!(summary.principals[1].populated_total, 500_000_000);
        assert_eq!(summary.principals[2].name, "small_principal");
        assert_eq!(summary.principals[2].populated_total, 100_000_000);
    }

    /// What is tested: VMO digest aggregation and merging when they have the same name.
    ///
    /// Expectations verified:
    /// - When multiple VMOs owned by a principal have distinct names ("blob-1111", "blob-2222")
    ///   that digest to the same bucket ("[blobs]"), they are merged into a single `VmoSummary`
    ///   entry.
    /// - Verifies `vmo_summary.count == 2` and that all committed/populated byte metrics (total and
    ///   private) are accurately summed across the aggregated VMOs.
    #[test]
    fn test_memory_summary_vmo_digest_aggregation() {
        let mut principals = HashMap::new();
        let mut p1 = make_test_principal(1, "blob_owner");
        p1.resources.insert(1001);
        p1.resources.insert(1002);
        principals.insert(GlobalPrincipalIdentifier::new_for_test(1), p1);

        let mut resources = HashMap::new();
        resources.insert(1001, make_test_vmo_resource(1001, 0, 100, 200, vec![(1, 1)]));
        resources.insert(1002, make_test_vmo_resource(1002, 1, 300, 400, vec![(1, 1)]));

        let resource_names =
            vec![ZXName::from_string_lossy("blob-1111"), ZXName::from_string_lossy("blob-2222")];

        let summary = MemorySummary::build(&principals, &resources, &resource_names);
        assert_eq!(summary.principals.len(), 1);
        let p_summary = &summary.principals[0];
        assert_eq!(p_summary.vmos.len(), 1);

        let blob_digest = ZXName::from_string_lossy("[blobs]");
        let vmo_summary = p_summary.vmos.get(&blob_digest).expect("Should aggregate under [blobs]");
        assert_eq!(vmo_summary.count, 2);
        assert_eq!(vmo_summary.committed_total, 400);
        assert_eq!(vmo_summary.populated_total, 600);
        assert_eq!(vmo_summary.committed_private, 400);
        assert_eq!(vmo_summary.populated_private, 600);
    }

    /// What is tested: Process formatting and alphabetical sorting of process strings in
    /// `PrincipalSummary.processes`.
    ///
    /// Expectations verified:
    /// - Multiple distinct process resources attributed to a principal are formatted as `"name (koid)"`
    ///   and sorted alphabetically (`"alpha_process (2002)"` before `"zeta_process (2001)"`).
    #[test]
    fn test_memory_summary_process_formatting_and_sorting() {
        let mut principals = HashMap::new();
        let mut p1 = make_test_principal(1, "proc_owner");
        p1.resources.insert(2001);
        p1.resources.insert(2002);
        principals.insert(GlobalPrincipalIdentifier::new_for_test(1), p1);

        let mut resources = HashMap::new();
        let r1 = InflatedResource::new(
            fplugin::Resource {
                koid: Some(2001),
                name_index: Some(0),
                resource_type: Some(fplugin::ResourceType::Process(fplugin::Process {
                    vmos: Some(vec![]),
                    mappings: None,
                    ..Default::default()
                })),
                ..Default::default()
            }
            .into(),
        );
        let r2 = InflatedResource::new(
            fplugin::Resource {
                koid: Some(2002),
                name_index: Some(1),
                resource_type: Some(fplugin::ResourceType::Process(fplugin::Process {
                    vmos: Some(vec![]),
                    mappings: None,
                    ..Default::default()
                })),
                ..Default::default()
            }
            .into(),
        );
        resources.insert(2001, r1);
        resources.insert(2002, r2);

        let resource_names = vec![
            ZXName::from_string_lossy("zeta_process"),
            ZXName::from_string_lossy("alpha_process"),
        ];

        let summary = MemorySummary::build(&principals, &resources, &resource_names);
        assert_eq!(summary.principals.len(), 1);
        assert_eq!(
            summary.principals[0].processes,
            vec!["alpha_process (2002)".to_owned(), "zeta_process (2001)".to_owned()]
        );
    }

    /// What is tested: `share_count` division and private vs. scaled memory calculations when a VMO
    /// is shared across multiple principals.
    ///
    /// Expectations verified:
    /// - When a VMO is shared among 2 distinct principals (`share_count == 2`), scaled bytes equal
    ///   `total / 2.0`.
    /// - Because `share_count > 1`, `committed_private` and `populated_private` are exactly 0 for
    ///   both sharing principals.
    #[test]
    fn test_memory_summary_share_count_calculation() {
        let mut principals = HashMap::new();
        let mut p1 = make_test_principal(1, "owner1");
        let mut p2 = make_test_principal(2, "owner2");
        p1.resources.insert(3001);
        p2.resources.insert(3001);
        principals.insert(GlobalPrincipalIdentifier::new_for_test(1), p1);
        principals.insert(GlobalPrincipalIdentifier::new_for_test(2), p2);

        let mut resources = HashMap::new();
        resources.insert(3001, make_test_vmo_resource(3001, 0, 1000, 2000, vec![(1, 1), (2, 2)]));

        let resource_names = vec![ZXName::from_string_lossy("shared_mem")];
        let summary = MemorySummary::build(&principals, &resources, &resource_names);

        assert_eq!(summary.principals.len(), 2);
        for p_sum in &summary.principals {
            assert_eq!(p_sum.committed_total, 1000);
            assert_eq!(p_sum.populated_total, 2000);
            assert_eq!(p_sum.committed_scaled, 500.0);
            assert_eq!(p_sum.populated_scaled, 1000.0);
            assert_eq!(p_sum.committed_private, 0);
            assert_eq!(p_sum.populated_private, 0);
        }
    }

    /// What is tested: Aggregation of unclaimed VMOs (VMO resources with an empty claims list) into
    /// `MemorySummary.unclaimed`.
    ///
    /// Expectations verified:
    /// - A VMO with no attribution claims has its `scaled_populated_bytes` added to `summary.
    ///   unclaimed`.
    #[test]
    fn test_memory_summary_unclaimed_vmos() {
        let principals = HashMap::new();
        let mut resources = HashMap::new();
        resources.insert(4001, make_test_vmo_resource(4001, 0, 500, 1234, vec![]));

        let resource_names = vec![ZXName::from_string_lossy("unclaimed_vmo")];
        let summary = MemorySummary::build(&principals, &resources, &resource_names);
        assert_eq!(summary.unclaimed, 1234);
    }
}
