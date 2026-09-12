# Punctual lights

`KHR_lights_punctual` supplies directional, point and spot sources. Scene conversion
attaches each referenced source to its original node; `SceneNode::light_index`
retains the document-local light index. Names remain available through
`PreparedDocument::gltf().lights()`. Light nodes add no geometry or picking coverage.

`PreparedDocument::light(index)` converts an individual source to a local-space
`PunctualLight`. Point and spot sources are at the origin. Spot emission points
down -Z; directional sources use +Z because the core stores the direction toward
the source. Linear RGB colors are converted to the core's sRGB input convention.

Node hierarchy and animation transform positions and normalized directions.
Range, intensity and cone angles are unchanged by scale. Shared light definitions
can be attached to multiple nodes; instantiated subtrees have independent poses,
visibility and overrides through `SceneGraph::set_light`.

```rust
use gpui_3d::{Camera, Scene, SceneGraph};
use gpui_3d_gltf::SceneAsset;

fn scene_with_lights(asset: &SceneAsset) -> anyhow::Result<Scene> {
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree())?;
    Ok(graph.evaluate()?.scene(Camera::default()))
}
```

Evaluated scenes use visible node lights instead of the default directional
source whenever the graph contains light-bearing nodes. Hiding all such nodes
leaves an empty direct-light list. Ambient and environment illumination remain
independently configurable. Assets without light nodes retain default lighting.
Shadows are not enabled automatically.

## Parameters and limits

- Color channels must be finite and in `[0, 1]`.
- The authored intensity value is retained as the core's linear multiplier.
  Values must be finite and in `[0, 65504]`. The core does not provide calibrated
  photometric exposure; importing the numeric candela/lux value does not establish
  an absolute display brightness.
- Point and spot range is optional, finite and positive; directional range is
  invalid. Attenuation uses the core's inverse-square falloff and a minimum
  distance of 0.01 world units.
- Spot half-angles are in radians and satisfy
  `0 <= inner < outer <= pi/2`. Their cosine values must remain distinguishable
  in the core's floating-point representation. Omitted angles default to zero
  and `pi/4`; the spot object itself is required.
- `SceneOptions::light_limit` counts light-bearing nodes in the selected scene,
  including multiple nodes sharing one light definition. Its default is eight.
  Raising this conversion limit does not raise the core rendering limit of eight
  visible sources across the complete scene. Callers must select or hide sources
  before rendering a larger graph.

Invalid references, parameters and unrepresentable world poses return errors.
No light is silently omitted to satisfy a limit. See [scenes](scenes.md) for
hierarchy admission and [core lighting](../../gpui_3d/docs/topics/lighting.md) for
rendering controls.
