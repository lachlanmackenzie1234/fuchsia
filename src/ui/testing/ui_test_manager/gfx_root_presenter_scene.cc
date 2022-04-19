// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/ui/testing/ui_test_manager/gfx_root_presenter_scene.h"

#include <fuchsia/ui/app/cpp/fidl.h>
#include <fuchsia/ui/policy/cpp/fidl.h>
#include <lib/ui/scenic/cpp/resources.h>
#include <lib/ui/scenic/cpp/session.h>
#include <lib/ui/scenic/cpp/view_ref_pair.h>
#include <lib/ui/scenic/cpp/view_token_pair.h>

#include "sdk/lib/syslog/cpp/macros.h"
#include "src/lib/fsl/handles/object_info.h"

namespace ui_testing {

void GfxRootPresenterScene::Initialize() {
  // Create the test view.
  auto scenic = realm_->Connect<fuchsia::ui::scenic::Scenic>();
  test_view_ = std::make_unique<GfxTestView>(std::move(scenic));

  auto test_view_tokens = scenic::ViewTokenPair::New();

  auto root_presenter = realm_->Connect<fuchsia::ui::policy::Presenter>();
  root_presenter->PresentOrReplaceView(std::move(test_view_tokens.view_holder_token),
                                       /* presentation */ nullptr);

  auto [test_control_ref, test_view_ref] = scenic::ViewRefPair::New();
  test_view_->CreateViewWithViewRef(std::move(test_view_tokens.view_token.value),
                                    std::move(test_control_ref), std::move(test_view_ref));

  auto client_view_provider = realm_->Connect<fuchsia::ui::app::ViewProvider>();
  test_view_->AttachChildView(std::move(client_view_provider));
}

bool GfxRootPresenterScene::ClientViewIsAttached() {
  if (!test_view_)
    return false;

  return test_view_->test_view_attached() && test_view_->child_view_connected();
}

bool GfxRootPresenterScene::ClientViewIsRendering() {
  return test_view_->child_view_is_rendering();
}

std::optional<zx_koid_t> GfxRootPresenterScene::ClientViewRefKoid() {
  const auto& child_view_ref = test_view_->child_view_ref();

  if (!child_view_ref)
    return std::nullopt;

  return fsl::GetKoid(child_view_ref->reference.get());
}

float GfxRootPresenterScene::ClientViewScaleFactor() {
  // The test manager's view won't apply any transforms to the client view, so
  // the two views' scale factors will be identical.
  return test_view_->scale_factor();
}

}  // namespace ui_testing
