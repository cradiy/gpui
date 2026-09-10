use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use gpui_3d::SceneGraph;
use gpui_3d_gltf::{
    AnimationOptions, AnimationTargetPolicy, Document, ImageDecodeLimits, Limits, SceneOptions,
};

#[path = "support/files.rs"]
mod files;
use files::{read_bounded, resource_path};

const USAGE: &str = "Usage: inspect <asset.gltf|asset.glb> [--scene INDEX] [--animation INDEX] [--time SECONDS]... [--weights NODE:W0,W1,...]...";

struct Options {
    path: PathBuf,
    scene: Option<usize>,
    animation: Option<usize>,
    times: Vec<Duration>,
    weights: Vec<(usize, Vec<f32>)>,
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
        weights: Vec::new(),
    };
    while let Some(argument) = arguments.next() {
        let name = argument.to_str().context("option name must be UTF-8")?;
        ensure!(
            matches!(name, "--scene" | "--animation" | "--time" | "--weights"),
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
            "--weights" => {
                let weights = parse_weights(value)?;
                ensure!(
                    options.weights.iter().all(|(node, _)| *node != weights.0),
                    "duplicate weight override for node {}",
                    weights.0
                );
                options.weights.push(weights);
            }
            _ => unreachable!(),
        }
    }
    if options.times.is_empty() {
        options.times.push(Duration::ZERO);
    }
    inspect(options)
}

