//! Configurable hotkeys: local ones (window focused) handled through egui input, global ones
//! (system-wide, including media keys) registered with the OS through `global-hotkey`.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use egui::Key;
use global_hotkey::hotkey::{Code, HotKey, Modifiers as HkMods};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    PlayPause,
    Pause,
    Stop,
    Prev,
    Next,
    SeekBack,
    SeekForward,
    VolumeUp,
    VolumeDown,
    Mute,
    Shuffle,
    Repeat,
    Bookmark,
    Record,
    Search,
    SelectAll,
    Remove,
    Queue,
    PlaySelected,
    MoveUp,
    MoveDown,
}

impl Action {
    pub const ALL: [Action; 21] = [
        Action::PlayPause,
        Action::Pause,
        Action::Stop,
        Action::Prev,
        Action::Next,
        Action::SeekBack,
        Action::SeekForward,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::Mute,
        Action::Shuffle,
        Action::Repeat,
        Action::Bookmark,
        Action::Record,
        Action::Search,
        Action::SelectAll,
        Action::Remove,
        Action::Queue,
        Action::PlaySelected,
        Action::MoveUp,
        Action::MoveDown,
    ];

    pub fn index(self) -> usize {
        Action::ALL.iter().position(|a| *a == self).unwrap_or(0)
    }

    /// Whether holding the key down repeats the action.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Action::SeekBack | Action::SeekForward | Action::VolumeUp | Action::VolumeDown | Action::MoveUp | Action::MoveDown
        )
    }

    /// Actions that make sense while the window is not focused.
    pub fn can_be_global(self) -> bool {
        !matches!(
            self,
            Action::Search | Action::SelectAll | Action::Remove | Action::Queue | Action::PlaySelected | Action::MoveUp | Action::MoveDown
        )
    }
}

/// The hotkey editor is waiting for a key for this binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub action: Action,
    pub global: bool,
    /// Which local shortcut to replace; `None` adds a new one.
    pub slot: Option<usize>,
}

/// A key with modifiers. `ctrl` means Cmd on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Shortcut {
    pub const fn new(key: Key) -> Self {
        Self { ctrl: false, alt: false, shift: false, key }
    }
    pub const fn ctrl(key: Key) -> Self {
        Self { ctrl: true, alt: false, shift: false, key }
    }
    pub const fn alt(key: Key) -> Self {
        Self { ctrl: false, alt: true, shift: false, key }
    }
    pub const fn ctrl_alt(key: Key) -> Self {
        Self { ctrl: true, alt: true, shift: false, key }
    }

    pub fn from_event(key: Key, m: egui::Modifiers) -> Self {
        Self { ctrl: m.command || m.ctrl, alt: m.alt, shift: m.shift, key }
    }

    /// The same combination for the OS hotkey API, if the key has a scan code there.
    pub fn to_global(self) -> Option<HotKey> {
        let mut mods = HkMods::empty();
        if self.ctrl {
            mods |= if cfg!(target_os = "macos") { HkMods::META } else { HkMods::CONTROL };
        }
        if self.alt {
            mods |= HkMods::ALT;
        }
        if self.shift {
            mods |= HkMods::SHIFT;
        }
        Some(HotKey::new(Some(mods), key_code(self.key)?))
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str(if cfg!(target_os = "macos") { "Cmd+" } else { "Ctrl+" })?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(self.key.name())
    }
}

impl FromStr for Shortcut {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut sc = Shortcut::new(Key::Space);
        let mut key = None;
        // "Ctrl++" style is not produced by Display (Plus is spelled out), so '+' is a separator.
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => sc.ctrl = true,
                "alt" | "option" => sc.alt = true,
                "shift" => sc.shift = true,
                _ => key = Some(Key::from_name(part).ok_or_else(|| format!("unknown key: {part}"))?),
            }
        }
        sc.key = key.ok_or_else(|| format!("no key in {s:?}"))?;
        Ok(sc)
    }
}

