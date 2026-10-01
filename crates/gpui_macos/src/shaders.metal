#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct SubtreeVertexOutput {
  float4 position [[position]];
};

vertex SubtreeVertexOutput subtree_vertex(uint vertex_id [[vertex_id]]) {
  float2 uv = float2((vertex_id << 1) & 2, vertex_id & 2);
  return {float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0)};
}

fragment float4 subtree_fragment(SubtreeVertexOutput input [[stage_in]],
                                texture2d<float> source [[texture(0)]]) {
  return source.read(uint2(input.position.xy));
}

float4 hsla_to_rgba(Hsla hsla);
float3 srgb_to_linear(float3 color);
float3 linear_to_srgb(float3 color);
float4 srgb_to_oklab(float4 color);
float4 oklab_to_srgb(float4 color);
float4 to_device_position(float2 unit_vertex, Bounds_ScaledPixels bounds,
                          constant Size_DevicePixels *viewport_size);
float4 to_device_position_transformed(float2 unit_vertex, Bounds_ScaledPixels bounds,
                          TransformationMatrix transformation,
                          constant Size_DevicePixels *input_viewport_size);

float2 to_tile_position(float2 unit_vertex, AtlasTile tile,
                        constant Size_DevicePixels *atlas_size);
float4 distance_from_clip_rect(float2 unit_vertex, Bounds_ScaledPixels bounds,
                               Bounds_ScaledPixels clip_bounds);
float4 distance_from_clip_rect_transformed(float2 unit_vertex, Bounds_ScaledPixels bounds,
                               Bounds_ScaledPixels clip_bounds, TransformationMatrix transformation);
float pick_corner_radius(float2 center_to_point, Corners_ScaledPixels corner_radii);
float quad_sdf(float2 point, Bounds_ScaledPixels bounds,
               Corners_ScaledPixels corner_radii);
float quad_sdf_impl(float2 center_to_point, float corner_radius);
float4 over(float4 below, float4 above);
float radians(float degrees);
float4 fill_color(Background background, float2 position, Bounds_ScaledPixels bounds,
  float4 solid_color, thread const float4 colors[4]);

struct GradientColor {
  float4 solid;
  float4 colors[4];
};
GradientColor prepare_fill_color(Background background);

struct MonochromeSpriteVertexOutput {
  float4 position [[position]];
  float2 tile_position;
  uint sprite_id [[flat]];
  float2 local_position;
  float4 background_solid [[flat]];
  float4 background_color0 [[flat]];
  float4 background_color1 [[flat]];
  float4 background_color2 [[flat]];
  float4 background_color3 [[flat]];
  float4 clip_distance;
};

struct MonochromeSpriteFragmentInput {
  float4 position [[position]];
  float2 tile_position;
  uint sprite_id [[flat]];
  float2 local_position;
  float4 background_solid [[flat]];
  float4 background_color0 [[flat]];
  float4 background_color1 [[flat]];
  float4 background_color2 [[flat]];
  float4 background_color3 [[flat]];
  float4 clip_distance;
};

vertex MonochromeSpriteVertexOutput monochrome_sprite_vertex(
    uint unit_vertex_id [[vertex_id]], uint sprite_id [[instance_id]],
    constant float2 *unit_vertices [[buffer(SpriteInputIndex_Vertices)]],
    constant MonochromeSprite *sprites [[buffer(SpriteInputIndex_Sprites)]],
    constant Size_DevicePixels *viewport_size
    [[buffer(SpriteInputIndex_ViewportSize)]],
    constant Size_DevicePixels *atlas_size
    [[buffer(SpriteInputIndex_AtlasTextureSize)]]) {
  float2 unit_vertex = unit_vertices[unit_vertex_id];
  MonochromeSprite sprite = sprites[sprite_id];
  float4 device_position =
      to_device_position_transformed(unit_vertex, sprite.bounds, sprite.transformation, viewport_size);
  float4 clip_distance = distance_from_clip_rect_transformed(unit_vertex, sprite.bounds,
                                                 sprite.content_mask.bounds, sprite.transformation);
  float2 tile_position = to_tile_position(unit_vertex, sprite.tile, atlas_size);
  float2 local_position =
      unit_vertex * float2(sprite.bounds.size.width, sprite.bounds.size.height) +
      float2(sprite.bounds.origin.x, sprite.bounds.origin.y);
  GradientColor gradient = prepare_fill_color(sprite.background);
  return MonochromeSpriteVertexOutput{
      device_position,
      tile_position,
      sprite_id,
      local_position,
      gradient.solid,
      gradient.colors[0],
      gradient.colors[1],
      gradient.colors[2],
      gradient.colors[3],
      {clip_distance.x, clip_distance.y, clip_distance.z, clip_distance.w}};
}

