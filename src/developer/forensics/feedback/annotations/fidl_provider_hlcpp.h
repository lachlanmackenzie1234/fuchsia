// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVELOPER_FORENSICS_FEEDBACK_ANNOTATIONS_FIDL_PROVIDER_HLCPP_H_
#define SRC_DEVELOPER_FORENSICS_FEEDBACK_ANNOTATIONS_FIDL_PROVIDER_HLCPP_H_

#include <lib/async/cpp/task.h>
#include <lib/async/dispatcher.h>
#include <lib/fidl/cpp/interface_ptr.h>
#include <lib/fit/function.h>
#include <lib/sys/cpp/service_directory.h>
#include <lib/syslog/cpp/macros.h>
#include <zircon/errors.h>

#include <memory>
#include <optional>

#include "src/developer/forensics/feedback/annotations/provider.h"
#include "src/developer/forensics/feedback/annotations/types.h"
#include "src/developer/forensics/utils/errors.h"
#include "src/lib/backoff/backoff.h"
#include "src/lib/fxl/memory/weak_ptr.h"
#include "src/lib/fxl/strings/substitute.h"

namespace forensics::feedback {

namespace internal {

// Defines how a provider should behave in the event of a connection error with the server.
struct DisconnectResponseHlcpp {
  static DisconnectResponseHlcpp BuildFrom(const zx_status_t status,
                                           const std::string_view interface_name) {
    DisconnectResponseHlcpp response;

    if (status == ZX_ERR_UNAVAILABLE || status == ZX_ERR_NOT_FOUND) {
      return DisconnectResponseHlcpp{
          .log_message = fxl::Substitute("$0 unavailable, will not retry", interface_name),
          .error = Error::kNotAvailableInProduct,
          .should_reconnect = false,
      };
    }

    return DisconnectResponseHlcpp{
        .log_message = fxl::Substitute("Lost connection to $0", interface_name),
        .error = Error::kConnectionError,
        .should_reconnect = true,
    };
  }

  std::string log_message;
  Error error;
  bool should_reconnect;
};

}  // namespace internal

// Static async annotation provider that handles calling a single FIDL method and
// returning the result of the call as Annotations when the method completes.
//
// |Interface| is the FIDL protocol being interacted with.
// |method| is the method being called on |Interface|.
// |Convert| is a function object type for converting the results of |method| to Annotations.
template <typename Interface, auto method, typename Convert>
class StaticSingleHlcppFidlMethodAnnotationProvider : public StaticAsyncAnnotationProvider {
 public:
  StaticSingleHlcppFidlMethodAnnotationProvider(async_dispatcher_t* dispatcher,
                                                std::shared_ptr<sys::ServiceDirectory> services,
                                                std::unique_ptr<backoff::Backoff> backoff);

  void GetOnce(::fit::callback<void(Annotations)> callback) override;

 private:
  void Call();

  async_dispatcher_t* dispatcher_;
  std::shared_ptr<sys::ServiceDirectory> services_;
  std::unique_ptr<backoff::Backoff> backoff_;
  Convert convert_;

  ::fidl::InterfacePtr<Interface> ptr_;
  ::fit::callback<void(Annotations)> callback_;
  fxl::WeakPtrFactory<StaticSingleHlcppFidlMethodAnnotationProvider> ptr_factory_{this};
};

// Dynamic async annotation provider that handles calling a single FIDL method and
// returning the result of the call as Annotations when the method completes.
//
// |Interface| is the FIDL protocol being interacted with.
// |method| is the method being called on |Interface|.
// |Convert| is a function object type for converting the results of |method| to Annotations.
template <typename Interface, auto method, typename Convert>
class DynamicSingleHlcppFidlMethodAnnotationProvider : public DynamicAsyncAnnotationProvider {
 public:
  DynamicSingleHlcppFidlMethodAnnotationProvider(async_dispatcher_t* dispatcher,
                                                 std::shared_ptr<sys::ServiceDirectory> services,
                                                 std::unique_ptr<backoff::Backoff> backoff);

  void Get(::fit::callback<void(Annotations)> callback) override;

 private:
  void CleanupCompleted();

  async_dispatcher_t* dispatcher_;
  std::shared_ptr<sys::ServiceDirectory> services_;
  std::unique_ptr<backoff::Backoff> backoff_;
  Convert convert_;
  std::optional<Error> connection_error_;

  ::fidl::InterfacePtr<Interface> ptr_;
  std::vector<::fit::callback<void(Annotations)>> callbacks_;
  fxl::WeakPtrFactory<DynamicSingleHlcppFidlMethodAnnotationProvider> ptr_factory_{this};
};

template <typename Interface, auto method, typename Convert>
class HangingGetSingleHlcppFidlMethodAnnotationProvider : public CachedAsyncAnnotationProvider {
 public:
  HangingGetSingleHlcppFidlMethodAnnotationProvider(async_dispatcher_t* dispatcher,
                                                    std::shared_ptr<sys::ServiceDirectory> services,
                                                    std::unique_ptr<backoff::Backoff> backoff);

  void GetOnUpdate(::fit::function<void(Annotations)> callback) override;

 private:
  void Call();

  async_dispatcher_t* dispatcher_;
  std::shared_ptr<sys::ServiceDirectory> services_;
  std::unique_ptr<backoff::Backoff> backoff_;
  std::optional<Annotations> last_annotations_;
  Convert convert_;

