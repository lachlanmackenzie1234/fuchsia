// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fidl/fuchsia.component.decl/cpp/fidl.h>
#include <fidl/fuchsia.driver.framework/cpp/fidl.h>
#include <fidl/fuchsia.offers.test/cpp/fidl.h>
#include <lib/driver2/logger.h>
#include <lib/driver2/namespace.h>
#include <lib/driver2/node_add_args.h>
#include <lib/driver2/outgoing_directory.h>
#include <lib/driver2/promise.h>
#include <lib/driver2/record_cpp.h>
#include <lib/fpromise/bridge.h>
#include <lib/fpromise/scope.h>
#include <lib/sys/component/cpp/outgoing_directory.h>

#include <bind/fuchsia/test/cpp/bind.h>

namespace fdf {
using namespace fuchsia_driver_framework;
}  // namespace fdf

namespace fcd = fuchsia_component_decl;
namespace fio = fuchsia_io;
namespace ft = fuchsia_offers_test;

using fpromise::error;
using fpromise::ok;
using fpromise::promise;
using fpromise::result;

namespace {

const std::string_view kChildName = "leaf";

class RootDriver : public fidl::Server<ft::Handshake> {
 public:
  RootDriver(fdf::UnownedDispatcher dispatcher, fidl::WireSharedClient<fdf::Node> node,
             driver::Namespace ns, driver::Logger logger)
      : dispatcher_(dispatcher->async_dispatcher()),
        outgoing_(driver::OutgoingDirectory::Create(dispatcher->get())),
        node_(std::move(node)),
        ns_(std::move(ns)),
        logger_(std::move(logger)) {}

  static constexpr const char* Name() { return "root"; }

  static zx::status<std::unique_ptr<RootDriver>> Start(fdf::wire::DriverStartArgs& start_args,
                                                       fdf::UnownedDispatcher dispatcher,
                                                       fidl::WireSharedClient<fdf::Node> node,
                                                       driver::Namespace ns,
                                                       driver::Logger logger) {
    auto driver = std::make_unique<RootDriver>(std::move(dispatcher), std::move(node),
                                               std::move(ns), std::move(logger));
    auto result = driver->Run(std::move(start_args.outgoing_dir()));
    if (result.is_error()) {
      return result.take_error();
    }
    return zx::ok(std::move(driver));
  }

 private:
  zx::status<> Run(fidl::ServerEnd<fio::Directory> outgoing_dir) {
    // Setup the outgoing directory.
    {
      driver::ServiceInstanceHandler handler;
      ft::Service::Handler service(&handler);

      auto device = [this](fidl::ServerEnd<ft::Handshake> server_end) mutable -> void {
        fidl::BindServer<fidl::Server<ft::Handshake>>(dispatcher_, std::move(server_end), this);
      };
      zx::status<> status = service.add_device(std::move(device));
      if (status.is_error()) {
        FDF_LOG(ERROR, "Failed to add device %s", status.status_string());
      }
      status = outgoing_.AddService<ft::Service>(std::move(handler), kChildName);
      if (status.is_error()) {
        FDF_LOG(ERROR, "Failed to add service %s", status.status_string());
      }
    }
    auto serve = outgoing_.Serve(std::move(outgoing_dir));
    if (serve.is_error()) {
      return serve.take_error();
    }

    // Start the driver.
    auto result = AddChild();
    if (result.is_error()) {
      return zx::error(ZX_ERR_INTERNAL);
    }

    return zx::ok();
  }

  fitx::result<fdf::wire::NodeError> AddChild() {
    fidl::Arena arena;

    // Set the offers of the node.
    auto service = fcd::OfferService{{
        .source_name = ft::Service::Name,
        .target_name = ft::Service::Name,
    }};

    auto mapping = fcd::NameMapping{{
        .source_name = std::string(kChildName),
        .target_name = "default",
    }};
    service.renamed_instances() = std::vector{std::move(mapping)};

    auto instance_filter = std::string("default");
    service.source_instance_filter() = std::vector{std::move(instance_filter)};

    auto offer = fcd::Offer::WithService(service);

    // Set the properties of the node that a driver will bind to.
    fdf::NodeProperty property =
        driver::MakeProperty(1 /* BIND_PROTOCOL */, bind_fuchsia_test::BIND_PROTOCOL_DEVICE);

    auto args = fdf::NodeAddArgs{{
        .name = std::string(kChildName),
        .offers = std::vector{std::move(offer)},
        .properties = std::vector{std::move(property)},
    }};

    // Create endpoints of the `NodeController` for the node.
    auto endpoints = fidl::CreateEndpoints<fdf::NodeController>();
    if (endpoints.is_error()) {
      return fitx::error(fdf::wire::NodeError::kInternal);
    }

    auto add_result =
        node_.sync()->AddChild(fidl::ToWire(arena, args), std::move(endpoints->server), {});
    if (!add_result.ok()) {
      return fitx::error(fdf::NodeError::kInternal);
    }
    if (add_result->is_error()) {
      return fitx::error(add_result->error_value());
    }
    controller_.Bind(std::move(endpoints->client), dispatcher_);
    return fitx::ok();
  }

  result<> UnbindNode(const fdf::wire::NodeError& error) {
    FDF_LOG(ERROR, "Failed to start root driver: %d", error);
    node_.AsyncTeardown();
    return ok();
  }

  // fidl::Server<ft::Handshake>
  void Do(DoRequest& request, DoCompleter::Sync& completer) override { completer.Reply(); }

  async_dispatcher_t* const dispatcher_;
  driver::OutgoingDirectory outgoing_;

  fidl::WireSharedClient<fdf::Node> node_;
  fidl::WireSharedClient<fdf::NodeController> controller_;
  driver::Namespace ns_;
  driver::Logger logger_;

  // NOTE: Must be the last member.
  fpromise::scope scope_;
};

}  // namespace

FUCHSIA_DRIVER_RECORD_CPP_V1(RootDriver);
