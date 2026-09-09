#![cfg(all(feature = "wgpu", not(target_family = "wasm")))]

use gpui::rgb;
use gpui_3d::{Camera, HeadlessRenderer, Material, Mesh, Node, Scene3dOutputConfig, SceneGraph};

#[test]
#[ignore = "requires a GPU adapter"]
fn rendered_ids_retain_node_identity_after_graph_edits() -> anyhow::Result<()> {
    let mut graph = SceneGraph::new();
    let node = graph.insert(
        None,
        Node::new()
            .id("panel")
            .mesh(Mesh::plane(), Material::color(rgb(0x80c0e0)).unlit(true)),
    )?;
    let mut renderer = HeadlessRenderer::new()?;
    let old = renderer.render(
        &graph.evaluate()?.scene(Camera::default()),
        Scene3dOutputConfig::new([65, 65]),
    )?;
    assert_eq!(old.object(1).unwrap().node, Some(node));
    graph.remove_subtree(node)?;
    let replacement = graph.insert(
        None,
        Node::new()
            .id("replacement")
            .mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
    )?;
    let new = renderer.render(
        &graph.evaluate()?.scene(Camera::default()),
        Scene3dOutputConfig::new([40, 30]),
    )?;
    assert_eq!(new.object(1).unwrap().node, Some(replacement));
    assert_ne!(node, replacement);
    drop(renderer);
    drop(graph);
    let mut read = old.readback()?;
    drop(old);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let pixels = loop {
        if let Some(pixels) = read.try_read()? {
            break pixels;
        }
        anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(pixels.object_at(32, 32).unwrap().node, Some(node));
    assert_eq!(pixels.object_at(32, 32).unwrap().id, Some("panel".into()));
    assert!(pixels.object_at(0, 0).is_none());
    assert!(pixels.object_at(65, 32).is_none());
    assert!(pixels.object(u32::MAX).is_none());
    Ok(())
}
