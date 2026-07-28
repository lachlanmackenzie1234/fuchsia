// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVELOPER_FORENSICS_FEEDBACK_ATTACHMENTS_TYPES_H_
#define SRC_DEVELOPER_FORENSICS_FEEDBACK_ATTACHMENTS_TYPES_H_

#include <lib/syslog/cpp/macros.h>
#include <lib/zx/time.h>

#include <map>
#include <memory>
#include <set>
#include <string>
#include <string_view>

#include "src/developer/forensics/utils/errors.h"

namespace forensics::feedback {

using AttachmentKey = std::string;
using AttachmentKeys = std::set<AttachmentKey>;

enum class AttachmentState {
  kComplete,
  kPartial,
  kMissing,
};

class AttachmentData {
 public:
  explicit AttachmentData(std::string value)
      : state_(AttachmentState::kComplete),
        value_(std::make_unique<std::string>(std::move(value))),
        error_(std::nullopt) {}
  AttachmentData(std::string value, enum Error error)
      : state_(AttachmentState::kPartial),
        value_(std::make_unique<std::string>(std::move(value))),
        error_(error) {}
  explicit AttachmentData(enum Error error)
      : state_(AttachmentState::kMissing), value_(nullptr), error_(error) {}

  bool HasValue() const { return value_ != nullptr; }

  std::string_view Value() const {
    FX_CHECK(HasValue());
    return *value_;
  }

  bool HasError() const { return error_.has_value(); }

  enum Error Error() const {
    FX_CHECK(HasError());
    return error_.value();
  }

  AttachmentState State() const { return state_; }

  AttachmentData Clone() const {
    if (HasValue() && HasError()) {
      return AttachmentData(*value_, *error_);
    }

    if (HasValue()) {
      return AttachmentData(*value_);
    }

    return AttachmentData(*error_);
  }

 private:
  AttachmentState state_;
  std::unique_ptr<std::string> value_;
  std::optional<enum Error> error_;
};

class AttachmentValue {
 public:
  AttachmentValue(AttachmentData data, zx::duration collection_duration)
      : data_(std::move(data)), collection_duration_(collection_duration) {}
  AttachmentValue(std::string value, zx::duration collection_duration)
      : data_(std::move(value)), collection_duration_(collection_duration) {}
  AttachmentValue(std::string value, enum Error error, zx::duration collection_duration)
      : data_(std::move(value), error), collection_duration_(collection_duration) {}
  AttachmentValue(enum Error error, zx::duration collection_duration)
      : data_(error), collection_duration_(collection_duration) {}

  bool HasValue() const { return data_.HasValue(); }

  std::string_view Value() const { return data_.Value(); }

  bool HasError() const { return data_.HasError(); }

  enum Error Error() const { return data_.Error(); }

  AttachmentState State() const { return data_.State(); }

  zx::duration CollectionDuration() const { return collection_duration_; }

  // Allow callers to explicitly copy an attachment.
  AttachmentValue Clone() const { return AttachmentValue(data_.Clone(), collection_duration_); }

 private:
  AttachmentData data_;
  zx::duration collection_duration_;
};

using Attachment = std::pair<AttachmentKey, AttachmentValue>;
using Attachments = std::map<AttachmentKey, AttachmentValue>;

}  // namespace forensics::feedback

#endif  // SRC_DEVELOPER_FORENSICS_FEEDBACK_ATTACHMENTS_TYPES_H_
