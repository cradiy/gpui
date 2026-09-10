use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fs::File,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use gpui_3d::SceneGraph;
use gpui_3d_gltf::{AnimationOptions, Document, ImageDecodeLimits, Limits, SceneOptions};

const USAGE: &str =
    "Usage: inspect <asset.gltf|asset.glb> [--scene INDEX] [--animation INDEX] [--time SECONDS]...";

struct Options {
    path: PathBuf,
    scene: Option<usize>,
    animation: Option<usize>,
    times: Vec<Duration>,
}

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let Some(path) = arguments.next() else {
        bail!(USAGE);
    };
    if path == "--help" || path == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    let mut options = Options {
        path: path.into(),
        scene: None,
        animation: None,
        times: Vec::new(),
    };
    while let Some(argument) = arguments.next() {
        let name = argument.to_str().context("option name must be UTF-8")?;
        ensure!(
            matches!(name, "--scene" | "--animation" | "--time"),
            "unknown option {name}; {USAGE}"
        );
        let value = arguments
            .next()
            .with_context(|| format!("missing value for {name}"))?;
        let value = value.to_str().context("option value must be UTF-8")?;
        match name {
            "--scene" => {
                ensure!(options.scene.is_none(), "--scene may be specified once");
                options.scene = Some(value.parse().context("invalid scene index")?);
            }
            "--animation" => {
                ensure!(
                    options.animation.is_none(),
                    "--animation may be specified once"
                );
                options.animation = Some(value.parse().context("invalid animation index")?);
            }
            "--time" => options.times.push(
                Duration::try_from_secs_f64(value.parse().context("invalid time")?)
                    .context("time must be finite, nonnegative, and representable")?,
            ),
            _ => unreachable!(),
        }
    }
    if options.times.is_empty() {
        options.times.push(Duration::ZERO);
    }
    inspect(options)
}

