#include "alpha_correction.hlsl"

cbuffer GlobalParams: register(b0) {
    float4 gamma_ratios;
    float2 global_viewport_size;
    float grayscale_enhanced_contrast;
    float subpixel_enhanced_contrast;
    uint is_bgr;
    uint3 global_pad;
};

Texture2D<float4> t_sprite: register(t0);
SamplerState s_sprite: register(s0);

struct SubpixelSpriteFragmentOutput {
    float4 foreground : SV_Target0;
    float4 alpha : SV_Target1;
};

struct Bounds {
    float2 origin;
    float2 size;
};

struct Corners {
    float top_left;
    float top_right;
    float bottom_right;
    float bottom_left;
};

struct Edges {
    float top;
    float right;
    float bottom;
    float left;
};

struct Hsla {
    float h;
    float s;
    float l;
    float a;
};

struct BorderColors {
    Hsla top;
    Hsla right;
    Hsla bottom;
    Hsla left;
};

struct BorderColorStop {
    Hsla color;
    float position;
};

struct BorderGradient {
    BorderColorStop stops[4];
    uint stop_count;
    uint color_space;
    float phase;
    uint pad;
};

struct LinearColorStop {
    Hsla color;
    float percentage;
};

struct Background {
    // 0u is Solid
    // 1u is LinearGradient
    // 2u is PatternSlash
    uint tag;
    // 0u is sRGB linear color
    // 1u is Oklab color
    uint color_space;
    Hsla solid;
    float gradient_angle_or_pattern_height;
    LinearColorStop colors[4];
    uint stop_count;
    float gradient_phase;
    uint gradient_repeating;
    float gradient_midpoints[4];
    float angular_seam_width;
    uint pad;
};

struct GradientColor {
  float4 solid;
  float4 colors[4];
};

struct AtlasTextureId {
    uint index;
    uint kind;
};

struct AtlasBounds {
    int2 origin;
    int2 size;
};

struct AtlasTile {
    AtlasTextureId texture_id;
    uint tile_id;
    uint padding;
    AtlasBounds bounds;
};

struct TransformationMatrix {
    float2x2 rotation_scale;
    float2 translation;
};

static const float M_PI_F = 3.141592653f;
static const float3 GRAYSCALE_FACTORS = float3(0.2126f, 0.7152f, 0.0722f);

float4 to_device_position_impl(float2 position) {
    float2 device_position = position / global_viewport_size * float2(2.0, -2.0) + float2(-1.0, 1.0);
    return float4(device_position, 0., 1.);
}

float4 to_device_position(float2 unit_vertex, Bounds bounds) {
    float2 position = unit_vertex * bounds.size + bounds.origin;
    return to_device_position_impl(position);
}

float4 distance_from_clip_rect_impl(float2 position, Bounds clip_bounds) {
    float2 tl = position - clip_bounds.origin;
    float2 br = clip_bounds.origin + clip_bounds.size - position;
    return float4(tl.x, br.x, tl.y, br.y);
}

float4 distance_from_clip_rect(float2 unit_vertex, Bounds bounds, Bounds clip_bounds) {
    float2 position = unit_vertex * bounds.size + bounds.origin;
    return distance_from_clip_rect_impl(position, clip_bounds);
}

float4 distance_from_clip_rect_transformed(float2 unit_vertex, Bounds bounds, Bounds clip_bounds, TransformationMatrix transformation) {
    float2 position = unit_vertex * bounds.size + bounds.origin;
    float2 transformed = mul(position, transformation.rotation_scale) + transformation.translation;
    return distance_from_clip_rect_impl(transformed, clip_bounds);
}

// Convert linear RGB to sRGB
float3 linear_to_srgb(float3 color) {
    return pow(color, float3(2.2, 2.2, 2.2));
}

// Convert sRGB to linear RGB
float3 srgb_to_linear(float3 color) {
    return pow(color, float3(1.0 / 2.2, 1.0 / 2.2, 1.0 / 2.2));
}

