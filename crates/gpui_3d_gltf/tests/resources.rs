use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use gpui_3d_gltf::{Document, Limits};
use serde_json::{Value, json};

fn parse(value: &Value) -> Document {
    Document::from_slice(&serde_json::to_vec(value).unwrap(), Limits::default()).unwrap()
}

#[test]
fn zero_initialized_accessors_preserve_schema_and_reference_validation() {
    let value = json!({"asset":{"version":"2.0"},
        "accessors":[{"componentType":5126,"count":3,"type":"VEC3"}]});
    let document = parse(&value);
    let prepared = document
        .prepare(|_| panic!("no resources to resolve"))
        .unwrap();
    assert_eq!(prepared.resource_bytes(), 0);
    for (field, invalid) in [
        ("componentType", json!(9999)),
        ("type", json!("BAD")),
        ("bufferView", json!(8)),
        ("byteOffset", json!(4)),
        ("count", json!(0)),
    ] {
        let mut invalid_document = value.clone();
        invalid_document["accessors"][0][field] = invalid;
        assert!(
            Document::from_slice(
                &serde_json::to_vec(&invalid_document).unwrap(),
                Limits::default()
            )
            .is_err(),
            "{field}"
        );
    }
    let mut invalid_mesh = value;
    invalid_mesh["meshes"] = json!([{"primitives":[{"attributes":{"POSITION":7}}]}]);
    assert!(
        Document::from_slice(
            &serde_json::to_vec(&invalid_mesh).unwrap(),
            Limits::default()
        )
        .is_err()
    );
}

fn binary(json: &Value, bytes: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(json).unwrap();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut data = bytes.to_vec();
    data.resize(data.len().next_multiple_of(4), 0);
    let length = 12 + 8 + json.len() + 8 + data.len();
    let mut result = b"glTF".to_vec();
    result.extend_from_slice(&2_u32.to_le_bytes());
    result.extend_from_slice(&(length as u32).to_le_bytes());
    result.extend_from_slice(&(json.len() as u32).to_le_bytes());
    result.extend_from_slice(&0x4e4f534a_u32.to_le_bytes());
    result.extend(json);
    result.extend_from_slice(&(data.len() as u32).to_le_bytes());
    result.extend_from_slice(&0x004e4942_u32.to_le_bytes());
    result.extend(data);
    result
}

#[test]
fn uri_resolution_preserves_policy_and_shares_duplicate_and_view_payloads() {
    let document = parse(&json!({
        "asset": {"version":"2.0"},
        "buffers": [{"byteLength":4,"uri":"../data%20set.bin"}, {"byteLength":2,"uri":"../data%20set.bin"}],
        "bufferViews": [{"buffer":0,"byteOffset":1,"byteLength":2}],
        "images": [{"bufferView":0,"mimeType":"image/png"}, {"uri":"custom:cover"}, {"uri":"custom:cover"}]
    }));
    let mut calls = HashMap::new();
    let prepared = document
        .prepare(|uri| {
            *calls.entry(uri.to_owned()).or_insert(0) += 1;
            match uri {
                "../data%20set.bin" => Ok(vec![1, 2, 3, 4, 5]),
                "custom:cover" => Ok(vec![9, 8, 7]),
                _ => panic!("unexpected resolver URI"),
            }
        })
        .unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.values().all(|&count| count == 1));
    assert_eq!(prepared.resource_bytes(), 8);
    assert_eq!(prepared.buffer(0).unwrap(), [1, 2, 3, 4]);
    assert_eq!(prepared.buffer(1).unwrap(), [1, 2]);
    assert_eq!(prepared.image(0).unwrap().bytes(), [2, 3]);
    assert_eq!(prepared.image(0).unwrap().mime_type(), Some("image/png"));
    assert!(std::ptr::eq(
        prepared.buffer(0).unwrap()[1..3].as_ptr(),
        prepared.image(0).unwrap().bytes().as_ptr()
    ));
    assert!(std::ptr::eq(
        prepared.image(1).unwrap().bytes().as_ptr(),
        prepared.image(2).unwrap().bytes().as_ptr()
    ));
    let retained = prepared.clone();
    drop(prepared);
    drop(document);
    assert_eq!(retained.image(1).unwrap().bytes(), [9, 8, 7]);
    assert_eq!(retained.gltf().buffers().len(), 2);
    assert!(retained.buffer(2).is_none());
    assert!(retained.image(3).is_none());
}

