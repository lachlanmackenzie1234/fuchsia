// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/connectivity/network/mdns/service/services/service_instance_publisher_service_impl.h"

#include <fuchsia/net/mdns/cpp/fidl.h>

#include "src/connectivity/network/mdns/service/encoding/dns_message.h"
#include "src/lib/testing/loop_fixture/real_loop_fixture.h"

namespace mdns {
namespace test {

class ServiceInstancePublisherServiceImplTests
    : public gtest::RealLoopFixture,
      public fuchsia::net::mdns::ServiceInstancePublicationResponder {
 public:
  void OnPublication(fuchsia::net::mdns::ServiceInstancePublicationCause publication_cause,
                     fidl::StringPtr subtype, std::vector<fuchsia::net::IpAddress> source_addresses,
                     OnPublicationCallback callback) override {
    on_publication_request_count_++;
    if (on_publication_handler_) {
      on_publication_handler_();
    }
    callback(fpromise::error(fuchsia::net::mdns::OnPublicationError::DO_NOT_RESPOND));
  }

  size_t on_publication_request_count() {
    auto result = on_publication_request_count_;
    on_publication_request_count_ = 0;
    return result;
  }

  void set_on_publication_handler(fit::closure handler) {
    on_publication_handler_ = std::move(handler);
  }

 private:
  size_t on_publication_request_count_ = 0;
  fit::closure on_publication_handler_;
};

namespace {

class TestTransceiver : public Mdns::Transceiver {
 public:
  // Mdns::Transceiver implementation.
  void SetVerbose(bool verbose) override {}

  void Start(fuchsia::net::interfaces::WatcherPtr watcher, fit::closure link_change_callback,
             InboundMessageCallback inbound_message_callback,
             InterfaceTransceiverCreateFunction transceiver_factory) override {
    inbound_message_callback_ = std::move(inbound_message_callback);
    link_change_callback();
  }

  void Stop() override {}

  bool HasInterfaces() override { return true; }

  void SendMessages(
      std::unordered_map<ReplyAddress, Mdns::DnsMessageBuilder, Mdns::ReplyAddressHash> messages)
      override {}

  void LogTraffic() override {}

  std::vector<HostAddress> LocalHostAddresses() override { return std::vector<HostAddress>(); }

  void InjectQuestion(const DnsName& name, DnsType type) {
    auto message = std::make_unique<DnsMessage>();
    message->questions_.push_back(std::make_shared<DnsQuestion>(name, type));
    inbound_message_callback_(std::move(message), ReplyAddress());
  }