fn inspect(options: Options) -> Result<()> {
    let limits = Limits::default();
    let path = options.path.canonicalize().context("asset path")?;
    let root = path.parent().context("asset has no parent directory")?;
    let bytes = read_bounded(&path, limits.document_bytes)?;
    let document = Document::from_slice(&bytes, limits)?;
    println!("Asset: {}", path.display());
    println!(
        "Scenes: {} · Meshes: {} · Animations: {}",
        document.gltf().scenes().len(),
        document.gltf().meshes().len(),
        document.gltf().animations().len()
    );
    let resources =
        document.prepare(|uri| read_bounded(&resource_path(root, uri)?, limits.resource_bytes))?;
    let definition = resources.scene(options.scene, SceneOptions::default())?;
    let image_count = definition
        .materials()
        .iter()
        .flat_map(|material| {
            material
                .textures()
                .iter()
                .map(|texture| texture.image_index())
        })
        .collect::<HashSet<_>>()
        .len();
    println!(
        "Scene {} · Materials: {} · Images: {} · Encoded bytes: {}",
        definition.index(),
        definition.materials().len(),
        image_count,
        resources.resource_bytes()
    );
    let asset = definition.decode_images(ImageDecodeLimits::default())?;
    let clip = options
        .animation
        .map(|index| resources.animation(index, AnimationOptions::default()))
        .transpose()?;
    if let Some(clip) = &clip {
        println!(
            "Animation {} {:?}: {:.6}–{:.6} s · Node tracks: {}",
            clip.index(),
            clip.name(),
            clip.start().as_secs_f64(),
            clip.end().as_secs_f64(),
            clip.nodes().len()
        );
    }
    println!(
        "Nodes: {} · Primitives: {} · Skin bindings: {} · Morph bindings: {}",
        asset.nodes().len(),
        asset.primitives().len(),
        asset.skins().len(),
        asset.morphs().len()
    );
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree())?;
    let bindings: HashMap<_, _> = asset
        .nodes()
        .iter()
        .map(|node| (node.index, instance.node(node.handle).unwrap()))
        .collect();
    let mut fingerprints = HashMap::new();
    for time in options.times {
        let mut transforms = Vec::new();
        let mut weights = Vec::new();
        let mut skipped = 0;
        if let Some(clip) = &clip {
            for animation in clip.nodes() {
                let Some(&node) = bindings.get(&animation.node_index()) else {
                    skipped += 1;
                    continue;
                };
                if let Some(track) = animation.transform() {
                    transforms.push((node, track.sample_transform(time)?));
                }
                if let Some(track) = animation.weights() {
                    weights.push((node, track.sample(time)?));
                }
            }
        }
        let poses = graph.evaluate_with_transforms(transforms.iter().copied())?;
        let replacements = asset.deform(&instance, &poses, &weights)?;
        let mut fingerprint = DefaultHasher::new();
        let mut vertices = 0;
        for (node, mesh) in replacements {
            vertices += mesh.vertex_count();
            for vertex in mesh.vertices() {
                for value in vertex
                    .position
                    .into_iter()
                    .chain(vertex.normal)
                    .chain(vertex.uv)
                {
                    value.to_bits().hash(&mut fingerprint);
                }
            }
            for tangent in mesh.tangents().into_iter().flatten() {
                tangent.map(f32::to_bits).hash(&mut fingerprint);
            }
            mesh.indices().hash(&mut fingerprint);
            graph.set_mesh(node, mesh)?;
        }
        let evaluated = graph.evaluate_with_transforms(transforms)?;
        for node in evaluated.nodes() {
            for value in node.world.matrix().into_iter().flatten() {
                value.to_bits().hash(&mut fingerprint);
            }
        }
        evaluated.prepare_spatial_index();
        let fingerprint = fingerprint.finish();
        if let Some(previous) = fingerprints.insert(time, fingerprint) {
            ensure!(
                previous == fingerprint,
                "repeated sampling at {time:?} changed the pose/geometry fingerprint"
            );
        }
        println!(
            "Time {:.6} s · Deformed vertices: {vertices} · Skipped node tracks: {skipped} · Fingerprint: {fingerprint:016x}",
            time.as_secs_f64()
        );
        println!("Bounds: {:?}", evaluated.bounds());
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "{} is not a regular file",
        path.display()
    );
    ensure!(
        metadata.len() <= limit as u64,
        "{} exceeds {limit} bytes",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "{} grew beyond {limit} bytes",
        path.display()
    );
    Ok(bytes)
}

fn resource_path(root: &Path, uri: &str) -> Result<PathBuf> {
    ensure!(
        !uri.starts_with('/') && !uri.contains(['\\', ':']),
        "only relative local resource URIs are accepted: {uri}"
    );
    let base = url::Url::from_directory_path(root)
        .map_err(|_| anyhow::anyhow!("invalid asset directory"))?;
    let resolved = base.join(uri).context("resource URI")?;
    ensure!(
        resolved.query().is_none() && resolved.fragment().is_none(),
        "resource query/fragment is unsupported: {uri}"
    );
    let path = resolved
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("resource is not a local file: {uri}"))?
        .canonicalize()
        .with_context(|| format!("resource {uri}"))?;
    ensure!(
        path.starts_with(root),
        "resource leaves the asset directory: {uri}"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_resolution_decodes_paths_and_rejects_directory_escapes() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("asset");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("color map.bin"), [1, 2, 3]).unwrap();
        std::fs::write(temporary.path().join("outside.bin"), [4]).unwrap();
        let root = root.canonicalize().unwrap();
        let path = resource_path(&root, "color%20map.bin").unwrap();
        assert_eq!(read_bounded(&path, 3).unwrap(), [1, 2, 3]);
        assert!(read_bounded(&path, 2).is_err());
        for uri in [
            "../outside.bin",
            "%2e%2e/outside.bin",
            "https://example.invalid/a",
            "file:///tmp/a",
            "//example.invalid/a",
            "color%20map.bin?x=1",
            "color%20map.bin#part",
            "..\\outside.bin",
        ] {
            assert!(resource_path(&root, uri).is_err(), "{uri}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temporary.path().join("outside.bin"), root.join("link.bin"))
                .unwrap();
            assert!(resource_path(&root, "link.bin").is_err());
        }
    }
}
