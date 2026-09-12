# Import diagnostics

`Document::diagnostics()` exposes nonfatal source advisories before resource
loading. `PreparedDocument::diagnostics()` returns the same shared records after
preparation. Cloning, worker transfer, preparation failure and retry do not change
the records. The library does not log them or choose how the application responds.

```rust
use gpui_3d_gltf::{Document, ImportDiagnostic};

fn ignored_extensions(document: &Document) -> Vec<(&str, &str)> {
    document.diagnostics().iter().filter_map(|diagnostic| {
        match diagnostic {
            ImportDiagnostic::IgnoredOptionalExtension { extension, path } => {
                Some((extension.as_str(), path.as_str()))
            }
            _ => None,
        }
    }).collect()
}
```

`IgnoredOptionalExtension` identifies an unsupported optional extension whose
payload is ignored while base glTF data remains available for conversion. Each
payload occurrence has a separate record, so two materials using the same
extension remain distinguishable. Unsupported declarations without a scanned
payload produce one record per name at their first `extensionsUsed` entry.
Known supported extensions do not produce this advisory.

Paths are [JSON Pointers](https://www.rfc-editor.org/rfc/rfc6901.html) into the
source JSON, including the JSON chunk of a GLB. A payload path might be
`/materials/2/extensions/KHR_materials_clearcoat`; a declaration path might be
`/extensionsUsed/0`. Records are sorted lexicographically by path. `Display`
includes the extension and path with escaped control characters.

The scan covers the whole document, not only a selected scene. It does not inspect
`extras` or descend into an ignored extension's opaque payload. Nested extensions
inside supported extension payloads are inspected. Records do not retain payload
contents, resource bytes or application metadata.

## Limits and failures

`Limits::diagnostics` defaults to 4,096 records. `Limits::diagnostic_bytes` defaults
to 1 MiB and counts the total UTF-8 bytes in retained extension names and escaped
pointer paths. These limits apply during parsing, before external resource loading.
Exceeding either limit returns an error rather than truncating the report. Zero
permits documents with no advisories and rejects those requiring a record.
These budgets bound diagnostic output, not the parser's total temporary memory;
`document_bytes` still bounds the complete input.

Unsupported required extensions remain errors, not advisories. An empty report
does not certify complete asset compatibility: geometry, material, animation and
scene conversion perform their own validation and can fail independently.
[Tangent repair reports](geometry.md) describe converted geometry separately.

The `inspect` example prints document advisories before resolving resources.
Applications may display them, retain them alongside their asset state, or reject
the asset according to caller-owned policy.
