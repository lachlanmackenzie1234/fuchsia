// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/developer/forensics/testing/stubs/diagnostics_archive.h"

namespace forensics {
namespace stubs {

void DiagnosticsArchive::StreamDiagnostics(StreamDiagnosticsRequest& request,
                                           StreamDiagnosticsCompleter::Sync& completer) {
  batch_iterator_->Bind(std::move(request.result_stream()), dispatcher_);
}

void DiagnosticsArchiveClosesFirstIteratorConnection::StreamDiagnostics(
    StreamDiagnosticsRequest& request, StreamDiagnosticsCompleter::Sync& completer) {
  if (is_first_) {
    request.result_stream().Close(ZX_ERR_PEER_CLOSED);
    is_first_ = false;
    return;
  }

  BatchIterator()->Bind(std::move(request.result_stream()), dispatcher());
}

void DiagnosticsArchiveClosesIteratorConnection::StreamDiagnostics(
    StreamDiagnosticsRequest& request, StreamDiagnosticsCompleter::Sync& completer) {
  request.result_stream().Close(ZX_ERR_PEER_CLOSED);
}

}  // namespace stubs
}  // namespace forensics