impl Serialize for Shortcut {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Shortcut {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Shift, Ctrl, Alt and Super on their own are not shortcuts.
pub fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// Map an egui key to a physical key code for the OS hotkey API.
fn key_code(key: Key) -> Option<Code> {
    let name = key.name();
    let candidate = match key {
        Key::Equals | Key::Plus => "Equal".to_string(),
        Key::OpenBracket => "BracketLeft".into(),
        Key::CloseBracket => "BracketRight".into(),
        Key::Backtick => "Backquote".into(),
        Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight => format!("Arrow{name}"),
        _ if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() => format!("Key{name}"),
        _ if name.len() == 1 && name.as_bytes()[0].is_ascii_digit() => format!("Digit{name}"),
        _ => name.to_string(),
    };
    Code::from_str(&candidate).ok()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySettings {
    pub local: BTreeMap<Action, Vec<Shortcut>>,
    pub global: BTreeMap<Action, Shortcut>,
    pub global_enabled: bool,
    /// Play/pause, stop, next and previous on multimedia keyboards.
    pub media_keys: bool,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        use Action as A;
        let local: BTreeMap<Action, Vec<Shortcut>> = [
            (A::PlayPause, vec![Shortcut::new(Key::Space), Shortcut::new(Key::X)]),
            (A::Pause, vec![Shortcut::new(Key::C)]),
            (A::Stop, vec![Shortcut::new(Key::V)]),
            (A::Prev, vec![Shortcut::new(Key::Z)]),
            (A::Next, vec![Shortcut::new(Key::B)]),
            (A::SeekBack, vec![Shortcut::new(Key::ArrowLeft)]),
            (A::SeekForward, vec![Shortcut::new(Key::ArrowRight)]),
            (A::VolumeUp, vec![Shortcut::ctrl(Key::ArrowUp)]),
            (A::VolumeDown, vec![Shortcut::ctrl(Key::ArrowDown)]),
            (A::Mute, vec![Shortcut::ctrl(Key::M)]),
            (A::Shuffle, vec![Shortcut::ctrl(Key::H)]),
            (A::Repeat, vec![Shortcut::ctrl(Key::T)]),
            (A::Bookmark, vec![Shortcut::ctrl(Key::B)]),
            (A::Record, vec![Shortcut::ctrl(Key::R)]),
            (A::Search, vec![Shortcut::ctrl(Key::F)]),
            (A::SelectAll, vec![Shortcut::ctrl(Key::A)]),
            (A::Remove, vec![Shortcut::new(Key::Delete)]),
            (A::Queue, vec![Shortcut::new(Key::Q)]),
            (A::PlaySelected, vec![Shortcut::new(Key::Enter)]),
            (A::MoveUp, vec![Shortcut::alt(Key::ArrowUp)]),
            (A::MoveDown, vec![Shortcut::alt(Key::ArrowDown)]),
        ]
        .into_iter()
        .collect();
        let global = [
            (A::PlayPause, Shortcut::ctrl_alt(Key::Home)),
            (A::Stop, Shortcut::ctrl_alt(Key::End)),
            (A::Prev, Shortcut::ctrl_alt(Key::PageUp)),
            (A::Next, Shortcut::ctrl_alt(Key::PageDown)),
            (A::VolumeUp, Shortcut::ctrl_alt(Key::ArrowUp)),
            (A::VolumeDown, Shortcut::ctrl_alt(Key::ArrowDown)),
        ]
        .into_iter()
        .collect();
        Self { local, global, global_enabled: true, media_keys: true }
    }
}

impl HotkeySettings {
    /// Fill in actions added in newer versions, keeping whatever the user changed.
    pub fn complete(&mut self) {
        let defaults = HotkeySettings::default();
        for a in Action::ALL {
            if let Some(d) = defaults.local.get(&a) {
                self.local.entry(a).or_insert_with(|| d.clone());
            }
        }
    }

    pub fn local_action(&self, sc: Shortcut) -> Option<Action> {
        self.local.iter().find(|(_, v)| v.contains(&sc)).map(|(a, _)| *a)
    }

    /// Other action already using this local shortcut.
    pub fn local_conflict(&self, sc: Shortcut, except: Action) -> Option<Action> {
        self.local.iter().find(|(a, v)| **a != except && v.contains(&sc)).map(|(a, _)| *a)
    }

