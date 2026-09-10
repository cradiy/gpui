use anyhow::{Context, Result, bail, ensure};
use gltf::{
    Accessor, Semantic,
    accessor::{DataType, Dimensions, Item, Iter},
};

use crate::PreparedDocument;

pub(crate) fn format(
    accessor: &Accessor<'_>,
    semantic: &Semantic,
    morph: bool,
    quantized: bool,
) -> bool {
    let dimensions = match semantic {
        Semantic::Positions | Semantic::Normals => Dimensions::Vec3,
        Semantic::Tangents if morph => Dimensions::Vec3,
        Semantic::Tangents => Dimensions::Vec4,
        Semantic::TexCoords(_) => Dimensions::Vec2,
        _ => return false,
    };
    if accessor.dimensions() != dimensions {
        return false;
    }
    match accessor.data_type() {
        DataType::F32 => !accessor.normalized(),
        DataType::I8 | DataType::I16 if quantized => {
            matches!(semantic, Semantic::Positions | Semantic::TexCoords(_))
                || accessor.normalized()
        }
        DataType::U8 | DataType::U16 if !morph => match semantic {
            Semantic::Positions => quantized,
            Semantic::TexCoords(_) => quantized || accessor.normalized(),
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn alignment(accessor: &Accessor<'_>) -> Result<()> {
    if let Some(view) = accessor.view() {
        ensure!(
            accessor.offset().is_multiple_of(4)
                && view.offset().is_multiple_of(4)
                && view.stride().unwrap_or(accessor.size()).is_multiple_of(4),
            "accessor {} quantized vertex elements must be four-byte aligned",
            accessor.index()
        );
    }
    Ok(())
}

impl PreparedDocument {
    pub(crate) fn vector<const N: usize>(&self, accessor: &Accessor<'_>) -> Result<Vec<[f32; N]>>
    where
        [f32; N]: Item + Default,
        [i8; N]: Item,
        [u8; N]: Item,
        [i16; N]: Item,
        [u16; N]: Item,
    {
        ensure!(
            accessor.dimensions().multiplicity() == N,
            "vector dimension mismatch"
        );
        let normalized = accessor.normalized();
        macro_rules! decode {
            ($ty:ty, $convert:expr) => {
                crate::geometry::collect(
                    accessor,
                    Iter::<[$ty; N]>::new(accessor.clone(), |buffer| self.buffer(buffer.index()))
                        .map(|values| values.map(|value| value.map($convert))),
                )
            };
        }
        let values = match accessor.data_type() {
            DataType::F32 => decode!(f32, |v| v),
            DataType::I8 => decode!(i8, |v: i8| if normalized {
                (f32::from(v) / 127.).max(-1.)
            } else {
                f32::from(v)
            }),
            DataType::U8 => decode!(u8, |v: u8| if normalized {
                f32::from(v) / 255.
            } else {
                f32::from(v)
            }),
            DataType::I16 => decode!(i16, |v: i16| if normalized {
                (f32::from(v) / 32767.).max(-1.)
            } else {
                f32::from(v)
            }),
            DataType::U16 => decode!(u16, |v: u16| if normalized {
                f32::from(v) / 65535.
            } else {
                f32::from(v)
            }),
            _ => bail!("unsupported vector component type"),
        };
        values.with_context(|| format!("accessor {} vector data", accessor.index()))
    }
}
