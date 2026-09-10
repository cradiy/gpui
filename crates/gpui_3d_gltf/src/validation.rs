use anyhow::{Context, Result, ensure};
use gltf::{Accessor, accessor::Dimensions, buffer::View};

pub(crate) fn container(bytes: &[u8]) -> Result<()> {
    if !bytes.starts_with(b"glTF") {
        return Ok(());
    }
    let word = |offset: usize| -> Result<u32> {
        let value: [u8; 4] = bytes
            .get(offset..offset + 4)
            .context("GLB: truncated header")?
            .try_into()?;
        Ok(u32::from_le_bytes(value))
    };
    ensure!(word(4)? == 2, "GLB: expected version 2");
    ensure!(
        word(8)? as usize == bytes.len(),
        "GLB: header length does not match input length"
    );
    let mut offset = 12;
    let mut chunks = 0;
    while offset < bytes.len() {
        let length = word(offset)? as usize;
        let kind = word(offset + 4)?;
        ensure!(
            length.is_multiple_of(4),
            "GLB chunk {chunks}: length is not four-byte aligned"
        );
        if chunks == 0 {
            ensure!(kind == 0x4e4f534a, "GLB: first chunk must be JSON");
        }
        offset = offset
            .checked_add(8)
            .and_then(|offset| offset.checked_add(length))
            .context("GLB chunk length overflow")?;
        ensure!(
            offset <= bytes.len(),
            "GLB chunk {chunks}: range exceeds input"
        );
        chunks += 1;
    }
    ensure!(chunks > 0, "GLB: missing JSON chunk");
    Ok(())
}

pub(crate) fn layout(document: &gltf::Document) -> Result<()> {
    let asset = &document.as_json().asset;
    ensure!(asset.version == "2.0", "asset.version: expected glTF 2.0");
    ensure!(
        asset
            .min_version
            .as_deref()
            .is_none_or(|version| version == "2.0"),
        "asset.minVersion: unsupported version"
    );
    for view in document.views() {
        let end = view
            .offset()
            .checked_add(view.length())
            .with_context(|| format!("buffer view {}: byte range overflow", view.index()))?;
        ensure!(
            end <= view.buffer().length(),
            "buffer view {}: range exceeds buffer {} declared length",
            view.index(),
            view.buffer().index()
        );
        ensure!(
            view.length() > 0,
            "buffer view {}: zero byte length",
            view.index()
        );
        if let Some(stride) = document.as_json().buffer_views[view.index()].byte_stride {
            ensure!(
                (4..=252).contains(&stride.0) && stride.0.is_multiple_of(4),
                "buffer view {}: invalid byte stride",
                view.index()
            );
        }
    }
    for accessor in document.accessors() {
        accessor_layout(&accessor).with_context(|| format!("accessor {}", accessor.index()))?;
    }
    Ok(())
}

fn accessor_layout(accessor: &Accessor<'_>) -> Result<()> {
    let element = element_size(accessor);
    let component = accessor.data_type().size();
    ensure!(accessor.count() > 0, "zero element count");
    if let Some(view) = accessor.view() {
        range(
            &view,
            accessor.offset(),
            accessor.count(),
            element,
            component,
        )?;
    } else {
        ensure!(accessor.offset() == 0, "byte offset requires a buffer view");
    }
    if let Some(sparse) = accessor.sparse() {
        ensure!(
            sparse.count() > 0 && sparse.count() <= accessor.count(),
            "sparse count exceeds accessor count or is zero"
        );
        let indices = sparse.indices();
        let view = indices.view();
        ensure!(
            view.stride().is_none() && view.target().is_none(),
            "sparse indices must not have stride or target"
        );
        let size = indices.index_type().size();
        range(&view, indices.offset(), sparse.count(), size, size).context("sparse indices")?;
        let values = sparse.values();
        let view = values.view();
        ensure!(
            view.stride().is_none() && view.target().is_none(),
            "sparse values must not have stride or target"
        );
        range(&view, values.offset(), sparse.count(), element, component)
            .context("sparse values")?;
    }
    Ok(())
}

fn element_size(accessor: &Accessor<'_>) -> usize {
    let component = accessor.data_type().size();
    let columns = match accessor.dimensions() {
        Dimensions::Mat2 => 2,
        Dimensions::Mat3 => 3,
        Dimensions::Mat4 => 4,
        _ => return accessor.size(),
    };
    (columns * component).next_multiple_of(4) * columns
}

fn range(
    view: &View<'_>,
    offset: usize,
    count: usize,
    element: usize,
    component: usize,
) -> Result<()> {
    let stride = view.stride().unwrap_or(element);
    ensure!(
        stride >= element && stride.is_multiple_of(component),
        "invalid element stride"
    );
    ensure!(
        offset.is_multiple_of(component) && view.offset().is_multiple_of(component),
        "misaligned component offset"
    );
    let end = count
        .checked_sub(1)
        .and_then(|n| n.checked_mul(stride))
        .and_then(|n| n.checked_add(element))
        .and_then(|n| n.checked_add(offset))
        .context("accessor byte range overflow")?;
    ensure!(
        end <= view.length(),
        "byte range exceeds buffer view {} length",
        view.index()
    );
    Ok(())
}

pub(crate) fn sparse<'a>(
    document: &gltf::Document,
    buffer: impl Fn(usize) -> Option<&'a [u8]>,
) -> Result<()> {
    for accessor in document.accessors() {
        let Some(sparse) = accessor.sparse() else {
            continue;
        };
        let indices = sparse.indices();
        let view = indices.view();
        let start = view.offset() + indices.offset();
        let size = indices.index_type().size();
        let data = buffer(view.buffer().index()).context("missing sparse index buffer")?;
        let data = data
            .get(start..start + sparse.count() * size)
            .context("sparse index range exceeds loaded bytes")?;
        let mut previous = None;
        for (offset, bytes) in data.chunks_exact(size).enumerate() {
            let value = bytes
                .iter()
                .enumerate()
                .fold(0_u32, |value, (shift, &byte)| {
                    value | u32::from(byte) << (shift * 8)
                });
            ensure!(
                (value as usize) < accessor.count(),
                "accessor {} sparse index {}: {} exceeds element count {}",
                accessor.index(),
                offset,
                value,
                accessor.count()
            );
            ensure!(
                previous.is_none_or(|previous| previous < value),
                "accessor {} sparse index {}: indices must be strictly increasing",
                accessor.index(),
                offset
            );
            previous = Some(value);
        }
    }
    Ok(())
}