/// Hsla to linear RGBA conversion.
float4 hsla_to_rgba(Hsla hsla) {
    float h = hsla.h * 6.0; // Now, it's an angle but scaled in [0, 6) range
    float s = hsla.s;
    float l = hsla.l;
    float a = hsla.a;

    float c = (1.0 - abs(2.0 * l - 1.0)) * s;
    float x = c * (1.0 - abs(fmod(h, 2.0) - 1.0));
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
float4 over(float4 below, float4 above) {
    float4 result;
    float alpha = above.a + below.a * (1.0 - above.a);
    result.rgb = (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / alpha;
    result.a = alpha;
    return result;
}

float2 to_tile_position(float2 unit_vertex, AtlasTile tile) {
    float2 atlas_size;
    t_sprite.GetDimensions(atlas_size.x, atlas_size.y);
    return (float2(tile.bounds.origin) + unit_vertex * float2(tile.bounds.size)) / atlas_size;
}

// Selects corner radius based on quadrant.
float pick_corner_radius(float2 center_to_point, Corners corner_radii) {
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

float4 to_device_position_transformed(float2 unit_vertex, Bounds bounds,
                                      TransformationMatrix transformation) {
    float2 position = unit_vertex * bounds.size + bounds.origin;
    float2 transformed = mul(position, transformation.rotation_scale) + transformation.translation;
    float2 device_position = transformed / global_viewport_size * float2(2.0, -2.0) + float2(-1.0, 1.0);
    return float4(device_position, 0.0, 1.0);
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
            length(max(float2(0.0, 0.0), corner_center_to_point)) +
            // 0 outside the inset quad, and negative inside
            min(0.0, max(corner_center_to_point.x, corner_center_to_point.y));

        return signed_distance_to_inset_quad - corner_radius;
    }
}

float quad_sdf(float2 pt, Bounds bounds, Corners corner_radii) {
    float2 half_size = bounds.size / 2.;
    float2 center = bounds.origin + half_size;
    float2 center_to_point = pt - center;
    float corner_radius = pick_corner_radius(center_to_point, corner_radii);
    float2 corner_to_point = abs(center_to_point) - half_size;
    float2 corner_center_to_point = corner_to_point + corner_radius;
    return quad_sdf_impl(corner_center_to_point, corner_radius);
}

GradientColor prepare_gradient_color(uint tag, uint color_space, Hsla solid, LinearColorStop colors[4]) {
    GradientColor output = (GradientColor)0;
    if (tag == 0 || tag == 2 || tag == 3) {
        output.solid = hsla_to_rgba(solid);
    } else if (tag == 1 || (tag >= 4 && tag <= 6)) {
        for (uint ix = 0; ix < 4; ix++) {
            float4 color = hsla_to_rgba(colors[ix].color);
            output.colors[ix] = color_space == 1 ? srgb_to_oklab(color) : color;
        }
    }

    return output;
}

float4 select_gradient_color(
    uint index,
    float4 color0,
    float4 color1,
    float4 color2,
    float4 color3) {
    if (index == 0) return color0;
    if (index == 1) return color1;
    if (index == 2) return color2;
    return color3;
}

float4 sample_gradient_stops(
    Background background,
    float position,
    float4 color0,
    float4 color1,
    float4 color2,
    float4 color3) {
    uint count = max(background.stop_count, 2u);
    float sample_position = clamp(position, 0.0, 1.0);
    uint left_ix = 0;
    uint right_ix = 0;
    float left_position = background.colors[0].percentage;
    float right_position = left_position;

    if (background.gradient_repeating != 0) {
        sample_position = frac(position + background.gradient_phase);
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
    float4 left_color = select_gradient_color(left_ix, color0, color1, color2, color3);
    float4 right_color = select_gradient_color(right_ix, color0, color1, color2, color3);
    float midpoint = background.gradient_midpoints[left_ix];
    float weight = t;
    if (midpoint != 0.5 && t > 0.0 && t < 1.0) {
        weight = pow(t, log(0.5) / log(midpoint));
    }
    float4 color = lerp(left_color, right_color, weight);
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
        color = weight0 * select_gradient_color(before_ix, color0, color1, color2, color3)
            + weight1 * left_color
            + weight2 * right_color
            + weight3 * select_gradient_color(after_ix, color0, color1, color2, color3);
    }
    return color;
}

float4 sample_linear_gradient(
    Background background,
    float position,
    float4 color0,
    float4 color1,
    float4 color2,
    float4 color3) {
    float4 color = sample_gradient_stops(background, position, color0, color1, color2, color3);
    float width = background.angular_seam_width;
    float half_width = width * 0.5;
    if (background.tag == 5 && background.gradient_repeating == 0 && width > 0.0
        && (position < half_width || position > 1.0 - half_width)) {
        float4 seam_start = sample_gradient_stops(background, 1.0 - half_width, color0, color1, color2, color3);
        float4 seam_end = sample_gradient_stops(background, half_width, color0, color1, color2, color3);
        float wrapped = position < half_width ? position : position - 1.0;
        color = lerp(seam_start, seam_end, smoothstep(-half_width, half_width, wrapped));
    }
    return background.color_space == 1 ? oklab_to_srgb(color) : color;
}

float2x2 rotate2d(float angle) {
    float s = sin(angle);
    float c = cos(angle);
    return float2x2(c, -s, s, c);
}

float4 gradient_color(Background background,
                      float2 position,
                      Bounds bounds,
                      float4 solid_color,
                      float4 color0,
                      float4 color1,
                      float4 color2,
                      float4 color3) {
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
            if (bounds.size.x > bounds.size.y) {
                direction.y *= bounds.size.y / bounds.size.x;
            } else {
                direction.x *=  bounds.size.x / bounds.size.y;
            }

            // Get the t value for the linear gradient with the color stop percentages.
            float2 half_size = bounds.size * 0.5;
            float2 center = bounds.origin + half_size;
            float2 center_to_point = position - center;
            float t = dot(center_to_point, direction) / length(direction);
            // Check the direct to determine the use x or y
            if (abs(direction.x) > abs(direction.y)) {
                t = (t + half_size.x) / bounds.size.x;
            } else {
                t = (t + half_size.y) / bounds.size.y;
            }

            if (background.tag != 1) {
                float2 q = center_to_point / max(half_size, float2(0.0001, 0.0001));
                float c = cos(radians), s = sin(radians);
                float2 r = float2(q.x * c + q.y * s, -q.x * s + q.y * c);
                if (background.tag == 4) t = length(q);
                else if (background.tag == 5) t = frac((atan2(q.y, q.x) - radians) / (2.0 * M_PI_F) + 1.0);
                else t = abs(r.x) + abs(r.y);
            }
            color = sample_linear_gradient(
                background, t, color0, color1, color2, color3);

            // Dither to reduce banding in gradients (especially dark/alpha).
            // Triangular-distributed noise breaks up 8-bit quantization steps.
            // ±2/255 for RGB (enough for dark-on-dark compositing),
            // ±3/255 for alpha (needs more because alpha × dark color = tiny steps).
            {
                float2 seed = position * 0.6180339887; // golden ratio spread
                float r1 = frac(sin(dot(seed, float2(12.9898, 78.233))) * 43758.5453);
                float r2 = frac(sin(dot(seed, float2(39.3460, 11.135))) * 24634.6345);
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
            float2 relative_position = position - bounds.origin;
            float2 rotated_point = mul(relative_position, rotation);
            float pattern = fmod(rotated_point.x, pattern_period);
            float distance = min(pattern, pattern_period - pattern) - pattern_period * (pattern_width / pattern_height) /  2.0f;
            color = solid_color;
            color.a *= saturate(0.5 - distance);
            break;
        }
        case 3: {
            // checkerboard
            float size = background.gradient_angle_or_pattern_height;
            float2 relative_position = position - bounds.origin;

            float x_index = floor(relative_position.x / size);
            float y_index = floor(relative_position.y / size);
            float should_be_colored = (x_index + y_index) % 2.0;

            color = solid_color;
            color.a *= saturate(should_be_colored);
            break;
        }
    }

    return color;
}

// Returns the dash velocity of a corner given the dash velocity of the two
// sides, by returning the slower velocity (larger dashes).
//
// Since 0 is used for dash velocity when the border width is 0 (instead of
// +inf), this returns the other dash velocity in that case.
//
// An alternative to this might be to appropriately interpolate the dash
// velocity around the corner, but that seems overcomplicated.
/*
**
**              Path Rasterization
**
*/

struct PathRasterizationSprite {
    float2 xy_position;
    float2 st_position;
    Background color;
    Bounds bounds;
};

StructuredBuffer<PathRasterizationSprite> path_rasterization_sprites: register(t1);

struct PathVertexOutput {
    float4 position: SV_Position;
    float2 st_position: TEXCOORD0;
    nointerpolation uint vertex_id: TEXCOORD1;
    float4 clip_distance: SV_ClipDistance;
};

struct PathFragmentInput {
    float4 position: SV_Position;
    float2 st_position: TEXCOORD0;
    nointerpolation uint vertex_id: TEXCOORD1;
};

PathVertexOutput path_rasterization_vertex(uint vertex_id: SV_VertexID) {
    PathRasterizationSprite sprite = path_rasterization_sprites[vertex_id];

    PathVertexOutput output;
    output.position = to_device_position_impl(sprite.xy_position);
    output.st_position = sprite.st_position;
    output.vertex_id = vertex_id;
    output.clip_distance = distance_from_clip_rect_impl(sprite.xy_position, sprite.bounds);

    return output;
}

float4 path_rasterization_fragment(PathFragmentInput input): SV_Target {
    float2 dx = ddx(input.st_position);
    float2 dy = ddy(input.st_position);
    PathRasterizationSprite sprite = path_rasterization_sprites[input.vertex_id];

    Background background = sprite.color;
    Bounds bounds = sprite.bounds;

    float alpha;
    if (length(float2(dx.x, dy.x))) {
        alpha = 1.0;
    } else {
        float2 gradient = 2.0 * input.st_position.xx * float2(dx.x, dy.x) - float2(dx.y, dy.y);
        float f = input.st_position.x * input.st_position.x - input.st_position.y;
        float distance = f / length(gradient);
        alpha = saturate(0.5 - distance);
    }

    GradientColor gradient = prepare_gradient_color(
        background.tag, background.color_space, background.solid, background.colors);

    float4 color = gradient_color(
        background,
        input.position.xy,
        bounds,
        gradient.solid,
        gradient.colors[0],
        gradient.colors[1],
        gradient.colors[2],
        gradient.colors[3]);
    return float4(color.rgb * color.a * alpha, alpha * color.a);
}

/*
**
**              Path Sprites
**
*/

struct PathSprite {
    Bounds bounds;
};

struct PathSpriteVertexOutput {
    float4 position: SV_Position;
    float2 texture_coords: TEXCOORD0;
};

StructuredBuffer<PathSprite> path_sprites: register(t1);

PathSpriteVertexOutput path_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    PathSprite sprite = path_sprites[sprite_id];

    // Don't apply content mask because it was already accounted for when rasterizing the path
    float4 device_position = to_device_position(unit_vertex, sprite.bounds);

    float2 screen_position = sprite.bounds.origin + unit_vertex * sprite.bounds.size;
    float2 texture_coords = screen_position / global_viewport_size;

    PathSpriteVertexOutput output;
    output.position = device_position;
    output.texture_coords = texture_coords;
    return output;
}

