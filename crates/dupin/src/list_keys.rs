//! `up_key` and `down_key`: chords the panel's lists take as ↑ and ↓, for keyboards without
//! arrow keys. They act before the panel's own keys, so they may only name chords nothing in
//! the panel, its search field or a global shortcut uses.

use dupin_core::Config;
use rune::{Key, KeyEvent, PhysicalKeyCode, Shortcut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Up,
    Down,
}

impl ListKey {
    pub const ALL: [Self; 2] = [Self::Up, Self::Down];

    pub fn accelerator(self, config: &Config) -> &str {
        match self {
            Self::Up => &config.up_key,
            Self::Down => &config.down_key,
        }
    }

    pub fn set(self, config: &mut Config, shortcut: Option<&Shortcut>) {
        let field = match self {
            Self::Up => &mut config.up_key,
            Self::Down => &mut config.down_key,
        };
        *field = shortcut.map(ToString::to_string).unwrap_or_default();
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Up => "Up key",
            Self::Down => "Down key",
        }
    }

    fn config_key(self) -> &'static str {
        match self {
            Self::Up => "up_key",
            Self::Down => "down_key",
        }
    }

    fn arrow(self) -> Key {
        match self {
            Self::Up => Key::Up,
            Self::Down => Key::Down,
        }
    }

    pub fn shortcut(self, config: &Config) -> Option<Shortcut> {
        crate::hotkey::parse(self.accelerator(config)).ok().flatten()
    }
}

/// The list key `shortcut` is in `config`, if it is one.
pub fn holding(config: &Config, shortcut: &Shortcut) -> Option<ListKey> {
    ListKey::ALL
        .into_iter()
        .find(|key| key.shortcut(config).as_ref() == Some(shortcut))
}

/// Why `shortcut` can't be `key`, in words: the panel or its field already uses it, or a
/// global shortcut or the other list key does.
pub fn problem(config: &Config, key: ListKey, shortcut: &Shortcut) -> Option<String> {
    let keys = shortcut.keycaps().join("");
    if shortcut.require_modifier().is_err() {
        return Some(format!("{keys} types text; add ⌃ or ⌥."));
    }
    if let Some(reserved) = reserved(shortcut) {
        return Some(format!("{keys} {reserved}."));
    }
    let mut others = config.clone();
    key.set(&mut others, None);
    crate::hotkey::holder(&others, shortcut, None).map(|holder| format!("{keys} is assigned to {holder}."))
}

/// What the panel or its search field does with `shortcut`, if it does anything.
fn reserved(shortcut: &Shortcut) -> Option<&'static str> {
    use PhysicalKeyCode::*;
    let m = shortcut.modifiers();
    match shortcut.physical_key() {
        ArrowUp | ArrowDown | ArrowLeft | ArrowRight | Home | End | PageUp | PageDown => {
            Some("already moves through the lists or the field")
        }
        Enter | Escape | Tab | Space | Backspace | Delete => Some("is already one of the panel's keys"),
        _ if m.cmd => Some("is left to the panel's and the field's ⌘ commands"),
        KeyK if m.ctrl => Some("already shows a result's actions"),
        KeyA | KeyE | KeyB | KeyF | KeyD | KeyH if m.ctrl && !m.alt => Some("already edits the search field"),
        _ => None,
    }
}

/// Whether `config`'s list keys are usable: each a chord, neither one the panel, its field, a
/// global shortcut or the other already uses.
pub fn check(config: &Config) -> Result<(), String> {
    for key in ListKey::ALL {
        let accelerator = key.accelerator(config);
        let Some(shortcut) = crate::hotkey::parse(accelerator)
            .map_err(|_| format!("`{}` {accelerator:?} isn't a key chord", key.config_key()))?
        else {
            continue;
        };
        if let Some(problem) = problem(config, key, &shortcut) {
            return Err(format!("`{}`: {problem}", key.config_key()));
        }
    }
    Ok(())
}

/// ↑ or ↓ for `ev` when it is one of `config`'s list keys. They are recorded as the key's
/// position, as global shortcuts are, so they match by the physical key, not the character it
/// types on the current layout.
pub fn arrow(config: &Config, ev: &KeyEvent, physical: Option<PhysicalKeyCode>) -> Option<Key> {
    let physical = physical?;
    ListKey::ALL.into_iter().find_map(|key| {
        let shortcut = key.shortcut(config)?;
        (shortcut.physical_key() == physical && shortcut.modifiers() == ev.mods).then(|| key.arrow())
    })
}

#[cfg(test)]
mod tests {
    use rune::Mods;

    use super::*;

    fn config(up: &str, down: &str) -> Config {
        Config {
            up_key: up.into(),
            down_key: down.into(),
            ..Config::default()
        }
    }

    #[test]
    fn list_keys_refuse_what_the_panel_field_or_a_hotkey_already_uses() {
        assert_eq!(check(&config("ctrl+p", "ctrl+n")), Ok(()));
        assert_eq!(check(&config("alt+i", "f2")), Ok(()));
        for taken in [
            "j",
            "shift+j",
            "ctrl+up",
            "alt+down",
            "ctrl+tab",
            "ctrl+enter",
            "cmd+j",
            "ctrl+k",
            "ctrl+shift+k",
            "ctrl+a",
            "ctrl+h",
            "cmd+space",
        ] {
            assert!(check(&config(taken, "")).is_err(), "{taken}");
            assert!(check(&config("", taken)).is_err(), "{taken}");
        }
        let error = check(&config("ctrl+n", "ctrl+n")).unwrap_err();
        assert!(
            error.starts_with("`up_key`: ⌃N is assigned to the Down key"),
            "{error}"
        );
        let bound = Config {
            bindings: Config::parse("[bindings.\"ctrl+alt+l\"]\ntarget = \"command\"\ncommand = \"true\"")
                .unwrap()
                .bindings,
            ..config("ctrl+alt+l", "")
        };
        assert!(check(&bound).unwrap_err().contains("a command binding"));
        assert!(
            check(&config("ctrl+", ""))
                .unwrap_err()
                .contains("isn't a key chord")
        );
    }

    #[test]
    fn list_keys_match_by_the_key_recorded_not_its_character() {
        let config = config("ctrl+p", "ctrl+y");
        let ev = |text: &str| KeyEvent {
            key: Key::Text(text.into()),
            mods: Mods {
                ctrl: true,
                cmd: !cfg!(target_os = "macos"),
                ..Mods::default()
            },
        };
        assert_eq!(
            arrow(&config, &ev("z"), Some(PhysicalKeyCode::KeyY)),
            Some(Key::Down)
        );
        assert_eq!(arrow(&config, &ev("y"), Some(PhysicalKeyCode::KeyZ)), None);
        assert_eq!(
            arrow(&config, &ev("p"), Some(PhysicalKeyCode::KeyP)),
            Some(Key::Up)
        );
        let plain = KeyEvent {
            key: Key::Text("p".into()),
            mods: Mods::default(),
        };
        assert_eq!(arrow(&config, &plain, Some(PhysicalKeyCode::KeyP)), None);
    }
}
