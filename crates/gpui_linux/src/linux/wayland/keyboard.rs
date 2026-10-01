use std::os::fd::OwnedFd;

use anyhow::{Context as _, bail};
use gpui::{Capslock, Keystroke, Modifiers};
use wayland_client::{WEnum, protocol::wl_keyboard::KeymapFormat};
use xkbcommon::xkb::{self, Keycode, Keysym};

use crate::linux::{capslock_from_xkb, keystroke_from_xkb, modifiers_from_xkb};

pub(super) fn load_keymap(
    context: &xkb::Context,
    format: WEnum<KeymapFormat>,
    fd: OwnedFd,
    size: u32,
) -> anyhow::Result<xkb::State> {
    if format != WEnum::Value(KeymapFormat::XkbV1) {
        bail!("Unsupported Wayland keymap format: {format:?}");
    }
    if size == 0 {
        bail!("Wayland keymap is empty");
    }
    // SAFETY: Wayland supplies an owned descriptor for the keymap mapping.
    let keymap = unsafe {
        xkb::Keymap::new_from_fd(
            context,
            fd,
            size as usize,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
    }
    .context("Failed to map Wayland keymap")?
    .context("Failed to parse Wayland keymap")?;
    Ok(xkb::State::new(&keymap))
}

pub(super) fn update_modifiers(
    keymap: Option<&mut xkb::State>,
    depressed: u32,
    latched: u32,
    locked: u32,
    group: u32,
) -> Option<(u32, Modifiers, Capslock)> {
    let keymap = keymap?;
    let old_layout = keymap.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE);
    keymap.update_mask(depressed, latched, locked, 0, 0, group);
    Some((
        old_layout,
        modifiers_from_xkb(keymap),
        capslock_from_xkb(keymap),
    ))
}

pub(super) fn translate_key(
    keymap: Option<&xkb::State>,
    modifiers: Modifiers,
    key: u32,
) -> Option<(Keycode, Keysym, Keystroke)> {
    let keymap = keymap?;
    let keycode = Keycode::from(key.checked_add(8)?);
    Some((
        keycode,
        keymap.key_get_one_sym(keycode),
        keystroke_from_xkb(keymap, modifiers, keycode),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, io::Write};

    const KEYMAP: &str = r#"xkb_keymap {
        xkb_keycodes "test" { minimum = 8; maximum = 255; <AC01> = 38; <LFSH> = 50; };
        xkb_types "test" {
            type "TWO_LEVEL" { modifiers = Shift; map[Shift] = Level2; };
        };
        xkb_compatibility "test" {};
        xkb_symbols "test" {
            key <AC01> { type = "TWO_LEVEL", [ a, A ] };
            key <LFSH> { [ Shift_L ] };
            modifier_map Shift { <LFSH> };
        };
    };"#;

    fn keymap_fd(text: &str) -> (OwnedFd, u32) {
        let path = std::env::temp_dir().join(format!("gpui-keymap-{}", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.write_all(&[0]).unwrap();
        (file.into(), text.len() as u32 + 1)
    }

    #[test]
    fn keyboard_events_recover_after_missing_or_invalid_keymap() {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let mut keymap = None;
        assert!(update_modifiers(keymap.as_mut(), 1, 0, 0, 0).is_none());
        assert!(translate_key(keymap.as_ref(), Modifiers::default(), 30).is_none());

        let (fd, size) = keymap_fd("invalid keymap");
        keymap = load_keymap(&context, WEnum::Value(KeymapFormat::XkbV1), fd, size).ok();
        assert!(update_modifiers(keymap.as_mut(), 0, 0, 0, 0).is_none());
        assert!(translate_key(keymap.as_ref(), Modifiers::default(), 30).is_none());

        let (fd, size) = keymap_fd(KEYMAP);
        keymap = Some(load_keymap(&context, WEnum::Value(KeymapFormat::XkbV1), fd, size).unwrap());
        let (_, modifiers, _) = update_modifiers(keymap.as_mut(), 0, 0, 0, 0).unwrap();
        let (_, _, key) = translate_key(keymap.as_ref(), modifiers, 30).unwrap();
        assert_eq!(key.key_char.as_deref(), Some("a"));

        let shift = keymap
            .as_ref()
            .unwrap()
            .get_keymap()
            .mod_get_index(xkb::MOD_NAME_SHIFT);
        let (_, modifiers, _) = update_modifiers(keymap.as_mut(), 1 << shift, 0, 0, 0).unwrap();
        assert!(modifiers.shift);
        let (_, _, key) = translate_key(keymap.as_ref(), modifiers, 30).unwrap();
        assert_eq!(key.key_char.as_deref(), Some("A"));
    }

    #[test]
    fn keyboard_keymap_rejects_unsupported_format_and_unmappable_fd() {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        for format in [WEnum::Value(KeymapFormat::NoKeymap), WEnum::Unknown(42)] {
            let (fd, size) = keymap_fd(KEYMAP);
            assert!(load_keymap(&context, format, fd, size).is_err());
        }
        let (fd, _) = keymap_fd(KEYMAP);
        assert!(load_keymap(&context, WEnum::Value(KeymapFormat::XkbV1), fd, 0).is_err());
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(
            load_keymap(
                &context,
                WEnum::Value(KeymapFormat::XkbV1),
                socket.into(),
                16
            )
            .is_err()
        );
    }
}
