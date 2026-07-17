// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/ui/scenic/lib/flatland/flatland_types.h"

#include <lib/syslog/cpp/macros.h>

#include <glm/gtc/epsilon.hpp>

namespace flatland {

namespace {

std::ostream& operator<<(std::ostream& str, glm::ivec2 v) {
  return str << "(" << v.x << ", " << v.y << ")";
}

std::ostream& operator<<(std::ostream& str, fuchsia::ui::composition::Orientation orientation) {
  switch (orientation) {
    case fuchsia::ui::composition::Orientation::CCW_0_DEGREES:
      return str << "CCW_0_DEGREES";
    case fuchsia::ui::composition::Orientation::CCW_90_DEGREES:
      return str << "CCW_90_DEGREES";
    case fuchsia::ui::composition::Orientation::CCW_180_DEGREES:
      return str << "CCW_180_DEGREES";
    case fuchsia::ui::composition::Orientation::CCW_270_DEGREES:
      return str << "CCW_270_DEGREES";
  }
}

}  // namespace

std::ostream& operator<<(std::ostream& str, const flatland::ImageRect& r) {
  return str << "ImageRect[origin:" << r.origin << " extent:" << r.extent
             << " orientation:" << r.orientation << " texel_uvs:[" << r.texel_uvs[0] << ","
             << r.texel_uvs[1] << "," << r.texel_uvs[2] << "," << r.texel_uvs[3] << "]]";
}

bool ImageRect::operator==(const ImageRect& other) const {
  constexpr float kEpsilon = 0.001f;
  return glm::all(glm::epsilonEqual(origin, other.origin, kEpsilon)) &&
         glm::all(glm::epsilonEqual(extent, other.extent, kEpsilon)) &&
         orientation == other.orientation && texel_uvs[0] == other.texel_uvs[0] &&
         texel_uvs[1] == other.texel_uvs[1] && texel_uvs[2] == other.texel_uvs[2] &&
         texel_uvs[3] == other.texel_uvs[3];
}

HitRegion::HitRegion(const types::RectangleF& region,
                     const fuchsia::ui::composition::HitTestInteraction& interaction)
    : region_(std::make_optional(region)), interaction_(interaction) {}

HitRegion::HitRegion(types::RectangleF::ConstructorArgs region,
                     fuchsia::ui::composition::HitTestInteraction interaction)
    : HitRegion(types::RectangleF(region), interaction) {}

HitRegion HitRegion::Infinite(fuchsia::ui::composition::HitTestInteraction interaction) {
  return HitRegion(interaction);
}

bool HitRegion::is_finite() const { return region_.has_value(); }

const types::RectangleF& HitRegion::region() const {
  FX_DCHECK(region_.has_value()) << "region accessor needs a value";
  return region_.value();
}

fuchsia::ui::composition::HitTestInteraction HitRegion::interaction() const { return interaction_; }

HitRegion::HitRegion(fuchsia::ui::composition::HitTestInteraction interaction)
    : interaction_(interaction) {}

std::ostream& operator<<(std::ostream& out, const LayerHandle& h) {
  out << "(L:" << h.GetInstanceId() << ":" << h.GetLayerId() << ")";
  return out;
}

std::ostream& operator<<(std::ostream& str, const flatland::ResolvedLayer& rl) {
  static_assert(2 == std::variant_size_v<decltype(flatland::ResolvedLayer::content)>,
                "operator<< must be updated to support new content types");

  str << "ResolvedLayer[rect:" << rl.rect << " multiply_color:(" << rl.multiply_color[0] << ","
      << rl.multiply_color[1] << "," << rl.multiply_color[2] << "," << rl.multiply_color[3] << ")"
      << " blend_mode:" << rl.blend_mode << " flip:" << static_cast<int>(rl.flip);
  if (std::holds_alternative<flatland::ResolvedLayer::SolidColorContent>(rl.content)) {
    const auto& solid = std::get<flatland::ResolvedLayer::SolidColorContent>(rl.content);
    str << " content:SolidColor(" << solid.color[0] << "," << solid.color[1] << ","
        << solid.color[2] << "," << solid.color[3] << ")";
  } else {
    const auto& image = std::get<flatland::ResolvedLayer::ImageContent>(rl.content);
    str << " content:Image(id:" << image.image_id << " size:" << image.width << "x" << image.height
        << ")";
  }
  str << " topology_index:" << rl.topology_index << "]";
  return str;
}

}  // namespace flatland
