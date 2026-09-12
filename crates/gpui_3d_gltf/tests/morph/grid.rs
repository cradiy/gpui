use super::super::super::Fixture;
use serde_json::json;

pub(super) fn fixture(authored_normals: bool) -> Fixture {
    let mut fixture = Fixture::new();
    let mut positions = Vec::new();
    let mut deltas = Vec::new();
    let mut normals = Vec::new();
    let mut normal_deltas = Vec::new();
    let mut uv = Vec::new();
    for row in 0..=10 {
        for col in 0..=10 {
            let x = (col as f32 / 10.).powf(1.3);
            let y = row as f32 / 10.;
            positions.extend([x, y, 0.1 * x * y]);
            deltas.extend([-0.05 * y, 0.1 * x, 0.3 * x * y]);
            normals.extend([-0.1 * y, -0.1 * x, 1.]);
            normal_deltas.extend([-0.3 * y, -0.3 * x, 0.]);
            uv.extend(if row >= 9 {
                [0.; 2]
            } else {
                [if col <= 5 { x } else { 1. - x }, y]
            });
        }
    }
    let position = fixture.floats("VEC3", &positions);
    fixture.extent(position, [0.; 3], [1., 1., 0.1]);
    fixture.attribute("POSITION", position);
    let delta = fixture.floats("VEC3", &deltas);
    fixture.extent(delta, [-0.05, 0., 0.], [0., 0.1, 0.3]);
    fixture.target("POSITION", delta);
    let uv = fixture.floats("VEC2", &uv);
    fixture.attribute("TEXCOORD_2", uv);
    let unused_uv = fixture.floats("VEC2", &[0.; 242]);
    fixture.attribute("TEXCOORD_0", unused_uv);
    if authored_normals {
        let normal = fixture.floats("VEC3", &normals);
        fixture.attribute("NORMAL", normal);
        let delta = fixture.floats("VEC3", &normal_deltas);
        fixture.target("NORMAL", delta);
    }
    let joints = fixture.raw("VEC4", 5121, 121, &[0; 484]);
    fixture.attribute("JOINTS_0", joints);
    let influences = fixture.floats("VEC4", &[1., 0., 0., 0.].repeat(121));
    fixture.attribute("WEIGHTS_0", influences);
    let mut faces = Vec::new();
    for row in 0..10 {
        for col in 0..10 {
            let a = row * 11 + col;
            faces.extend([[a, a + 1, a + 12], [a, a + 12, a + 11]]);
        }
    }
    let mut indices = Vec::<u16>::new();
    for slot in 0..faces.len() {
        indices.extend(faces[slot * 31 % faces.len()]);
    }
    if authored_normals {
        indices.extend([0, 1, 1].repeat(65));
    }
    let index = fixture.raw(
        "SCALAR",
        5123,
        indices.len(),
        &indices
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    fixture.json["meshes"][0]["primitives"][0]["indices"] = json!(index);
    fixture.json["meshes"][0]["primitives"][0]["material"] = json!(0);
    fixture.json["materials"] = json!([{
        "normalTexture":{"index":0,"texCoord":2}, "doubleSided":true
    }]);
    fixture.json["textures"] = json!([{"source":0}]);
    fixture.json["images"] = json!([{"uri":"normal.png","mimeType":"image/png"}]);
    fixture.json["skins"] = json!([{"joints":[1]}]);
    fixture
}