float4 path_sprite_fragment(PathSpriteVertexOutput input): SV_Target {
    return t_sprite.Sample(s_sprite, input.texture_coords);
}

/*
**
**              Monochrome sprites
**
*/

struct MonochromeSprite {
    uint order;
    uint pad;
    Bounds bounds;
    Bounds content_mask;
    Background background;
    Bounds background_bounds;
    AtlasTile tile;
    TransformationMatrix transformation;
};

struct MonochromeSpriteVertexOutput {
    nointerpolation uint sprite_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 tile_position: TEXCOORD1;
    float2 local_position: TEXCOORD2;
    nointerpolation float4 background_solid: COLOR0;
    nointerpolation float4 background_color0: COLOR1;
    nointerpolation float4 background_color1: COLOR2;
    nointerpolation float4 background_color2: COLOR3;
    nointerpolation float4 background_color3: COLOR4;
    float4 clip_distance: SV_ClipDistance;
};

struct MonochromeSpriteFragmentInput {
    nointerpolation uint sprite_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 tile_position: TEXCOORD1;
    float2 local_position: TEXCOORD2;
    nointerpolation float4 background_solid: COLOR0;
    nointerpolation float4 background_color0: COLOR1;
    nointerpolation float4 background_color1: COLOR2;
    nointerpolation float4 background_color2: COLOR3;
    nointerpolation float4 background_color3: COLOR4;
    float4 clip_distance: SV_ClipDistance;
};