  ::fidl::InterfacePtr<Interface> ptr_;
  ::fit::function<void(Annotations)> on_update_;
  fxl::WeakPtrFactory<HangingGetSingleHlcppFidlMethodAnnotationProvider> ptr_factory_{this};
};

template <typename Interface, auto method, typename Convert>
StaticSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::
    StaticSingleHlcppFidlMethodAnnotationProvider(async_dispatcher_t* dispatcher,
                                                  std::shared_ptr<sys::ServiceDirectory> services,
                                                  std::unique_ptr<backoff::Backoff> backoff)
    : dispatcher_(dispatcher), services_(std::move(services)), backoff_(std::move(backoff)) {
  ptr_.set_error_handler([this](const zx_status_t status) {
    FX_PLOGS(WARNING, status) << "Lost connection to " << Interface::Name_;
    async::PostDelayedTask(
        dispatcher_,
        [self = ptr_factory_.GetWeakPtr()] {
          if (self) {
            self->Call();
          }
        },
        backoff_->GetNext());
  });
}

template <typename Interface, auto method, typename Convert>
void StaticSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::GetOnce(
    ::fit::callback<void(Annotations)> callback) {
  callback_ = std::move(callback);
  Call();
}

template <typename Interface, auto method, typename Convert>
void StaticSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::Call() {
  if (!ptr_.is_bound()) {
    services_->Connect(ptr_.NewRequest(dispatcher_));
  }

  ((*ptr_).*method)([this](auto&&... result) {
    callback_(convert_(result...));
    ptr_.Unbind();
  });
}

template <typename Interface, auto method, typename Convert>
DynamicSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::
    DynamicSingleHlcppFidlMethodAnnotationProvider(async_dispatcher_t* dispatcher,
                                                   std::shared_ptr<sys::ServiceDirectory> services,
                                                   std::unique_ptr<backoff::Backoff> backoff)
    : dispatcher_(dispatcher),
      services_(std::move(services)),
      backoff_(std::move(backoff)),
      connection_error_(std::nullopt) {
  services_->Connect(ptr_.NewRequest(dispatcher_));

  ptr_.set_error_handler([this](const zx_status_t status) {
    const internal::DisconnectResponseHlcpp disconnect =
        internal::DisconnectResponseHlcpp::BuildFrom(status, Interface::Name_);

    FX_PLOGS(WARNING, status) << disconnect.log_message;
    connection_error_ = disconnect.error;

    // Complete any outstanding callbacks with |error|.
    for (auto& callback : callbacks_) {
      if (callback != nullptr) {
        callback(convert_(*connection_error_));
      }
    }

    CleanupCompleted();

    if (disconnect.should_reconnect) {
      async::PostDelayedTask(
          dispatcher_,
          [self = ptr_factory_.GetWeakPtr()] {
            if (self) {
              self->services_->Connect(self->ptr_.NewRequest(self->dispatcher_));
            }
          },
          backoff_->GetNext());
    }
  });
}

template <typename Interface, auto method, typename Convert>
void DynamicSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::Get(
    ::fit::callback<void(Annotations)> callback) {
  // A reconnection is possibly in progress or the server was unavailable on the product.
  if (!ptr_.is_bound()) {
    FX_CHECK(connection_error_.has_value());
    callback(convert_(*connection_error_));
    return;
  }

  callbacks_.push_back(callback.share());
  ((*ptr_).*method)([this, callback = std::move(callback)](auto&&... result) mutable {
    if (callback != nullptr) {
      callback(convert_(result...));
    }
    CleanupCompleted();
  });
}

template <typename Interface, auto method, typename Convert>
void DynamicSingleHlcppFidlMethodAnnotationProvider<Interface, method,
                                                    Convert>::CleanupCompleted() {
  callbacks_.erase(std::remove_if(callbacks_.begin(), callbacks_.end(),
                                  [](const ::fit::callback<void(Annotations)>& callback) {
                                    return callback == nullptr;
                                  }),
                   callbacks_.end());
}

template <typename Interface, auto method, typename Convert>
HangingGetSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::
    HangingGetSingleHlcppFidlMethodAnnotationProvider(
        async_dispatcher_t* dispatcher, std::shared_ptr<sys::ServiceDirectory> services,
        std::unique_ptr<backoff::Backoff> backoff)
    : dispatcher_(dispatcher), services_(std::move(services)), backoff_(std::move(backoff)) {
  ptr_.set_error_handler([this](const zx_status_t status) {
    FX_PLOGS(WARNING, status) << "Lost connection to " << Interface::Name_;

    async::PostDelayedTask(
        dispatcher_,
        [self = ptr_factory_.GetWeakPtr()] {
          if (self) {
            self->services_->Connect(self->ptr_.NewRequest(self->dispatcher_));
            self->Call();
          }
        },
        backoff_->GetNext());
  });

  services_->Connect(ptr_.NewRequest(dispatcher_));
  Call();
}

template <typename Interface, auto method, typename Convert>
void HangingGetSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::GetOnUpdate(
    ::fit::function<void(Annotations)> callback) {
  FX_CHECK(on_update_ == nullptr) << "GetOnUpdate can only be called once";
  on_update_ = std::move(callback);

  if (last_annotations_.has_value()) {
    on_update_(*last_annotations_);
  }
}

template <typename Interface, auto method, typename Convert>
void HangingGetSingleHlcppFidlMethodAnnotationProvider<Interface, method, Convert>::Call() {
  FX_CHECK(ptr_.is_bound()) << "Attempting to make call to " << Interface::Name_
                            << " while reconnecting";

  ((*ptr_).*method)([this](auto&&... result) mutable {
    last_annotations_ = convert_(result...);
    on_update_(*last_annotations_);

    Call();
  });
}

}  // namespace forensics::feedback

#endif  // SRC_DEVELOPER_FORENSICS_FEEDBACK_ANNOTATIONS_FIDL_PROVIDER_HLCPP_H_