fragment float4 monochrome_sprite_fragment(
    MonochromeSpriteFragmentInput input [[stage_in]],
    constant MonochromeSprite *sprites [[buffer(SpriteInputIndex_Sprites)]],
    texture2d<float> atlas_texture [[texture(SpriteInputIndex_AtlasTexture)]]) {
  if (any(input.clip_distance < float4(0.0))) {
    return float4(0.0);
  }

  constexpr sampler atlas_texture_sampler(mag_filter::linear,
                                          min_filter::linear);
  float4 sample =
      atlas_texture.sample(atlas_texture_sampler, input.tile_position);
  MonochromeSprite sprite = sprites[input.sprite_id];
  float4 colors[4] = {
      input.background_color0,
      input.background_color1,
      input.background_color2,
      input.background_color3,
  };
  float4 color = fill_color(
      sprite.background,
      input.local_position,
      sprite.background_bounds,
      input.background_solid,
      colors);
  color.a *= sample.r;
  return color;
}

struct PolychromeSpriteVertexOutput {
  float4 position [[position]];
  float2 tile_position;
  uint sprite_id [[flat]];
  float2 local_position;
  float clip_distance [[clip_distance]][4];
};

struct PolychromeSpriteFragmentInput {
  float4 position [[position]];
  float2 tile_position;
  uint sprite_id [[flat]];
  float2 local_position;
};

vertex PolychromeSpriteVertexOutput polychrome_sprite_vertex(
    uint unit_vertex_id [[vertex_id]], uint sprite_id [[instance_id]],
    constant float2 *unit_vertices [[buffer(SpriteInputIndex_Vertices)]],
    constant PolychromeSprite *sprites [[buffer(SpriteInputIndex_Sprites)]],
    constant Size_DevicePixels *viewport_size
    [[buffer(SpriteInputIndex_ViewportSize)]],
    constant Size_DevicePixels *atlas_size
    [[buffer(SpriteInputIndex_AtlasTextureSize)]]) {

  float2 unit_vertex = unit_vertices[unit_vertex_id];
  PolychromeSprite sprite = sprites[sprite_id];
  float4 device_position = to_device_position_transformed(
      unit_vertex, sprite.bounds, sprite.transformation, viewport_size);
  float4 clip_distance = distance_from_clip_rect_transformed(
      unit_vertex, sprite.bounds, sprite.content_mask.bounds, sprite.transformation);
  float2 local_position =
      unit_vertex * float2(sprite.bounds.size.width, sprite.bounds.size.height) +
      float2(sprite.bounds.origin.x, sprite.bounds.origin.y);
  float2 tile_position = to_tile_position(unit_vertex, sprite.tile, atlas_size);
  return PolychromeSpriteVertexOutput{
      device_position,
      tile_position,
      sprite_id,
      local_position,
      {clip_distance.x, clip_distance.y, clip_distance.z, clip_distance.w}};
}

