// Copyright 2023 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "zircon/system/utest/device-enumeration/common.h"

#include <lib/async-loop/cpp/loop.h>
#include <lib/async-loop/default.h>
#include <lib/async/cpp/task.h>
#include <lib/component/incoming/cpp/protocol.h>

#include <iostream>
#include <map>
#include <unordered_map>
#include <unordered_set>

#include "src/lib/fsl/io/device_watcher.h"

namespace device_enumeration {

void WaitForClassDeviceCount(const std::string& path_in_devfs, size_t count) {
  async::Loop loop = async::Loop(&kAsyncLoopConfigNeverAttachToThread);

  async::TaskClosure task([path_in_devfs, &count]() {
    // stdout doesn't show up in test logs.
    std::cerr << "still waiting for " << count << " devices in " << path_in_devfs << '\n';
  });

  ASSERT_OK(task.PostDelayed(loop.dispatcher(), zx::min(1)));

  std::map<std::string, int> devices_found;

  std::unique_ptr watcher = fsl::DeviceWatcher::Create(
      std::string("/dev/") + path_in_devfs,
      [&devices_found, &count, &loop](const fidl::ClientEnd<fuchsia_io::Directory>& dir,
                                      const std::string& name) {
        devices_found.emplace(name, 0);
        if (devices_found.size() == count) {
          loop.Shutdown();
        }
      },
      loop.dispatcher());

  loop.Run();
}

}  // namespace device_enumeration

DeviceEnumerationTest::Requirement DeviceEnumerationTest::AllOf(
    std::span<const char* const> node_monikers) {
  std::vector<Requirement> children;
  for (const char* moniker : node_monikers) {
    children.push_back({.type = Requirement::Type::kNode, .node = moniker, .children = {}});
  }
  return {.type = Requirement::Type::kAllOf, .node = "", .children = std::move(children)};
}

DeviceEnumerationTest::Requirement DeviceEnumerationTest::OneOf(
    std::span<const char* const> node_monikers) {
  std::vector<Requirement> children;
  for (const char* moniker : node_monikers) {
    children.push_back({.type = Requirement::Type::kNode, .node = moniker, .children = {}});
  }
  return {.type = Requirement::Type::kOneOf, .node = "", .children = std::move(children)};
}

DeviceEnumerationTest::Requirement DeviceEnumerationTest::AllOf(std::vector<Requirement> children) {
  return {.type = Requirement::Type::kAllOf, .node = "", .children = std::move(children)};
}

DeviceEnumerationTest::Requirement DeviceEnumerationTest::OneOf(std::vector<Requirement> children) {
  return {.type = Requirement::Type::kOneOf, .node = "", .children = std::move(children)};
}

DeviceEnumerationTest::MatchResult DeviceEnumerationTest::GetMatchedNodes(
    const Requirement& req) const {
  switch (req.type) {
    case Requirement::Type::kNode:
      if (node_info_.contains(req.node)) {
        return fit::ok(std::vector<std::string>{req.node});
      }
      return fit::error("node '" + req.node + "' not found");
    case Requirement::Type::kAllOf: {
      std::vector<std::string> all_matches;
      std::string errors;
      for (const auto& child : req.children) {
        MatchResult child_result = GetMatchedNodes(child);
        if (child_result.is_error()) {
          if (!errors.empty()) {
            errors += ", ";
          }
          errors += child_result.error_value();
        } else if (child_result.is_ok()) {
          all_matches.insert(all_matches.end(), child_result.value().begin(),
                             child_result.value().end());
        }
      }
      if (!errors.empty()) {
        return fit::error("AllOf failed: [" + errors + "]");
      }
      return fit::ok(std::move(all_matches));
    }
    case Requirement::Type::kOneOf: {
      std::string errors;
      for (const auto& child : req.children) {
        MatchResult child_result = GetMatchedNodes(child);
        if (child_result.is_ok()) {
          return child_result;
        }
        if (!errors.empty()) {
          errors += ", ";
        }
        errors += child_result.error_value();
      }
      return fit::error("OneOf failed: [" + errors + "]");
    }
  }
}

void DeviceEnumerationTest::Verify(const Requirement& requirement, bool fail_on_unexpected_nodes) {
  MatchResult result = GetMatchedNodes(requirement);

  if (result.is_error()) {
    std::cerr << "Requirement not satisfied: " << result.error_value() << '\n';
  }

  std::unordered_set<std::string> matched_nodes;
  if (result.is_ok()) {
    matched_nodes.insert(result.value().begin(), result.value().end());
  }

  std::unordered_map<std::string, fuchsia_driver_development::NodeInfo> leftover_nodes;
  for (auto& [moniker, node] : node_info_) {
    if (!matched_nodes.contains(moniker)) {
      leftover_nodes[moniker] = node;
    }
  }

  if (!leftover_nodes.empty()) {
    std::cerr << "Found " << leftover_nodes.size() << " unexpected node(s):\n";
    for (auto& [moniker, node] : leftover_nodes) {
      std::cerr << "     " << moniker << ":\n";
    }
  }

  ASSERT_TRUE(result.is_ok());
  if (fail_on_unexpected_nodes) {
    ASSERT_TRUE(leftover_nodes.empty());
  }
}

void DeviceEnumerationTest::VerifyNodes(std::span<const char* const> node_monikers,
                                        bool fail_on_unexpected_nodes) {
  Verify(AllOf(node_monikers), fail_on_unexpected_nodes);
}

void DeviceEnumerationTest::VerifyOneOf(std::span<const char* const> node_monikers) {
  Verify(OneOf(node_monikers));
}

void DeviceEnumerationTest::RetrieveNodeInfo() {
  // This uses the development API for its convenience over directory traversal. It would be more
  // useful to log paths in devfs for the purposes of this test, but less convenient.
  zx::result driver_development = component::Connect<fuchsia_driver_development::Manager>();
  ASSERT_OK(driver_development.status_value());

  const fidl::Status bootup_result = fidl::WireCall(driver_development.value())->WaitForBootup();
  ASSERT_OK(bootup_result.status());

  {
    auto [client, server] = fidl::Endpoints<fuchsia_driver_development::NodeInfoIterator>::Create();

    const fidl::Status result = fidl::WireCall(driver_development.value())
                                    ->GetNodeInfo({}, std::move(server), /* exact_match= */ true);
    ASSERT_OK(result.status());

    // NB: this uses iostream (rather than printf) because FIDL strings aren't null-terminated.
    std::cout << "BEGIN printing all node monikers:" << '\n';
    while (true) {
      const fidl::WireResult result = fidl::WireCall(client)->GetNext();
      ASSERT_OK(result.status());
      const fidl::WireResponse response = result.value();
      if (response.nodes.empty()) {
        break;
      }
      for (const fuchsia_driver_development::wire::NodeInfo& info : response.nodes) {
        ASSERT_TRUE(info.has_moniker());
        if (info.has_quarantined() && info.quarantined()) {
          std::cerr << info.moniker().get() << " exists but has failed to start successfully."
                    << '\n';
        } else {
          std::cout << info.moniker().get() << '\n';
          node_info_[std::string(info.moniker().get())] = fidl::ToNatural(info);
        }
      }
    }
    std::cout << "END printing all node monikers." << '\n';
  }
}