#[test]
fn glb_padding_is_excluded_from_buffer_reads_and_resource_views_share_storage() {
    let value = json!({
        "asset":{"version":"2.0"}, "buffers":[{"byteLength":5}],
        "bufferViews":[{"buffer":0,"byteOffset":1,"byteLength":4}],
        "images":[{"bufferView":0,"mimeType":"image/png"}]
    });
    let document =
        Document::from_slice(&binary(&value, &[1, 2, 3, 4, 5]), Limits::default()).unwrap();
    let prepared = document
        .prepare(|_| panic!("GLB should not resolve a URI"))
        .unwrap();
    assert_eq!(prepared.buffer(0).unwrap(), [1, 2, 3, 4, 5]);
    assert_eq!(prepared.image(0).unwrap().bytes(), [2, 3, 4, 5]);
    assert_eq!(prepared.resource_bytes(), 8);
    let missing =
        Document::from_slice(&serde_json::to_vec(&value).unwrap(), Limits::default()).unwrap_err();
    assert!(format!("{missing:#}").contains("missing GLB binary chunk"));
}

#[test]
fn base64_inputs_obey_exact_budgets_and_never_escape_to_the_resolver() {
    let uri = format!(
        "data:application/octet-stream;base64,{}",
        STANDARD.encode([1, 2, 3, 4])
    );
    let value = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":uri}]});
    let bytes = serde_json::to_vec(&value).unwrap();
    let prepared = Document::from_slice(
        &bytes,
        Limits {
            resource_bytes: 4,
            ..Limits::default()
        },
    )
    .unwrap()
    .prepare(|_| panic!("data URI must not be delegated"))
    .unwrap();
    assert_eq!(prepared.resource_bytes(), 4);
    assert_eq!(prepared.buffer(0).unwrap(), [1, 2, 3, 4]);
    assert!(
        Document::from_slice(
            &bytes,
            Limits {
                resource_bytes: 3,
                ..Limits::default()
            }
        )
        .unwrap()
        .prepare(|_| panic!("over-budget data URI must not be delegated"))
        .is_err()
    );
    for uri in [
        "data:image/png,abc",
        "data:image/png;base64,####",
        "data:image/png;base64,a===",
        "data:image/png;base64",
    ] {
        let value = json!({"asset":{"version":"2.0"}, "images":[{"uri":uri}]});
        let error = parse(&value)
            .prepare(|_| panic!("invalid data URI must not be delegated"))
            .unwrap_err();
        assert!(format!("{error:#}").contains("image 0"));
    }
    let image = parse(
        &json!({"asset":{"version":"2.0"},"images":[{"uri":"data:image/png;base64,AQ==","mimeType":"image/jpeg"}]}),
    );
    assert!(
        format!("{:#}", image.prepare(|_| unreachable!()).unwrap_err())
            .contains("MIME type differs")
    );
}

#[test]
fn resource_failures_are_contextual_retryable_and_do_not_expose_partial_results() {
    let document =
        parse(&json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"mesh.bin"}]}));
    for result in [Err(anyhow::anyhow!("offline")), Ok(vec![0; 3])] {
        let mut result = Some(result);
        let error = document.prepare(|_| result.take().unwrap()).unwrap_err();
        assert!(format!("{error:#}").contains("buffer 0"));
    }
    let prepared = document.prepare(|_| Ok(vec![0; 4])).unwrap();
    assert_eq!(prepared.buffer(0).unwrap().len(), 4);
    let bytes =
        serde_json::to_vec(&json!({"asset":{"version":"2.0"},"images":[{"uri":"a"},{"uri":"b"}]}))
            .unwrap();
    let document = Document::from_slice(
        &bytes,
        Limits {
            resource_bytes: 5,
            ..Limits::default()
        },
    )
    .unwrap();
    let error = document.prepare(|_| Ok(vec![0; 3])).unwrap_err();
    assert!(format!("{error:#}").contains("image 1"));
    assert!(format!("{error:#}").contains("exceeds limit 5"));
    assert_eq!(
        document
            .prepare(|_| Ok(vec![0; 2]))
            .unwrap()
            .resource_bytes(),
        4
    );
}