StructuredBuffer<MonochromeSprite> mono_sprites: register(t1);

MonochromeSpriteVertexOutput monochrome_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    MonochromeSprite sprite = mono_sprites[sprite_id];
    float4 device_position =
        to_device_position_transformed(unit_vertex, sprite.bounds, sprite.transformation);
    float4 clip_distance = distance_from_clip_rect_transformed(unit_vertex, sprite.bounds, sprite.content_mask, sprite.transformation);
    float2 tile_position = to_tile_position(unit_vertex, sprite.tile);
    float2 local_position = unit_vertex * sprite.bounds.size + sprite.bounds.origin;
    GradientColor gradient = prepare_gradient_color(
        sprite.background.tag,
        sprite.background.color_space,
        sprite.background.solid,
        sprite.background.colors
    );

    MonochromeSpriteVertexOutput output;
    output.sprite_id = sprite_id;
    output.position = device_position;
    output.tile_position = tile_position;
    output.local_position = local_position;
    output.background_solid = gradient.solid;
    output.background_color0 = gradient.colors[0];
    output.background_color1 = gradient.colors[1];
    output.background_color2 = gradient.colors[2];
    output.background_color3 = gradient.colors[3];
    output.clip_distance = clip_distance;
    return output;
}

float4 monochrome_sprite_fragment(MonochromeSpriteFragmentInput input): SV_Target {
    MonochromeSprite sprite = mono_sprites[input.sprite_id];
    float4 color = gradient_color(
        sprite.background,
        input.local_position,
        sprite.background_bounds,
        input.background_solid,
        input.background_color0,
        input.background_color1,
        input.background_color2,
        input.background_color3
    );
    float sample = t_sprite.Sample(s_sprite, input.tile_position).r;
    float alpha_corrected = apply_contrast_and_gamma_correction(sample, color.rgb, grayscale_enhanced_contrast, gamma_ratios);
    return float4(color.rgb, color.a * alpha_corrected);
}