fragment float4 polychrome_sprite_fragment(
    PolychromeSpriteFragmentInput input [[stage_in]],
    constant PolychromeSprite *sprites [[buffer(SpriteInputIndex_Sprites)]],
    texture2d<float> atlas_texture [[texture(SpriteInputIndex_AtlasTexture)]]) {
  PolychromeSprite sprite = sprites[input.sprite_id];
  constexpr sampler atlas_texture_sampler(mag_filter::linear,
                                          min_filter::linear);
  float2 atlas_size = float2(atlas_texture.get_width(), atlas_texture.get_height());
  float2 tile_origin = float2(sprite.tile.bounds.origin.x, sprite.tile.bounds.origin.y);
  float2 tile_size = float2(sprite.tile.bounds.size.width, sprite.tile.bounds.size.height);
  float2 tile_min = (tile_origin + 0.5) / atlas_size;
  float2 tile_max = (tile_origin + tile_size - 0.5) / atlas_size;
  float4 sample =
      atlas_texture.sample(atlas_texture_sampler, clamp(input.tile_position, tile_min, tile_max));
  float distance =
      quad_sdf(input.local_position, sprite.clip_bounds, sprite.corner_radii);

  float4 color = sample;
  if (sprite.grayscale) {
    float grayscale = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
    color.r = grayscale;
    color.g = grayscale;
    color.b = grayscale;
  }
  color.a *= sprite.opacity * saturate(0.5 - distance);
  return color;
}

struct PathSpriteVertexOutput {
  float4 position [[position]];
  float2 texture_coords;
};

vertex PathSpriteVertexOutput path_sprite_vertex(
  uint unit_vertex_id [[vertex_id]],
  uint sprite_id [[instance_id]],
  constant float2 *unit_vertices [[buffer(SpriteInputIndex_Vertices)]],
  constant PathSprite *sprites [[buffer(SpriteInputIndex_Sprites)]],
  constant Size_DevicePixels *viewport_size [[buffer(SpriteInputIndex_ViewportSize)]]
) {
  float2 unit_vertex = unit_vertices[unit_vertex_id];
  PathSprite sprite = sprites[sprite_id];
  // Don't apply content mask because it was already accounted for when
  // rasterizing the path.
  float4 device_position =
      to_device_position(unit_vertex, sprite.bounds, viewport_size);

  float2 screen_position = float2(sprite.bounds.origin.x, sprite.bounds.origin.y) + unit_vertex * float2(sprite.bounds.size.width, sprite.bounds.size.height);
  float2 texture_coords = screen_position / float2(viewport_size->width, viewport_size->height);

  return PathSpriteVertexOutput{
    device_position,
    texture_coords
  };
}

fragment float4 path_sprite_fragment(
  PathSpriteVertexOutput input [[stage_in]],
  texture2d<float> intermediate_texture [[texture(SpriteInputIndex_AtlasTexture)]]
) {
  constexpr sampler intermediate_texture_sampler(mag_filter::linear, min_filter::linear);
  return intermediate_texture.sample(intermediate_texture_sampler, input.texture_coords);
}

struct SurfaceVertexOutput {
  float4 position [[position]];
  float2 texture_position;
  float clip_distance [[clip_distance]][4];
};

struct SurfaceFragmentInput {
  float4 position [[position]];
  float2 texture_position;
};

vertex SurfaceVertexOutput surface_vertex(
    uint unit_vertex_id [[vertex_id]], uint surface_id [[instance_id]],
    constant float2 *unit_vertices [[buffer(SurfaceInputIndex_Vertices)]],
    constant SurfaceBounds *surfaces [[buffer(SurfaceInputIndex_Surfaces)]],
    constant Size_DevicePixels *viewport_size
    [[buffer(SurfaceInputIndex_ViewportSize)]]) {
  float2 unit_vertex = unit_vertices[unit_vertex_id];
  SurfaceBounds surface = surfaces[surface_id];
  float4 device_position =
      to_device_position(unit_vertex, surface.bounds, viewport_size);
  float4 clip_distance = distance_from_clip_rect(unit_vertex, surface.bounds,
                                                 surface.content_mask.bounds);
  float2 uv_origin = float2(surface.uv_origin[0], surface.uv_origin[1]);
  float2 uv_size = float2(surface.uv_size[0], surface.uv_size[1]);
  float2 texture_position = uv_origin + unit_vertex * uv_size;
  return SurfaceVertexOutput{
      device_position,
      texture_position,
      {clip_distance.x, clip_distance.y, clip_distance.z, clip_distance.w}};
}