 private:
  InboundMessageCallback inbound_message_callback_;
};

}  // namespace

// Tests that publications outlive publishers.
TEST_F(ServiceInstancePublisherServiceImplTests, PublicationLifetime) {
  // Instantiate |Mdns| so we can register a publisher with it.
  TestTransceiver transceiver;
  Mdns mdns(transceiver);
  bool ready_callback_called = false;
  mdns.Start(nullptr, DnsName("TestHostName"), /* perform probe */ false,
             [&ready_callback_called]() {
               // Ready callback.
               ready_callback_called = true;
             },
             {});

  // Create the publisher bound to the |publisher_ptr| channel.
  fuchsia::net::mdns::ServiceInstancePublisherPtr publisher_ptr;
  bool delete_callback_called = false;
  auto under_test = std::make_unique<ServiceInstancePublisherServiceImpl>(
      mdns, publisher_ptr.NewRequest(), [&delete_callback_called]() {
        // Delete callback.
        delete_callback_called = true;
      });

  // Expect that the |Mdns| instance is ready and the publisher has not requested deletion.
  RunLoopUntilIdle();
  EXPECT_TRUE(ready_callback_called);
  EXPECT_FALSE(delete_callback_called);

  // Instantiate a publisher.
  fuchsia::net::mdns::ServiceInstancePublicationResponderHandle responder_handle;
  fidl::Binding<fuchsia::net::mdns::ServiceInstancePublicationResponder> binding(
      this, responder_handle.NewRequest());
  zx_status_t binding_status = ZX_OK;
  binding.set_error_handler([&binding_status](zx_status_t status) { binding_status = status; });

  // Register the publisher with the |Mdns| instance.
  publisher_ptr->PublishServiceInstance(
      "_testservice._tcp.", "TestInstanceName",
      fuchsia::net::mdns::ServiceInstancePublicationOptions(), std::move(responder_handle),
      [](fuchsia::net::mdns::ServiceInstancePublisher_PublishServiceInstance_Result result) {});

  // Expect the responder binding is fine and the publisher has not requested deletion.
  RunLoopUntilIdle();
  EXPECT_EQ(ZX_OK, binding_status);
  EXPECT_FALSE(delete_callback_called);

  // Close the publisher channel. Expect that the responder binding is fine and the publisher has
  // requested deletion.
  publisher_ptr = nullptr;
  RunLoopUntilIdle();
  EXPECT_EQ(ZX_OK, binding_status);
  EXPECT_TRUE(delete_callback_called);

  // Actually delete the publisher as requested by the delete callback. Expect that the binding is
  // fine.
  under_test = nullptr;
  RunLoopUntilIdle();
  EXPECT_EQ(ZX_OK, binding_status);

  binding.Close(ZX_ERR_PEER_CLOSED);
  RunLoopUntilIdle();
}

// Tests that |OnPublication| responses of |DO_NOT_RESPOND| don't prevent subsequent
// |OnPublication| requests from being sent (regression test).
TEST_F(ServiceInstancePublisherServiceImplTests, DoNotRespond) {
  // Instantiate |Mdns| so we can register a publisher with it.
  TestTransceiver transceiver;
  Mdns mdns(transceiver);
  bool ready_callback_called = false;
  mdns.Start(nullptr, DnsName("TestHostName"), /* perform probe */ false,
             [&ready_callback_called]() {
               // Ready callback.
               ready_callback_called = true;
             },
             {});

  // Create the publisher bound to the |publisher_ptr| channel.
  fuchsia::net::mdns::ServiceInstancePublisherPtr publisher_ptr;
  auto under_test = std::make_unique<ServiceInstancePublisherServiceImpl>(
      mdns, publisher_ptr.NewRequest(), []() {});

  // Expect that the |Mdns| instance is ready.
  RunLoopUntilIdle();
  EXPECT_TRUE(ready_callback_called);

  // Instantiate a publisher.
  fuchsia::net::mdns::ServiceInstancePublicationResponderHandle responder_handle;
  fidl::Binding<fuchsia::net::mdns::ServiceInstancePublicationResponder> binding(
      this, responder_handle.NewRequest());
  zx_status_t binding_status = ZX_OK;
  binding.set_error_handler([&binding_status](zx_status_t status) { binding_status = status; });

  auto options = fuchsia::net::mdns::ServiceInstancePublicationOptions();
  options.set_perform_probe(false);

  // Register the publisher with the |Mdns| instance.
  publisher_ptr->PublishServiceInstance(
      "_testservice._tcp.", "TestInstanceName", std::move(options), std::move(responder_handle),
      [](fuchsia::net::mdns::ServiceInstancePublisher_PublishServiceInstance_Result result) {});

  // Expect the responder binding is fine.
  RunLoopUntilIdle();
  EXPECT_EQ(ZX_OK, binding_status);

  // Expect one |OnPublication| request for the initial announcement. We answer with
  // |DO_NOT_RESPOND|.
  EXPECT_EQ(1u, on_publication_request_count());

  // Ask for two reannouncements.
  binding.events().Reannounce();
  binding.events().Reannounce();

  // Expect the responder binding is fine.
  RunLoopUntilIdle();
  EXPECT_EQ(ZX_OK, binding_status);

  // Expect two more |OnPublication| requests for the reannouncements. Prior to the fix,
  // we were seeing only one request here, because request throttling was not handling
  // |DO_NOT_RESPOND| correctly.
  EXPECT_EQ(2u, on_publication_request_count());

  publisher_ptr = nullptr;
  binding.Close(ZX_ERR_PEER_CLOSED);
  RunLoopUntilIdle();
}

// Tests that closing the responder connection during GetPublication doesn't delete
// ResponderPublisher until GetPublicationNow completes.
TEST_F(ServiceInstancePublisherServiceImplTests, DisconnectionDuringGetPublication) {
  // Instantiate |Mdns| so we can register a publisher with it.
  TestTransceiver transceiver;
  Mdns mdns(transceiver);
  bool ready_callback_called = false;
  mdns.Start(nullptr, DnsName("TestHostName"), /* perform probe */ false,
             [&ready_callback_called]() {
               // Ready callback.
               ready_callback_called = true;
             },
             {});

  // Create the publisher bound to the |publisher_ptr| channel.
  fuchsia::net::mdns::ServiceInstancePublisherPtr publisher_ptr;
  bool delete_callback_called = false;
  auto under_test = std::make_unique<ServiceInstancePublisherServiceImpl>(
      mdns, publisher_ptr.NewRequest(), [&delete_callback_called]() {
        // Delete callback.
        delete_callback_called = true;
      });

  // Expect that the |Mdns| instance is ready.
  RunLoopUntilIdle();
  EXPECT_TRUE(ready_callback_called);

  // Instantiate a responder.
  fuchsia::net::mdns::ServiceInstancePublicationResponderHandle responder_handle;
  fidl::Binding<fuchsia::net::mdns::ServiceInstancePublicationResponder> binding(
      this, responder_handle.NewRequest());

  // Close the responder binding when OnPublication is invoked.
  set_on_publication_handler([&binding]() { binding.Unbind(); });

  auto options = fuchsia::net::mdns::ServiceInstancePublicationOptions();
  options.set_perform_probe(false);

  // Register the publisher with the |Mdns| instance.
  bool callback_called = false;
  publisher_ptr->PublishServiceInstance(
      "_testservice._tcp.", "TestInstanceName", std::move(options), std::move(responder_handle),
      [&callback_called](
          fuchsia::net::mdns::ServiceInstancePublisher_PublishServiceInstance_Result result) {
        callback_called = true;
      });

  RunLoopUntilIdle();

  // The publisher callback should be called (meaning ResponderPublisher was deleted).
  EXPECT_TRUE(callback_called);

  // Clean up.
  set_on_publication_handler(nullptr);
}

// Tests that queries with invalid UTF-8 subtypes are ignored.
TEST_F(ServiceInstancePublisherServiceImplTests, InvalidSubtypeUtf8) {
  // Instantiate |Mdns| so we can register a publisher with it.
  TestTransceiver transceiver;
  Mdns mdns(transceiver);
  bool ready_callback_called = false;
  mdns.Start(nullptr, DnsName("TestHostName"), /* perform probe */ false,
             [&ready_callback_called]() {
               // Ready callback.
               ready_callback_called = true;
             },
             {});

  // Create the publisher bound to the |publisher_ptr| channel.
  fuchsia::net::mdns::ServiceInstancePublisherPtr publisher_ptr;
  auto under_test = std::make_unique<ServiceInstancePublisherServiceImpl>(
      mdns, publisher_ptr.NewRequest(), []() {});

  // Expect that the |Mdns| instance is ready.
  RunLoopUntilIdle();
  EXPECT_TRUE(ready_callback_called);

  // Instantiate a responder.
  fuchsia::net::mdns::ServiceInstancePublicationResponderHandle responder_handle;
  fidl::Binding<fuchsia::net::mdns::ServiceInstancePublicationResponder> binding(
      this, responder_handle.NewRequest());
  zx_status_t binding_status = ZX_OK;
  binding.set_error_handler([&binding_status](zx_status_t status) { binding_status = status; });

  auto options = fuchsia::net::mdns::ServiceInstancePublicationOptions();
  options.set_perform_probe(false);

  // Register the publisher with the |Mdns| instance.
  publisher_ptr->PublishServiceInstance(
      "_testservice._tcp.", "TestInstanceName", std::move(options), std::move(responder_handle),
      [](fuchsia::net::mdns::ServiceInstancePublisher_PublishServiceInstance_Result result) {});

  // Run the loop until the initial announcement is done.
  RunLoopUntilIdle();
  EXPECT_EQ(ZX_OK, binding_status);

  // Discard the initial announcement callback count.
  EXPECT_EQ(1u, on_publication_request_count());

  // Inject a query for an invalid UTF-8 subtype: "\xff" (which is invalid UTF-8).
  // The service name is "_testservice._tcp.", and subtype suffix is "._sub.".
  // So the full question name is: "\xff._sub._testservice._tcp.local."
  DnsName question_name =
      DnsName("\xff").append("_sub").append("_testservice").append("_tcp").append("local");
  transceiver.InjectQuestion(question_name, DnsType::kPtr);

  RunLoopUntilIdle();

  // Expect no OnPublication requests, since the subtype was invalid UTF-8.
  EXPECT_EQ(0u, on_publication_request_count());

  publisher_ptr = nullptr;
  binding.Close(ZX_ERR_PEER_CLOSED);
  RunLoopUntilIdle();
}

}  // namespace test
}  // namespace mdns