fn sparse_document() -> Value {
    json!({
        "asset":{"version":"2.0"}, "buffers":[{"uri":"sparse.bin","byteLength":28}],
        "bufferViews":[{"buffer":0,"byteLength":2},{"buffer":0,"byteOffset":4,"byteLength":24}],
        "accessors":[{"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[4,5,6],
            "sparse":{"count":2,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
    })
}

#[test]
fn validated_sparse_buffers_feed_accessor_readers_and_reject_bad_indices() {
    let document = parse(&sparse_document());
    let mut payload = vec![0, 2, 0, 0];
    for value in [1_f32, 2., 3., 4., 5., 6.] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    let prepared = document.prepare(|_| Ok(payload.clone())).unwrap();
    let mesh = prepared.gltf().meshes().next().unwrap();
    let primitive = mesh.primitives().next().unwrap();
    let positions: Vec<_> = primitive
        .reader(|buffer| prepared.buffer(buffer.index()))
        .read_positions()
        .unwrap()
        .collect();
    assert_eq!(positions, [[1., 2., 3.], [0., 0., 0.], [4., 5., 6.]]);
    for (component, size) in [(5123, 2), (5125, 4)] {
        let mut value = sparse_document();
        value["buffers"][0]["byteLength"] = (size * 2 + 24).into();
        value["bufferViews"][0]["byteLength"] = (size * 2).into();
        value["bufferViews"][1]["byteOffset"] = (size * 2).into();
        value["accessors"][0]["count"] = 257.into();
        value["accessors"][0]["sparse"]["indices"]["componentType"] = component.into();
        let mut data = vec![0; size * 2];
        data[size..size * 2].copy_from_slice(&256_u32.to_le_bytes()[..size]);
        data.extend_from_slice(&payload[4..]);
        let prepared = parse(&value).prepare(|_| Ok(data.clone())).unwrap();
        let mesh = prepared.gltf().meshes().next().unwrap();
        let primitive = mesh.primitives().next().unwrap();
        let positions: Vec<_> = primitive
            .reader(|buffer| prepared.buffer(buffer.index()))
            .read_positions()
            .unwrap()
            .collect();
        assert_eq!(positions.len(), 257);
        assert_eq!(positions[0], [1., 2., 3.]);
        assert_eq!(positions[1], [0.; 3]);
        assert_eq!(positions[256], [4., 5., 6.]);
    }
    for indices in [[0, 0], [2, 1], [0, 3]] {
        let mut invalid = payload.clone();
        invalid[..2].copy_from_slice(&indices);
        let error = document.prepare(|_| Ok(invalid.clone())).unwrap_err();
        assert!(format!("{error:#}").contains("accessor 0 sparse index 1"));
    }
}

#[test]
fn declared_ranges_strides_and_padded_matrices_are_checked_before_io() {
    let base = json!({
        "asset":{"version":"2.0"},"buffers":[{"uri":"a","byteLength":24}],
        "bufferViews":[{"buffer":0,"byteLength":24}],
        "accessors":[{"bufferView":0,"componentType":5121,"count":2,"type":"MAT3"}]
    });
    assert!(Document::from_slice(&serde_json::to_vec(&base).unwrap(), Limits::default()).is_ok());
    for (section, field, value) in [
        ("bufferViews", "byteLength", 25_u64),
        ("bufferViews", "byteOffset", u64::MAX),
        ("bufferViews", "byteStride", 5),
        ("accessors", "count", 3),
        ("accessors", "count", u64::MAX),
        ("accessors", "byteOffset", 1),
    ] {
        let mut invalid = base.clone();
        invalid[section][0][field] = value.into();
        assert!(
            Document::from_slice(&serde_json::to_vec(&invalid).unwrap(), Limits::default())
                .is_err(),
            "{section}.{field}={value}"
        );
    }
    let mut sparse = sparse_document();
    sparse["accessors"][0]["sparse"]["count"] = 4.into();
    assert!(
        Document::from_slice(&serde_json::to_vec(&sparse).unwrap(), Limits::default()).is_err()
    );
    let mut sparse = sparse_document();
    sparse["bufferViews"][1]["byteOffset"] = 3.into();
    assert!(
        Document::from_slice(&serde_json::to_vec(&sparse).unwrap(), Limits::default()).is_err()
    );
}

#[test]
fn malformed_containers_versions_and_limits_fail_without_panics() {
    let value = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}]});
    let valid = binary(&value, &[0; 4]);
    for length in [0_u32, 8, 12, valid.len() as u32 - 1, valid.len() as u32 + 1] {
        let mut bytes = valid.clone();
        bytes[8..12].copy_from_slice(&length.to_le_bytes());
        assert!(Document::from_slice(&bytes, Limits::default()).is_err());
    }
    for end in 0..valid.len() {
        assert!(Document::from_slice(&valid[..end], Limits::default()).is_err());
    }
    let mut bytes = valid.clone();
    bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Document::from_slice(&bytes, Limits::default()).is_err());
    for version in ["1.0", "2.1", "garbage"] {
        let value = json!({"asset":{"version":version}});
        assert!(
            Document::from_slice(&serde_json::to_vec(&value).unwrap(), Limits::default()).is_err()
        );
    }
    assert!(
        Document::from_slice(
            &valid,
            Limits {
                document_bytes: valid.len() - 1,
                ..Limits::default()
            }
        )
        .is_err()
    );
    assert!(
        Document::from_slice(
            &valid,
            Limits {
                buffers: 0,
                ..Limits::default()
            }
        )
        .is_err()
    );
    let sparse = serde_json::to_vec(&sparse_document()).unwrap();
    assert!(
        Document::from_slice(
            &sparse,
            Limits {
                accessors: 0,
                ..Limits::default()
            }
        )
        .is_err()
    );
}
