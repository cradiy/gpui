use gpui_3d_gltf::{Document, ImportDiagnostic, Limits};
use serde_json::{Value, json};

fn parse(source: &Value, limits: Limits) -> anyhow::Result<Document> {
    Document::from_slice(&serde_json::to_vec(source)?, limits)
}

fn entries(document: &Document) -> Vec<(&str, &str)> {
    document
        .diagnostics()
        .iter()
        .map(|diagnostic| match diagnostic {
            ImportDiagnostic::IgnoredOptionalExtension { extension, path } => {
                (extension.as_str(), path.as_str())
            }
            _ => panic!("unexpected diagnostic"),
        })
        .collect()
}

#[test]
fn optional_extension_occurrences_have_stable_paths_without_duplicate_declarations() {
    let source = json!({"asset":{"version":"2.0"},
    "extensionsUsed":["KHR_materials_clearcoat","VENDOR_unused","KHR_materials_unlit","KHR_mesh_quantization"],
    "extensionsRequired":["KHR_mesh_quantization"],
    "materials":[
        {"extensions":{"KHR_materials_clearcoat":{"clearcoatFactor":1.},"KHR_materials_unlit":{}}},
        {"extensions":{"KHR_materials_clearcoat":{"clearcoatFactor":0.5}}}
    ]});
    let document = parse(&source, Limits::default()).unwrap();
    assert_eq!(
        entries(&document),
        [
            ("VENDOR_unused", "/extensionsUsed/1"),
            (
                "KHR_materials_clearcoat",
                "/materials/0/extensions/KHR_materials_clearcoat"
            ),
            (
                "KHR_materials_clearcoat",
                "/materials/1/extensions/KHR_materials_clearcoat"
            ),
        ]
    );
    for (_, path) in entries(&document) {
        assert!(source.pointer(path).is_some());
    }
    let prepared = document.prepare(|_| panic!("no resources")).unwrap();
    assert!(prepared.material(Some(0)).is_ok());
    assert!(prepared.material(Some(1)).is_ok());
}

#[test]
fn extras_and_unknown_extension_payloads_are_opaque_but_known_light_nodes_are_inspected() {
    let source = json!({"asset":{"version":"2.0"},
    "extras":{"extensions":{"VENDOR_extra":{}}},
    "materials":[{"extras":{"extensions":{"VENDOR_material_extra":{}}},
        "extensions":{"VENDOR_surface":{"extensions":{"VENDOR_nested":{}}}}}],
    "extensions":{"KHR_lights_punctual":{"lights":[
        {"type":"point","extras":{"extensions":{"VENDOR_light_extra":{}}},
            "extensions":{"VENDOR_light_profile":{"value":2}}}
    ]}}});
    let document = parse(&source, Limits::default()).unwrap();
    assert_eq!(
        entries(&document),
        [
            (
                "VENDOR_light_profile",
                "/extensions/KHR_lights_punctual/lights/0/extensions/VENDOR_light_profile"
            ),
            ("VENDOR_surface", "/materials/0/extensions/VENDOR_surface"),
        ]
    );
    let prepared = document.prepare(|_| panic!()).unwrap();
    prepared.light(0).unwrap();
}

#[test]
fn escaped_json_pointers_resolve_and_display_does_not_emit_control_characters() {
    let name = "VENDOR_a~/中\n\u{1b}";
    let source = json!({"asset":{"version":"2.0"},"nodes":[{"extensions":{name:{"x":1}}}]});
    let document = parse(&source, Limits::default()).unwrap();
    let [(extension, path)] = entries(&document)[..] else {
        panic!()
    };
    assert_eq!(extension, name);
    assert_eq!(path, "/nodes/0/extensions/VENDOR_a~0~1中\n\u{1b}");
    assert_eq!(source.pointer(path), Some(&json!({"x":1})));
    let display = document.diagnostics()[0].to_string();
    assert!(!display.contains('\n'));
    assert!(!display.contains('\u{1b}'));
}

#[test]
fn preparation_retry_clones_and_worker_transfer_keep_shared_diagnostics() {
    let source = json!({"asset":{"version":"2.0"},
        "buffers":[{"uri":"data.bin","byteLength":4}],
        "extensionsUsed":["VENDOR_metadata"]});
    let document = parse(&source, Limits::default()).unwrap();
    let clone = document.clone();
    assert!(std::ptr::eq(document.diagnostics(), clone.diagnostics()));
    assert!(document.prepare(|_| anyhow::bail!("unavailable")).is_err());
    let prepared =
        futures::executor::block_on(document.prepare_async(|_| async { Ok(vec![0; 4]) })).unwrap();
    assert!(std::ptr::eq(document.diagnostics(), prepared.diagnostics()));
    drop(document);
    drop(clone);
    let prepared = std::thread::spawn(move || prepared).join().unwrap();
    let duplicate = prepared.clone();
    assert!(std::ptr::eq(
        prepared.diagnostics(),
        duplicate.diagnostics()
    ));
    assert_eq!(prepared.diagnostics().len(), 1);
    assert_eq!(prepared.buffer(0), Some(&[0; 4][..]));
}

#[test]
fn diagnostic_limits_admit_exact_counts_and_utf8_bytes_without_truncation() {
    let source = json!({"asset":{"version":"2.0"},"nodes":[
        {"extensions":{"VENDOR_中~":{"value":0}}},
        {"extensions":{"VENDOR_中~":{"value":1}}}
    ]});
    let document = parse(&source, Limits::default()).unwrap();
    let bytes = entries(&document)
        .iter()
        .map(|(name, path)| name.len() + path.len())
        .sum();
    let limits = Limits {
        diagnostics: 2,
        diagnostic_bytes: bytes,
        ..Default::default()
    };
    assert_eq!(
        parse(&source, limits).unwrap().diagnostics(),
        document.diagnostics()
    );
    for (limits, message) in [
        (
            Limits {
                diagnostics: 1,
                ..limits
            },
            "diagnostic count limit",
        ),
        (
            Limits {
                diagnostic_bytes: bytes - 1,
                ..limits
            },
            "diagnostic text byte limit",
        ),
        (
            Limits {
                diagnostics: 0,
                ..limits
            },
            "diagnostic count limit",
        ),
    ] {
        let error = parse(&source, limits).unwrap_err();
        assert!(format!("{error:#}").contains(message));
    }
    let none = json!({"asset":{"version":"2.0"},"extras":{"extensions":{"VENDOR_extra":{}}}});
    assert!(
        parse(
            &none,
            Limits {
                diagnostics: 0,
                diagnostic_bytes: 0,
                ..Default::default()
            }
        )
        .unwrap()
        .diagnostics()
        .is_empty()
    );
}

#[test]
fn json_and_glb_report_the_same_advisories_and_required_extensions_still_fail() {
    let mut source = json!({"asset":{"version":"2.0"},"extensionsUsed":["VENDOR_data"],
        "nodes":[{"extensions":{"VENDOR_data":{}}}]});
    let expected = parse(&source, Limits::default()).unwrap();
    let mut json = serde_json::to_vec(&source).unwrap();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut glb = b"glTF".to_vec();
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&((20 + json.len()) as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4e4f534au32.to_le_bytes());
    glb.extend(json);
    let binary = Document::from_slice(&glb, Limits::default()).unwrap();
    assert_eq!(expected.diagnostics(), binary.diagnostics());
    source["extensionsRequired"] = json!(["VENDOR_data"]);
    let error = parse(&source, Limits::default()).unwrap_err();
    assert!(format!("{error:#}").contains("VENDOR_data"));
}