fragment float4 surface_rgba_fragment(
    SurfaceFragmentInput input [[stage_in]],
    constant SurfaceBounds *surfaces [[buffer(SurfaceInputIndex_Surfaces)]],
    texture2d<float> texture [[texture(SurfaceInputIndex_YTexture)]]) {
  constexpr sampler texture_sampler(mag_filter::linear, min_filter::linear);
  SurfaceBounds surface = surfaces[0];
  float4 color = texture.sample(texture_sampler, input.texture_position);
  float distance = quad_sdf(input.position.xy, surface.clip_bounds,
                            surface.corner_radii);
  color.a *= surface.opacity * saturate(0.5 - distance);
  return color;
}

fragment float4 surface_nv12_fragment(
    SurfaceFragmentInput input [[stage_in]],
    constant SurfaceBounds *surfaces [[buffer(SurfaceInputIndex_Surfaces)]],
    texture2d<float> y_texture [[texture(SurfaceInputIndex_YTexture)]],
    texture2d<float> cb_cr_texture
    [[texture(SurfaceInputIndex_CbCrTexture)]]) {
  constexpr sampler texture_sampler(mag_filter::linear, min_filter::linear);
  SurfaceBounds surface = surfaces[0];
  float4 ycbcr = float4(
      y_texture.sample(texture_sampler, input.texture_position).r,
      cb_cr_texture.sample(texture_sampler, input.texture_position).rg, 1.0);
  float3 rgb = float3(
      dot(float4(surface.color_rows[0][0], surface.color_rows[0][1],
                 surface.color_rows[0][2], surface.color_rows[0][3]), ycbcr),
      dot(float4(surface.color_rows[1][0], surface.color_rows[1][1],
                 surface.color_rows[1][2], surface.color_rows[1][3]), ycbcr),
      dot(float4(surface.color_rows[2][0], surface.color_rows[2][1],
                 surface.color_rows[2][2], surface.color_rows[2][3]), ycbcr));
  float distance = quad_sdf(input.position.xy, surface.clip_bounds,
                            surface.corner_radii);
  return float4(rgb, surface.opacity * saturate(0.5 - distance));
}

float4 hsla_to_rgba(Hsla hsla) {
  float h = hsla.h * 6.0; // Now, it's an angle but scaled in [0, 6) range
  float s = hsla.s;
  float l = hsla.l;
  float a = hsla.a;

  float c = (1.0 - fabs(2.0 * l - 1.0)) * s;
  float x = c * (1.0 - fabs(fmod(h, 2.0) - 1.0));
  float m = l - c / 2.0;

  float r = 0.0;
  float g = 0.0;
  float b = 0.0;

  if (h >= 0.0 && h < 1.0) {
    r = c;
    g = x;
    b = 0.0;
  } else if (h >= 1.0 && h < 2.0) {
    r = x;
    g = c;
    b = 0.0;
  } else if (h >= 2.0 && h < 3.0) {
    r = 0.0;
    g = c;
    b = x;
  } else if (h >= 3.0 && h < 4.0) {
    r = 0.0;
    g = x;
    b = c;
  } else if (h >= 4.0 && h < 5.0) {
    r = x;
    g = 0.0;
    b = c;
  } else {
    r = c;
    g = 0.0;
    b = x;
  }

  float4 rgba;
  rgba.x = (r + m);
  rgba.y = (g + m);
  rgba.z = (b + m);
  rgba.w = a;
  return rgba;
}

float3 srgb_to_linear(float3 color) {
  return pow(color, float3(2.2));
}

float3 linear_to_srgb(float3 color) {
  return pow(color, float3(1.0 / 2.2));
}

