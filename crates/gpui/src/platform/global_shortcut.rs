use std::{
    fmt,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{Keystroke, Platform, SharedString};

static NEXT_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);

/// Identifies one group of global shortcuts registered together.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlobalShortcutRegistrationId(u64);

impl GlobalShortcutRegistrationId {
    pub(crate) fn next() -> Self {
        Self(NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// Returns the numeric value of this identifier.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Describes a global keyboard shortcut requested by an application.
///
/// The operating system may reject or replace `preferred_trigger`. Use the
/// corresponding [`RegisteredGlobalShortcut`] to display the effective
/// trigger to users.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalShortcut {
    id: SharedString,
    description: SharedString,
    preferred_trigger: Keystroke,
}

impl GlobalShortcut {
    /// Creates a global shortcut descriptor.
    pub fn new(
        id: impl Into<SharedString>,
        description: impl Into<SharedString>,
        preferred_trigger: Keystroke,
    ) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            preferred_trigger,
        }
    }

    /// Returns the application-defined stable identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the user-readable description of the shortcut's action.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the application's preferred trigger.
    pub fn preferred_trigger(&self) -> &Keystroke {
        &self.preferred_trigger
    }
}

/// Describes a global shortcut accepted by the operating system.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredGlobalShortcut {
    id: SharedString,
    trigger_description: SharedString,
}

impl RegisteredGlobalShortcut {
    /// Creates a description of an accepted global shortcut.
    pub fn new(id: impl Into<SharedString>, trigger_description: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            trigger_description: trigger_description.into(),
        }
    }

    /// Returns the application-defined stable identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the platform-provided text describing the effective trigger.
    pub fn trigger_description(&self) -> &str {
        &self.trigger_description
    }
}

/// An event emitted by the global shortcut service.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GlobalShortcutEvent {
    /// A registered shortcut was activated.
    Activated {
        /// The registration group containing the shortcut.
        registration_id: GlobalShortcutRegistrationId,
        /// The application-defined shortcut identifier.
        shortcut_id: SharedString,
        /// An optional platform activation token. Wayland applications may
        /// use this value when activating a window in response to the event.
        activation_token: Option<SharedString>,
    },
    /// The operating system changed the shortcuts bound to a registration.
    ShortcutsChanged {
        /// The affected registration group.
        registration_id: GlobalShortcutRegistrationId,
        /// The complete set of effective shortcuts in the group.
        shortcuts: Vec<RegisteredGlobalShortcut>,
    },
}

impl GlobalShortcutEvent {
    /// Returns the registration group that emitted this event.
    pub fn registration_id(&self) -> GlobalShortcutRegistrationId {
        match self {
            Self::Activated {
                registration_id, ..
            }
            | Self::ShortcutsChanged {
                registration_id, ..
            } => *registration_id,
        }
    }
}

/// Owns a group of global shortcut registrations.
///
/// Dropping this value unregisters every shortcut in the group.
#[must_use]
pub struct GlobalShortcutRegistration {
    id: GlobalShortcutRegistrationId,
    shortcuts: Vec<RegisteredGlobalShortcut>,
    platform: Option<Rc<dyn Platform>>,
}

impl GlobalShortcutRegistration {
    pub(crate) fn new(
        id: GlobalShortcutRegistrationId,
        shortcuts: Vec<RegisteredGlobalShortcut>,
        platform: Rc<dyn Platform>,
    ) -> Self {
        Self {
            id,
            shortcuts,
            platform: Some(platform),
        }
    }

    /// Returns this registration group's identifier.
    pub fn id(&self) -> GlobalShortcutRegistrationId {
        self.id
    }

    /// Returns the shortcuts accepted by the operating system.
    pub fn shortcuts(&self) -> &[RegisteredGlobalShortcut] {
        &self.shortcuts
    }

    /// Unregisters all shortcuts in this group immediately.
    pub fn unregister(mut self) {
        self.unregister_inner();
    }

    fn unregister_inner(&mut self) {
        if let Some(platform) = self.platform.take() {
            platform.unregister_global_shortcuts(self.id);
        }
    }
}

impl Drop for GlobalShortcutRegistration {
    fn drop(&mut self) {
        self.unregister_inner();
    }
}

impl fmt::Debug for GlobalShortcutRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GlobalShortcutRegistration")
            .field("id", &self.id)
            .field("shortcuts", &self.shortcuts)
            .finish_non_exhaustive()
    }
}