MonochromeSpriteVertexOutput subpixel_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    return monochrome_sprite_vertex(vertex_id, sprite_id);
}

SubpixelSpriteFragmentOutput subpixel_sprite_fragment(MonochromeSpriteFragmentInput input) {
    float3 sample = t_sprite.Sample(s_sprite, input.tile_position).rgb;
    if (is_bgr) {
        sample = sample.bgr;
    }
    float3 alpha_corrected = apply_contrast_and_gamma_correction3(sample, input.background_solid.rgb, subpixel_enhanced_contrast, gamma_ratios);

    SubpixelSpriteFragmentOutput output;
    output.foreground = float4(input.background_solid.rgb, 1.0f);
    output.alpha = float4(input.background_solid.a * alpha_corrected, 1.0f);
    return output;
}

/*
**
**              Polychrome sprites
**
*/

struct PolychromeSprite {
    uint order;
    uint pad;
    uint grayscale;
    float opacity;
    Bounds bounds;
    Bounds clip_bounds;
    Bounds content_mask;
    Corners corner_radii;
    AtlasTile tile;
    TransformationMatrix transformation;
};

struct PolychromeSpriteVertexOutput {
    nointerpolation uint sprite_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 tile_position: POSITION;
    float2 local_position: TEXCOORD1;
    float4 clip_distance: SV_ClipDistance;
};

struct PolychromeSpriteFragmentInput {
    nointerpolation uint sprite_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 tile_position: POSITION;
    float2 local_position: TEXCOORD1;
};

StructuredBuffer<PolychromeSprite> poly_sprites: register(t1);

PolychromeSpriteVertexOutput polychrome_sprite_vertex(uint vertex_id: SV_VertexID, uint sprite_id: SV_InstanceID) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    PolychromeSprite sprite = poly_sprites[sprite_id];
    float4 device_position = to_device_position_transformed(
        unit_vertex, sprite.bounds, sprite.transformation);
    float4 clip_distance = distance_from_clip_rect_transformed(
        unit_vertex, sprite.bounds, sprite.content_mask, sprite.transformation);
    float2 tile_position = to_tile_position(unit_vertex, sprite.tile);
    float2 local_position = unit_vertex * sprite.bounds.size + sprite.bounds.origin;

    PolychromeSpriteVertexOutput output;
    output.position = device_position;
    output.tile_position = tile_position;
    output.local_position = local_position;
    output.sprite_id = sprite_id;
    output.clip_distance = clip_distance;
    return output;
}