// Converts a sRGB color to the Oklab color space.
// Reference: https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab
float4 srgb_to_oklab(float4 color) {
  // Convert non-linear sRGB to linear sRGB
  color = float4(srgb_to_linear(color.rgb), color.a);

  float l = 0.4122214708 * color.r + 0.5363325363 * color.g + 0.0514459929 * color.b;
  float m = 0.2119034982 * color.r + 0.6806995451 * color.g + 0.1073969566 * color.b;
  float s = 0.0883024619 * color.r + 0.2817188376 * color.g + 0.6299787005 * color.b;

  float l_ = pow(l, 1.0/3.0);
  float m_ = pow(m, 1.0/3.0);
  float s_ = pow(s, 1.0/3.0);

  return float4(
   	0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
   	1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
   	0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
   	color.a
  );
}

// Converts an Oklab color to the sRGB color space.
float4 oklab_to_srgb(float4 color) {
  float l_ = color.r + 0.3963377774 * color.g + 0.2158037573 * color.b;
  float m_ = color.r - 0.1055613458 * color.g - 0.0638541728 * color.b;
  float s_ = color.r - 0.0894841775 * color.g - 1.2914855480 * color.b;

  float l = l_ * l_ * l_;
  float m = m_ * m_ * m_;
  float s = s_ * s_ * s_;

  float3 linear_rgb = float3(
   	4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
   	-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
   	-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
  );

  // Convert linear sRGB to non-linear sRGB
  return float4(linear_to_srgb(linear_rgb), color.a);
}

float4 to_device_position(float2 unit_vertex, Bounds_ScaledPixels bounds,
                          constant Size_DevicePixels *input_viewport_size) {
  float2 position =
      unit_vertex * float2(bounds.size.width, bounds.size.height) +
      float2(bounds.origin.x, bounds.origin.y);
  float2 viewport_size = float2((float)input_viewport_size->width,
                                (float)input_viewport_size->height);
  float2 device_position =
      position / viewport_size * float2(2., -2.) + float2(-1., 1.);
  return float4(device_position, 0., 1.);
}

float4 to_device_position_transformed(float2 unit_vertex, Bounds_ScaledPixels bounds,
                          TransformationMatrix transformation,
                          constant Size_DevicePixels *input_viewport_size) {
  float2 position =
      unit_vertex * float2(bounds.size.width, bounds.size.height) +
      float2(bounds.origin.x, bounds.origin.y);

  // Apply the transformation matrix to the position via matrix multiplication.
  float2 transformed_position = float2(0, 0);
  transformed_position[0] = position[0] * transformation.rotation_scale[0][0] + position[1] * transformation.rotation_scale[0][1];
  transformed_position[1] = position[0] * transformation.rotation_scale[1][0] + position[1] * transformation.rotation_scale[1][1];

  // Add in the translation component of the transformation matrix.
  transformed_position[0] += transformation.translation[0];
  transformed_position[1] += transformation.translation[1];

  float2 viewport_size = float2((float)input_viewport_size->width,
                                (float)input_viewport_size->height);
  float2 device_position =
      transformed_position / viewport_size * float2(2., -2.) + float2(-1., 1.);
  return float4(device_position, 0., 1.);
}


float2 to_tile_position(float2 unit_vertex, AtlasTile tile,
                        constant Size_DevicePixels *atlas_size) {
  float2 tile_origin = float2(tile.bounds.origin.x, tile.bounds.origin.y);
  float2 tile_size = float2(tile.bounds.size.width, tile.bounds.size.height);
  return (tile_origin + unit_vertex * tile_size) /
         float2((float)atlas_size->width, (float)atlas_size->height);
}

// Selects corner radius based on quadrant.
float pick_corner_radius(float2 center_to_point, Corners_ScaledPixels corner_radii) {
  if (center_to_point.x < 0.) {
    if (center_to_point.y < 0.) {
      return corner_radii.top_left;
    } else {
      return corner_radii.bottom_left;
    }
  } else {
    if (center_to_point.y < 0.) {
      return corner_radii.top_right;
    } else {
      return corner_radii.bottom_right;
    }
  }
}

