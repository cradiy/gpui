---
name: uic-component-design
description: Design and refactor public APIs for components under the GPUI uic crate. Use when adding or changing UIC components, public Appearance types, component styling methods, typography, layout, overlays, menus, modals, inputs, pickers, dropdowns, toasts, or material surfaces. Enforce Styled-first public APIs while keeping Appearance limited to component-specific semantic states.
---

# UIC Component Design

Design the public API first. Internal storage and rendering may use any private
representation that keeps behavior correct.

## Public API contract

Apply these rules to every public UIC component.

1. Implement gpui::Styled when the caller is styling a rendered component or
   overlay descriptor.
2. Expose ordinary element properties through Styled, including:
   - width, height, min/max size, margin, padding, gap, and positioning;
   - background, border, corner radius, shadow, and opacity;
   - font, font_family, font_weight, font_style, font_features, text_size,
     line_height, and text_color.
3. Keep a public Appearance field only when it describes a component-specific
   semantic state or optical primitive that one outer Styled element cannot
   express.
4. Keep implementation-only measurements and resolved styles private. The rule
   concerns the public API, not internal data structures.

Adding impl Styled is not a migration by itself. Remove duplicate public
Appearance fields and public setters, update callers, and make Styled the real
source of truth.

## Classify properties

Use Styled for common properties:

~~~text
background  border  border_width  radius  shadow  opacity
width  height  min/max size  margin  padding  gap
font family/weight/style/features  text size/line height/color
~~~

Keep semantic state in Appearance:

~~~text
selected / disabled / danger / error / focus colors
placeholder / caret / selection colors
track / thumb / marker / accent colors
material blur / tint / optical edge / merge response
~~~

Do not put behavior or placement policy in Appearance. Use named configuration
or builder methods for values such as submenu delay, viewport avoidance,
placement direction, dismissal policy, or maximum nesting.

## Top-level boundary

A component's Styled implementation targets its main public surface. Migrate
ordinary properties when they form a meaningful top-level styling API: size,
spacing, background, border, radius, typography, shadow, and opacity.

- Do not create a StyleRefinement target for every internal div.
- Keep one or two local properties of a child part in the component-specific
  Appearance when that is clearer, such as marker size, row height, separator
  color, or button hover color.
- Add a separate public style target only when the child is itself a substantial
  public surface with many independently useful properties and callers need to
  style it as a unit.
- Prefer a small semantic Appearance over a proliferation of item_style,
  marker_style, header_style, and similar closures.

For overlays, store the top-level style refinement on the public descriptor and
apply it to the actual rendered layer. Root menus and submenus must inherit
typography consistently.

## Typography

Verify the complete inherited text style, not only font(...) or
text_color(...). Custom-painted text must use window.text_style() so
font_family, fallbacks, features, weight, style, size, line height, and color
reach the final TextRun.

Child content inherits the component style by default. A caller-provided child
may override it locally.

## Migration workflow

1. Inspect the public component, its Appearance type, render root, custom paint
   code, examples, docs, and repository call sites.
2. List each public property as common style, semantic state, behavior, or
   private implementation data.
3. Add or correct Styled forwarding so refinements reach the real rendered
   surface and override defaults.
4. Remove common public Appearance fields and duplicated builder methods.
5. Add a subpart style target only when it represents a substantial public
   surface, not one or two implementation details.
6. Move behavior and placement values to named configuration APIs.
7. Migrate every in-repository caller without a compatibility layer unless the
   user explicitly requests compatibility.
8. Write documentation as a coherent API guide. Do not add patch-note prose
   explaining that a property was newly migrated.

## Review checks

Reject public Appearance fields that duplicate ordinary properties of the
component's main Styled surface. Local child geometry or state styling may stay
in Appearance when exposing another style target would add more API than value.

Reject implementations where Styled exists but render ignores its refinement,
or where Appearance continues to control the same public property.

Prefer a public call shaped like:

~~~rust
Widget::new()
    .w(px(320.0))
    .p_3()
    .rounded(px(12.0))
    .bg(surface)
    .font_family("Inter")
    .text_size(px(15.0))
~~~

Keep only semantic fields in WidgetAppearance, and apply private defaults before
the caller's StyleRefinement so explicit Styled calls win.

## Validation

Run validation proportional to the change:

~~~sh
cargo fmt -p uic
cargo test -p uic
cargo check -p uic --all-targets
cargo check --workspace
git diff --check
~~~

Add focused tests only when they exercise observable behavior or protect a
meaningful invariant. Do not extract a constant-returning helper solely to make
it testable, assert that a hard-coded value equals the same hard-coded value, or
add a test that merely restates the implementation. If the change is already
covered by type checking and has no behavior that can fail independently, use
the appropriate check or real rendered inspection instead of manufacturing a
unit test.

Useful focused tests can prove that:

- the component exposes the expected Styled chain;
- Styled values reach custom-painted text or nested overlay levels;
- semantic states still override base style where intended;
- changing style does not change component layout unexpectedly.

For visual components, run the relevant example and inspect the rendered result.
Compilation alone does not validate layout, inheritance, clipping, or material
behavior.