fn parse_weights(value: &str) -> Result<(usize, Vec<f32>)> {
    let (node, weights) = value
        .split_once(':')
        .context("weights require NODE:W0,W1,...")?;
    let node = node.parse().context("invalid weight node index")?;
    let weights = weights
        .split(',')
        .map(|value| {
            let weight: f32 = value.parse().context("invalid morph weight")?;
            ensure!(weight.is_finite(), "morph weights must be finite");
            Ok(weight)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((node, weights))
}

fn hash_mesh(mesh: &gpui_3d::Mesh, fingerprint: &mut impl Hasher) {
    mesh.vertex_count().hash(fingerprint);
    for vertex in mesh.vertices() {
        for value in vertex
            .position
            .into_iter()
            .chain(vertex.normal)
            .chain(vertex.uv)
        {
            value.to_bits().hash(fingerprint);
        }
    }
    mesh.tangent_uv_set().hash(fingerprint);
    for tangent in mesh.tangents().into_iter().flatten() {
        tangent.map(f32::to_bits).hash(fingerprint);
    }
    mesh.uv_sets().count().hash(fingerprint);
    for set in mesh.uv_sets().skip(1) {
        set.hash(fingerprint);
        for vertex in 0..mesh.vertex_count() {
            mesh.uv_at(set, vertex)
                .unwrap()
                .map(f32::to_bits)
                .hash(fingerprint);
        }
    }
    mesh.vertex_colors().is_some().hash(fingerprint);
    for color in mesh.vertex_colors().into_iter().flatten() {
        color.map(f32::to_bits).hash(fingerprint);
    }
    mesh.indices().hash(fingerprint);
}

fn inspect(options: Options) -> Result<()> {
    let limits = Limits::default();
    let path = options.path.canonicalize().context("asset path")?;
    let root = path.parent().context("asset has no parent directory")?;
    let bytes = read_bounded(&path, limits.document_bytes)?;
    let document = Document::from_slice(&bytes, limits)?;
    println!("Asset: {}", path.display());
    for diagnostic in document.diagnostics() {
        println!("Import: {diagnostic}");
    }
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
    for geometry in definition.geometries() {
        let repairs = geometry.tangent_repairs();
        if !repairs.is_empty() {
            let orthonormal = repairs
                .iter()
                .filter(|repair| repair.kind == gpui_3d::TangentRepairKind::OrthonormalBasis)
                .count();
            println!(
                "Mesh {} primitive {} · Tangent repairs: {} derivative, {} orthonormal · First corners: {:?}",
                geometry.mesh_index(),
                geometry.primitive_index(),
                repairs.len() - orthonormal,
                orthonormal,
                &repairs[..repairs.len().min(8)]
            );
        }
    }
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
    let skinned: HashSet<_> = asset.skins().iter().map(|skin| skin.primitive()).collect();
    let combined = asset
        .morphs()
        .iter()
        .filter(|morph| skinned.contains(&morph.primitive()))
        .count();
    println!(
        "Nodes: {} · Primitives: {} · Skin bindings: {} · Morph bindings: {} · Combined primitives: {combined}",
        asset.nodes().len(),
        asset.primitives().len(),
        asset.skins().len(),
        asset.morphs().len()
    );
    let mut graph = SceneGraph::new();
    let instance = asset.instantiate(&mut graph, None)?;
    let animation = clip
        .as_ref()
        .map(|clip| clip.bind(&instance, AnimationTargetPolicy::SkipMissing))
        .transpose()?;
    let overrides: Vec<_> = options
        .weights
        .iter()
        .map(|(index, weights)| {
            let node = instance
                .node(*index)
                .with_context(|| format!("weight node {index} is outside the selected scene"))?;
            let morph = asset
                .morphs()
                .iter()
                .find(|morph| morph.node_index() == *index)
                .with_context(|| format!("weight node {index} has no morph targets"))?;
            ensure!(
                weights.len() == morph.default_weights().len(),
                "weight node {index} requires {} weights, received {}",
                morph.default_weights().len(),
                weights.len()
            );
            Ok((node, weights.clone()))
        })
        .collect::<Result<_>>()?;
    let mut fingerprints = HashMap::new();
    for time in options.times {
        let (pose, mut weights) = animation
            .as_ref()
            .map(|binding| binding.sample(time))
            .transpose()?
            .unwrap_or_default()
            .into_parts();
        let skipped = animation
            .as_ref()
            .map_or(0, |binding| binding.missing_nodes().len());
        for (node, values) in &overrides {
            if let Some((_, animated)) = weights.iter_mut().find(|(target, _)| target == node) {
                animated.clone_from(values);
            } else {
                weights.push((*node, values.clone()));
            }
        }
        let poses = graph.evaluate_with_transforms(pose.transforms())?;
        let replacements = instance.deform(&poses, &weights)?;
        let mut fingerprint = DefaultHasher::new();
        let mut vertices = 0;
        for (_, mesh) in &replacements {
            vertices += mesh.vertex_count();
            hash_mesh(mesh, &mut fingerprint);
        }
        let evaluated = graph.evaluate_with_overrides(pose.transforms(), replacements)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_overrides_accept_signed_values_and_reject_invalid_arguments() {
        assert_eq!(
            parse_weights("7:-0.25,1.5,0").unwrap(),
            (7, vec![-0.25, 1.5, 0.])
        );
        for value in [
            "", "7", "-1:0", "node:1", "7:", "7:1,", "7:NaN", "7:inf", "7:1e100", "7:1:2",
        ] {
            assert!(parse_weights(value).is_err(), "{value}");
        }
    }

    #[test]
    fn geometry_fingerprints_include_additional_attributes_and_tangent_associations() {
        let hash = |mesh: &gpui_3d::Mesh| {
            let mut hasher = DefaultHasher::new();
            hash_mesh(mesh, &mut hasher);
            hasher.finish()
        };
        let base = gpui_3d::Mesh::plane();
        let uv = base
            .with_uv_set(7, base.vertices().iter().map(|v| v.uv).collect())
            .unwrap();
        let tangent = uv
            .with_tangents_for_uv_set(7, uv.tangents().unwrap().to_vec())
            .unwrap();
        let colored = tangent
            .with_vertex_colors(vec![[0.2, 0.4, 0.8, 0.5]; base.vertex_count()])
            .unwrap();
        let recolored = colored
            .with_vertex_colors(vec![[0.2, 0.4, 0.8, 0.75]; base.vertex_count()])
            .unwrap();
        let moved = uv
            .with_uv_set(7, vec![[0.25, 0.5]; base.vertex_count()])
            .unwrap();
        let fingerprints: HashSet<_> = [&base, &uv, &tangent, &colored, &recolored, &moved]
            .into_iter()
            .map(hash)
            .collect();
        assert_eq!(fingerprints.len(), 6);
        assert_eq!(hash(&colored), hash(&colored.clone()));
    }

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