// Signed distance of the point to the quad's border - positive outside the
// border, and negative inside.
float quad_sdf(float2 point, Bounds_ScaledPixels bounds,
               Corners_ScaledPixels corner_radii) {
    float2 half_size = float2(bounds.size.width, bounds.size.height) / 2.0;
    float2 center = float2(bounds.origin.x, bounds.origin.y) + half_size;
    float2 center_to_point = point - center;
    float corner_radius = pick_corner_radius(center_to_point, corner_radii);
    float2 corner_to_point = fabs(center_to_point) - half_size;
    float2 corner_center_to_point = corner_to_point + corner_radius;
    return quad_sdf_impl(corner_center_to_point, corner_radius);
}

// Implementation of quad signed distance field
float quad_sdf_impl(float2 corner_center_to_point, float corner_radius) {
    if (corner_radius == 0.0) {
        // Fast path for unrounded corners
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        // Signed distance of the point from a quad that is inset by corner_radius
        // It is negative inside this quad, and positive outside
        float signed_distance_to_inset_quad =
            // 0 inside the inset quad, and positive outside
            length(max(float2(0.0), corner_center_to_point)) +
            // 0 outside the inset quad, and negative inside
            min(0.0, max(corner_center_to_point.x, corner_center_to_point.y));

        return signed_distance_to_inset_quad - corner_radius;
    }
}
float4 distance_from_clip_rect(float2 unit_vertex, Bounds_ScaledPixels bounds,
                               Bounds_ScaledPixels clip_bounds) {
  float2 position =
      unit_vertex * float2(bounds.size.width, bounds.size.height) +
      float2(bounds.origin.x, bounds.origin.y);
  return float4(position.x - clip_bounds.origin.x,
                clip_bounds.origin.x + clip_bounds.size.width - position.x,
                position.y - clip_bounds.origin.y,
                clip_bounds.origin.y + clip_bounds.size.height - position.y);
}

float4 distance_from_clip_rect_transformed(float2 unit_vertex, Bounds_ScaledPixels bounds,
                               Bounds_ScaledPixels clip_bounds, TransformationMatrix transformation) {
  float2 position =
      unit_vertex * float2(bounds.size.width, bounds.size.height) +
      float2(bounds.origin.x, bounds.origin.y);
  float2 transformed_position = float2(0, 0);
  transformed_position[0] = position[0] * transformation.rotation_scale[0][0] + position[1] * transformation.rotation_scale[0][1];
  transformed_position[1] = position[0] * transformation.rotation_scale[1][0] + position[1] * transformation.rotation_scale[1][1];
  transformed_position[0] += transformation.translation[0];
  transformed_position[1] += transformation.translation[1];

  return float4(transformed_position.x - clip_bounds.origin.x,
                clip_bounds.origin.x + clip_bounds.size.width - transformed_position.x,
                transformed_position.y - clip_bounds.origin.y,
                clip_bounds.origin.y + clip_bounds.size.height - transformed_position.y);
}