    pub fn global_conflict(&self, sc: Shortcut, except: Action) -> Option<Action> {
        self.global.iter().find(|(a, v)| **a != except && **v == sc).map(|(a, _)| *a)
    }
}

/// System-wide hotkeys registered with the OS.
pub struct GlobalHotkeys {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<(HotKey, Action)>,
    /// Shortcuts the OS refused (usually taken by another program).
    pub failed: Vec<String>,
    pub unavailable: Option<String>,
}

impl GlobalHotkeys {
    /// Must be created on the main thread (required on macOS and Windows).
    pub fn new(repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("global hotkeys unavailable: {e}");
                None
            }
        };
        // Wake the UI so events are handled even when the window is idle or minimized.
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            let _ = EVENTS.0.send(e);
            repaint();
        }));
        let unavailable = manager.is_none().then(|| "global hotkeys are not supported here".to_string());
        Self { manager, registered: Vec::new(), failed: Vec::new(), unavailable }
    }

    /// Replace all registrations with the ones from `settings`.
    pub fn apply(&mut self, settings: &HotkeySettings) {
        let Some(m) = &self.manager else { return };
        for (hk, _) in self.registered.drain(..) {
            let _ = m.unregister(hk);
        }
        self.failed.clear();
        let mut wanted: Vec<(HotKey, Action, String)> = Vec::new();
        if settings.global_enabled {
            for (a, sc) in &settings.global {
                match sc.to_global() {
                    Some(hk) => wanted.push((hk, *a, sc.to_string())),
                    None => self.failed.push(sc.to_string()),
                }
            }
        }
        if settings.media_keys {
            for (code, a) in [
                (Code::MediaPlayPause, Action::PlayPause),
                (Code::MediaPlay, Action::PlayPause),
                (Code::MediaStop, Action::Stop),
                (Code::MediaTrackNext, Action::Next),
                (Code::MediaTrackPrevious, Action::Prev),
            ] {
                // Not every platform knows every media key; silently skip those it lacks.
                wanted.push((HotKey::new(None, code), a, String::new()));
            }
        }
        for (hk, a, label) in wanted {
            if self.registered.iter().any(|(r, _)| r.id() == hk.id()) {
                continue;
            }
            match m.register(hk) {
                Ok(()) => self.registered.push((hk, a)),
                Err(e) => {
                    log::debug!("global hotkey {hk} not registered: {e}");
                    if !label.is_empty() {
                        self.failed.push(label);
                    }
                }
            }
        }
    }

    /// Actions triggered since the last call.
    pub fn poll(&self) -> Vec<Action> {
        EVENTS
            .1
            .try_iter()
            .filter(|e| e.state == HotKeyState::Pressed)
            .filter_map(|e| self.registered.iter().find(|(hk, _)| hk.id() == e.id).map(|(_, a)| *a))
            .collect()
    }
}

type EventChannel = (crossbeam_channel::Sender<GlobalHotKeyEvent>, crossbeam_channel::Receiver<GlobalHotKeyEvent>);
static EVENTS: std::sync::LazyLock<EventChannel> = std::sync::LazyLock::new(crossbeam_channel::unbounded);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_round_trip() {
        let sc = Shortcut { ctrl: true, alt: false, shift: true, key: Key::ArrowUp };
        let s = sc.to_string();
        assert_eq!(s.parse::<Shortcut>().unwrap(), sc);
        assert_eq!("Alt+Plus".parse::<Shortcut>().unwrap(), Shortcut::alt(Key::Plus));
        assert!("Ctrl+".parse::<Shortcut>().is_err());
    }

    #[test]
    fn maps_keys_to_codes() {
        assert_eq!(key_code(Key::A), Some(Code::KeyA));
        assert_eq!(key_code(Key::Num7), Some(Code::Digit7));
        assert_eq!(key_code(Key::ArrowLeft), Some(Code::ArrowLeft));
        assert_eq!(key_code(Key::F5), Some(Code::F5));
        assert_eq!(key_code(Key::PageDown), Some(Code::PageDown));
        assert_eq!(key_code(Key::Space), Some(Code::Space));
    }

    #[test]
    fn settings_serialize_and_complete() {
        let mut s = HotkeySettings::default();
        s.local.remove(&Action::Mute);
        s.local.insert(Action::Stop, vec![]);
        let json = serde_json::to_string(&s).unwrap();
        let mut back: HotkeySettings = serde_json::from_str(&json).unwrap();
        back.complete();
        assert_eq!(back.local[&Action::Mute], vec![Shortcut::ctrl(Key::M)]);
        assert!(back.local[&Action::Stop].is_empty(), "cleared bindings stay cleared");
        assert_eq!(back.local_action(Shortcut::new(Key::Space)), Some(Action::PlayPause));
    }
}