float4 polychrome_sprite_fragment(PolychromeSpriteFragmentInput input): SV_Target {
    PolychromeSprite sprite = poly_sprites[input.sprite_id];
    float2 atlas_size;
    t_sprite.GetDimensions(atlas_size.x, atlas_size.y);
    float2 tile_min = (float2(sprite.tile.bounds.origin) + 0.5) / atlas_size;
    float2 tile_max = (float2(sprite.tile.bounds.origin + sprite.tile.bounds.size) - 0.5) / atlas_size;
    float4 sample = t_sprite.Sample(s_sprite, clamp(input.tile_position, tile_min, tile_max));
    float distance = quad_sdf(input.local_position, sprite.clip_bounds, sprite.corner_radii);

    float4 color = sample;
    if (sprite.grayscale != 0u) {
        float3 grayscale = dot(color.rgb, GRAYSCALE_FACTORS);
        color = float4(grayscale, sample.a);
    }
    color.a *= sprite.opacity * saturate(0.5 - distance);
    return color;
}

/*
**
**              Dynamic surfaces
*/

struct SurfaceInstance {
    Bounds bounds;
    Bounds clip_bounds;
    Bounds content_mask;
    Bounds uv_bounds;
    Corners corner_radii;
    float4 color_rows[3];
    float opacity;
    float3 pad;
};

struct SurfaceVertexOutput {
    nointerpolation uint surface_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 texture_position: TEXCOORD1;
    float4 clip_distance: SV_ClipDistance;
};

struct SurfaceFragmentInput {
    nointerpolation uint surface_id: TEXCOORD0;
    float4 position: SV_Position;
    float2 texture_position: TEXCOORD1;
};

StructuredBuffer<SurfaceInstance> surfaces: register(t1);
Texture2D<float4> t_surface_uv: register(t2);

SurfaceVertexOutput surface_vertex_impl(uint vertex_id, uint surface_id) {
    float2 unit_vertex = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    SurfaceInstance surface = surfaces[surface_id];

    SurfaceVertexOutput output;
    output.surface_id = surface_id;
    output.position = to_device_position(unit_vertex, surface.bounds);
    output.texture_position =
        surface.uv_bounds.origin + unit_vertex * surface.uv_bounds.size;
    output.clip_distance =
        distance_from_clip_rect(unit_vertex, surface.bounds, surface.content_mask);
    return output;
}

SurfaceVertexOutput surface_rgba_vertex(uint vertex_id: SV_VertexID, uint surface_id: SV_InstanceID) {
    return surface_vertex_impl(vertex_id, surface_id);
}

float4 surface_rgba_fragment(SurfaceFragmentInput input): SV_Target {
    SurfaceInstance surface = surfaces[input.surface_id];
    float4 color = t_sprite.Sample(s_sprite, input.texture_position);
    float distance = quad_sdf(input.position.xy, surface.clip_bounds, surface.corner_radii);
    color.a *= surface.opacity * saturate(0.5 - distance);
    return color;
}

SurfaceVertexOutput surface_nv12_vertex(uint vertex_id: SV_VertexID, uint surface_id: SV_InstanceID) {
    return surface_vertex_impl(vertex_id, surface_id);
}

float4 surface_nv12_fragment(SurfaceFragmentInput input): SV_Target {
    SurfaceInstance surface = surfaces[input.surface_id];
    float4 yuv = float4(
        t_sprite.Sample(s_sprite, input.texture_position).r,
        t_surface_uv.Sample(s_sprite, input.texture_position).rg,
        1.0);
    float3 rgb = float3(
        dot(surface.color_rows[0], yuv),
        dot(surface.color_rows[1], yuv),
        dot(surface.color_rows[2], yuv));
    float distance = quad_sdf(input.position.xy, surface.clip_bounds, surface.corner_radii);
    float alpha = surface.opacity * saturate(0.5 - distance);
    return float4(rgb, alpha);
}