float4 over(float4 below, float4 above) {
  float4 result;
  float alpha = above.a + below.a * (1.0 - above.a);
  result.rgb =
      (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / alpha;
  result.a = alpha;
  return result;
}

GradientColor prepare_fill_color(Background background) {
  GradientColor out = {};
  if (background.tag == 0 || background.tag == 2 || background.tag == 3) {
    out.solid = hsla_to_rgba(background.solid);
  } else if (background.tag == 1 || (background.tag >= 4 && background.tag <= 6)) {
    for (uint ix = 0; ix < 4; ix++) {
      float4 color = hsla_to_rgba(background.colors[ix].color);
      out.colors[ix] = background.color_space == 1
        ? srgb_to_oklab(color)
        : color;
    }
  }

  return out;
}

float4 sample_gradient_stops(
    Background background,
    float position,
    thread const float4 colors[4]) {
  uint count = max(background.stop_count, 2u);
  float sample_position = clamp(position, 0.0, 1.0);
  uint left_ix = 0;
  uint right_ix = 0;
  float left_position = background.colors[0].percentage;
  float right_position = left_position;

  if (background.gradient_repeating != 0) {
    sample_position = fract(position + background.gradient_phase);
    left_ix = count - 1;
    right_ix = 0;
    left_position = background.colors[left_ix].percentage;
    right_position = background.colors[0].percentage + 1.0;
    for (uint ix = 0; ix + 1 < count; ix++) {
      if (sample_position >= background.colors[ix].percentage
          && sample_position < background.colors[ix + 1].percentage) {
        left_ix = ix;
        right_ix = ix + 1;
        left_position = background.colors[left_ix].percentage;
        right_position = background.colors[right_ix].percentage;
      }
    }
    if (right_ix == 0 && sample_position < background.colors[0].percentage) {
      sample_position += 1.0;
    }
  } else if (sample_position <= background.colors[0].percentage) {
    left_ix = 0;
    right_ix = 0;
  } else if (sample_position >= background.colors[count - 1].percentage) {
    left_ix = count - 1;
    right_ix = count - 1;
    left_position = background.colors[left_ix].percentage;
    right_position = left_position;
  } else {
    for (uint ix = 0; ix + 1 < count; ix++) {
      if (sample_position >= background.colors[ix].percentage
          && sample_position < background.colors[ix + 1].percentage) {
        left_ix = ix;
        right_ix = ix + 1;
        left_position = background.colors[left_ix].percentage;
        right_position = background.colors[right_ix].percentage;
      }
    }
  }

  float segment = max(right_position - left_position, 0.000001);
  float t = clamp((sample_position - left_position) / segment, 0.0, 1.0);
  // Match CSS color-hint interpolation and the WGSL backend.
  float midpoint = background.gradient_midpoints[left_ix];
  float weight = t;
  if (midpoint != 0.5 && t > 0.0 && t < 1.0) {
    weight = pow(t, log(0.5) / log(midpoint));
  }
  float4 color = mix(colors[left_ix], colors[right_ix], weight);
  if (background.gradient_repeating != 0) {
    uint before_ix = left_ix > 0 ? left_ix - 1 : count - 1;
    uint after_ix = right_ix + 1 < count ? right_ix + 1 : 0;

    // Treat the stops as periodic cubic B-spline control points. Four
    // neighboring colors influence every sample, avoiding visible bands
    // centered on exact stop colors while keeping the curve C2-continuous.
    float t2 = t * t;
    float t3 = t2 * t;
    float one_minus_t = 1.0 - t;
    float weight0 = one_minus_t * one_minus_t * one_minus_t / 6.0;
    float weight1 = (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0;
    float weight2 = (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0;
    float weight3 = t3 / 6.0;
    color = weight0 * colors[before_ix]
      + weight1 * colors[left_ix]
      + weight2 * colors[right_ix]
      + weight3 * colors[after_ix];
  }
  return color;
}

float4 sample_linear_gradient(Background background, float position,
    thread const float4 colors[4]) {
  float4 color = sample_gradient_stops(background, position, colors);
  float width = background.angular_seam_width;
  float half_width = width * 0.5;
  if (background.tag == 5 && background.gradient_repeating == 0 && width > 0.0
      && (position < half_width || position > 1.0 - half_width)) {
    float4 seam_start = sample_gradient_stops(background, 1.0 - half_width, colors);
    float4 seam_end = sample_gradient_stops(background, half_width, colors);
    float wrapped = position < half_width ? position : position - 1.0;
    color = mix(seam_start, seam_end, smoothstep(-half_width, half_width, wrapped));
  }
  return background.color_space == 1 ? oklab_to_srgb(color) : color;
}

float2x2 rotate2d(float angle) {
    float s = sin(angle);
    float c = cos(angle);
    return float2x2(c, -s, s, c);
}

float4 fill_color(Background background,
                      float2 position,
                      Bounds_ScaledPixels bounds,
                      float4 solid_color, thread const float4 colors[4]) {
  float4 color;

  switch (background.tag) {
    case 0:
      color = solid_color;
      break;
    case 1: case 4: case 5: case 6: {
      // -90 degrees to match the CSS gradient angle.
      float gradient_angle = background.gradient_angle_or_pattern_height;
      float radians = (fmod(gradient_angle, 360.0) - 90.0) * (M_PI_F / 180.0);
      float2 direction = float2(cos(radians), sin(radians));

      // Expand the short side to be the same as the long side
      if (bounds.size.width > bounds.size.height) {
          direction.y *= bounds.size.height / bounds.size.width;
      } else {
          direction.x *=  bounds.size.width / bounds.size.height;
      }

      // Get the t value for the linear gradient with the color stop percentages.
      float2 half_size = float2(bounds.size.width, bounds.size.height) / 2.;
      float2 center = float2(bounds.origin.x, bounds.origin.y) + half_size;
      float2 center_to_point = position - center;
      float t = dot(center_to_point, direction) / length(direction);
      // Check the direction to determine whether to use x or y
      if (abs(direction.x) > abs(direction.y)) {
          t = (t + half_size.x) / bounds.size.width;
      } else {
          t = (t + half_size.y) / bounds.size.height;
      }

      if (background.tag != 1) {
          float2 q = center_to_point / max(half_size, float2(0.0001));
          float c = cos(radians), s = sin(radians);
          float2 r = float2(q.x * c + q.y * s, -q.x * s + q.y * c);
          if (background.tag == 4) t = length(q);
          else if (background.tag == 5) t = fract((atan2(q.y, q.x) - radians) / (2.0 * M_PI_F) + 1.0);
          else t = abs(r.x) + abs(r.y);
      }
      color = sample_linear_gradient(background, t, colors);

      // Dither to reduce banding in gradients (especially dark/alpha).
      // Triangular-distributed noise breaks up 8-bit quantization steps.
      // ±2/255 for RGB (enough for dark-on-dark compositing),
      // ±3/255 for alpha (needs more because alpha × dark color = tiny steps).
      {
        float2 seed = position * 0.6180339887; // golden ratio spread
        float r1 = fract(sin(dot(seed, float2(12.9898, 78.233))) * 43758.5453);
        float r2 = fract(sin(dot(seed, float2(39.3460, 11.135))) * 24634.6345);
        float tri = r1 + r2 - 1.0; // triangular PDF, range [-1, +1]
        color.rgb += tri * 2.0 / 255.0;
        color.a   += tri * 3.0 / 255.0;
      }

      break;
    }
    case 2: {
        float gradient_angle_or_pattern_height = background.gradient_angle_or_pattern_height;
        float pattern_width = (gradient_angle_or_pattern_height / 65535.0f) / 255.0f;
        float pattern_interval = fmod(gradient_angle_or_pattern_height, 65535.0f) / 255.0f;
        float pattern_height = pattern_width + pattern_interval;
        float stripe_angle = M_PI_F / 4.0;
        float pattern_period = pattern_height * sin(stripe_angle);
        float2x2 rotation = rotate2d(stripe_angle);
        float2 relative_position = position - float2(bounds.origin.x, bounds.origin.y);
        float2 rotated_point = rotation * relative_position;
        float pattern = fmod(rotated_point.x, pattern_period);
        float distance = min(pattern, pattern_period - pattern) - pattern_period * (pattern_width / pattern_height) /  2.0f;
        color = solid_color;
        color.a *= saturate(0.5 - distance);
        break;
    }
    case 3: {
        // checkerboard
        float size = background.gradient_angle_or_pattern_height;
        float2 relative_position = position - float2(bounds.origin.x, bounds.origin.y);

        float x_index = floor(relative_position.x / size);
        float y_index = floor(relative_position.y / size);
        float should_be_colored = fmod(x_index + y_index, 2.0);

        color = solid_color;
        color.a *= saturate(should_be_colored);
        break;
    }
  }

  return color;
}
