//! Interface translations. Adding a language = adding one `Strings` table.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ru];

    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }

    /// Pick the UI language from the system locale.
    pub fn detect() -> Self {
        let vars = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"];
        let ru = vars.iter().filter_map(|v| std::env::var(v).ok()).any(|v| v.to_lowercase().starts_with("ru"));
        if ru { Lang::Ru } else { Lang::En }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::Ru => &RU,
        }
    }

    /// Count with a correctly inflected noun, e.g. "5 треков".
    pub fn tracks(self, n: usize) -> String {
        match self {
            Lang::En => format!("{n} {}", if n == 1 { "track" } else { "tracks" }),
            Lang::Ru => {
                let word = match (n % 10, n % 100) {
                    (1, r) if r != 11 => "трек",
                    (2..=4, r) if !(12..=14).contains(&r) => "трека",
                    _ => "треков",
                };
                format!("{n} {word}")
            }
        }
    }
}

pub struct Strings {
    pub menu: &'static str,
    pub add_files: &'static str,
    pub add_folder: &'static str,
    pub open_playlist: &'static str,
    pub save_playlist: &'static str,
    pub new_playlist: &'static str,
    pub rename_playlist: &'static str,
    pub delete_playlist: &'static str,
    pub clear_playlist: &'static str,
    pub default_playlist: &'static str,
    pub settings: &'static str,
    pub language: &'static str,
    pub exit: &'static str,
    pub minimize: &'static str,
    pub maximize: &'static str,
    pub close: &'static str,

    pub play: &'static str,
    pub pause: &'static str,
    pub stop: &'static str,
    pub prev: &'static str,
    pub next: &'static str,
    pub shuffle: &'static str,
    pub repeat_off: &'static str,
    pub repeat_all: &'static str,
    pub repeat_one: &'static str,
    pub volume: &'static str,
    pub mute: &'static str,
    pub balance: &'static str,
    pub time_toggle_hint: &'static str,
    pub nothing_playing: &'static str,
    pub stopped: &'static str,
    pub paused: &'static str,

    pub ctx_play: &'static str,
    pub ctx_queue: &'static str,
    pub ctx_unqueue: &'static str,
    pub ctx_remove: &'static str,
    pub ctx_open_folder: &'static str,
    pub select_all: &'static str,
    pub remove_missing: &'static str,
    pub remove_duplicates: &'static str,
    pub remove: &'static str,
    pub add: &'static str,

    pub sort: &'static str,
    pub sort_title: &'static str,
    pub sort_artist: &'static str,
    pub sort_album: &'static str,
    pub sort_duration: &'static str,
    pub sort_path: &'static str,
    pub sort_random: &'static str,
    pub sort_reverse: &'static str,

    pub group: &'static str,
    pub group_none: &'static str,
    pub group_album: &'static str,
    pub group_artist: &'static str,
    pub group_folder: &'static str,
    pub group_genre: &'static str,
    pub group_year: &'static str,
    pub unknown: &'static str,

    pub search_hint: &'static str,
    pub drop_hint: &'static str,
    pub settings_title: &'static str,
    pub system_frame: &'static str,
    pub show_spectrum: &'static str,
    pub accent_color: &'static str,
    pub hotkeys: &'static str,
    pub hotkeys_help: &'static str,
    pub about: &'static str,
    pub about_text: &'static str,
    pub playback_error: &'static str,
    pub audio_files: &'static str,
    pub playlists: &'static str,
    pub ffmpeg: &'static str,
    pub ffmpeg_missing: &'static str,
    pub add_url: &'static str,
    pub url_hint: &'static str,
    pub radio: &'static str,
    pub connecting: &'static str,
    pub record: &'static str,
    pub stop_recording: &'static str,
    pub recording_saved: &'static str,
    pub recording_error: &'static str,
    pub record_section: &'static str,
    pub record_format: &'static str,
    pub record_original: &'static str,
    pub record_dir: &'static str,
    pub record_split: &'static str,
    pub browse: &'static str,
}

pub static EN: Strings = Strings {
    menu: "Menu",
    add_files: "Add files…",
    add_folder: "Add folder…",
    open_playlist: "Open playlist…",
    save_playlist: "Save playlist as M3U8…",
    new_playlist: "New playlist",
    rename_playlist: "Rename",
    delete_playlist: "Close playlist",
    clear_playlist: "Clear playlist",
    default_playlist: "Default",
    settings: "Settings…",
    language: "Language",
    exit: "Exit",
    minimize: "Minimize",
    maximize: "Maximize",
    close: "Close",

    play: "Play",
    pause: "Pause",
    stop: "Stop",
    prev: "Previous",
    next: "Next",
    shuffle: "Shuffle",
    repeat_off: "Repeat: off",
    repeat_all: "Repeat: playlist",
    repeat_one: "Repeat: track",
    volume: "Volume",
    mute: "Mute",
    balance: "Balance",
    time_toggle_hint: "Click to toggle elapsed / remaining time",
    nothing_playing: "Nothing is playing",
    stopped: "Stopped",
    paused: "Paused",

    ctx_play: "Play",
    ctx_queue: "Add to queue",
    ctx_unqueue: "Remove from queue",
    ctx_remove: "Remove from playlist",
    ctx_open_folder: "Open containing folder",
    select_all: "Select all",
    remove_missing: "Remove missing files",
    remove_duplicates: "Remove duplicates",
    remove: "Remove",
    add: "Add",

    sort: "Sort",
    sort_title: "By title",
    sort_artist: "By artist / album",
    sort_album: "By album",
    sort_duration: "By duration",
    sort_path: "By file path",
    sort_random: "Randomize",
    sort_reverse: "Reverse order",

    group: "Grouping",
    group_none: "No grouping",
    group_album: "By album",
    group_artist: "By artist",
    group_folder: "By folder",
    group_genre: "By genre",
    group_year: "By year",
    unknown: "Unknown",

    search_hint: "Search in playlist…",
    drop_hint: "Drop files or folders here, or use “+” below",
    settings_title: "Settings",
    system_frame: "Use system window frame (restart required)",
    show_spectrum: "Show spectrum analyzer",
    accent_color: "Accent color",
    hotkeys: "Hotkeys",
    hotkeys_help: "Space / X — play·pause, V — stop, Z / B — previous / next,\n\
        ←/→ — seek 5 s, Ctrl+↑/↓ — volume, Enter — play selected,\n\
        Q — add to queue, Delete — remove, Ctrl+A — select all, Ctrl+F — search",
    about: "About",
    about_text: "janmp — a desktop music player in the spirit of AIMP, written in Rust with egui.",
    playback_error: "Playback error",
    audio_files: "Audio files",
    playlists: "Playlists and cue sheets",
    ffmpeg: "FFmpeg (APE, WMA, Opus, DTS, trackers…)",
    ffmpeg_missing: "FFmpeg not found. Install it or put ffmpeg next to janmp to play APE, WMA, Opus, DTS, AC3, WavPack, DSD and tracker modules.",
    add_url: "Add URL (internet radio)…",
    url_hint: "http://… stream or playlist URL",
    radio: "Internet radio",
    connecting: "Connecting…",
    record: "Record stream",
    stop_recording: "Stop recording",
    recording_saved: "Recording saved",
    recording_error: "Recording failed",
    record_section: "Radio recording",
    record_format: "Format",
    record_original: "Original stream (lossless for MP3/AAC)",
    record_dir: "Folder",
    record_split: "New file for each song",
    browse: "Browse…",
};

pub static RU: Strings = Strings {
    menu: "Меню",
    add_files: "Добавить файлы…",
    add_folder: "Добавить папку…",
    open_playlist: "Открыть плейлист…",
    save_playlist: "Сохранить плейлист в M3U8…",
    new_playlist: "Новый плейлист",
    rename_playlist: "Переименовать",
    delete_playlist: "Закрыть плейлист",
    clear_playlist: "Очистить плейлист",
    default_playlist: "Основной",
    settings: "Настройки…",
    language: "Язык",
    exit: "Выход",
    minimize: "Свернуть",
    maximize: "Развернуть",
    close: "Закрыть",

    play: "Воспроизвести",
    pause: "Пауза",
    stop: "Стоп",
    prev: "Предыдущий",
    next: "Следующий",
    shuffle: "Случайный порядок",
    repeat_off: "Повтор: выключен",
    repeat_all: "Повтор: плейлист",
    repeat_one: "Повтор: трек",
    volume: "Громкость",
    mute: "Без звука",
    balance: "Баланс",
    time_toggle_hint: "Нажмите, чтобы переключить прошедшее / оставшееся время",
    nothing_playing: "Ничего не воспроизводится",
    stopped: "Остановлено",
    paused: "Пауза",

    ctx_play: "Воспроизвести",
    ctx_queue: "Добавить в очередь",
    ctx_unqueue: "Убрать из очереди",
    ctx_remove: "Удалить из плейлиста",
    ctx_open_folder: "Открыть папку с файлом",
    select_all: "Выделить всё",
    remove_missing: "Удалить несуществующие файлы",
    remove_duplicates: "Удалить дубликаты",
    remove: "Удалить",
    add: "Добавить",

    sort: "Сортировка",
    sort_title: "По названию",
    sort_artist: "По исполнителю / альбому",
    sort_album: "По альбому",
    sort_duration: "По длительности",
    sort_path: "По пути к файлу",
    sort_random: "Перемешать",
    sort_reverse: "Обратный порядок",

    group: "Группировка",
    group_none: "Без группировки",
    group_album: "По альбому",
    group_artist: "По исполнителю",
    group_folder: "По папке",
    group_genre: "По жанру",
    group_year: "По году",
    unknown: "Неизвестно",

    search_hint: "Поиск в плейлисте…",
    drop_hint: "Перетащите сюда файлы или папки либо нажмите «+» внизу",
    settings_title: "Настройки",
    system_frame: "Системная рамка окна (нужен перезапуск)",
    show_spectrum: "Показывать анализатор спектра",
    accent_color: "Цвет акцента",
    hotkeys: "Горячие клавиши",
    hotkeys_help: "Пробел / X — воспроизведение·пауза, V — стоп, Z / B — предыдущий / следующий,\n\
        ←/→ — перемотка на 5 с, Ctrl+↑/↓ — громкость, Enter — играть выделенный,\n\
        Q — в очередь, Delete — удалить, Ctrl+A — выделить всё, Ctrl+F — поиск",
    about: "О программе",
    about_text: "janmp — музыкальный плеер для рабочего стола в духе AIMP, написан на Rust и egui.",
    playback_error: "Ошибка воспроизведения",
    audio_files: "Аудиофайлы",
    playlists: "Плейлисты и CUE",
    ffmpeg: "FFmpeg (APE, WMA, Opus, DTS, трекерная музыка…)",
    ffmpeg_missing: "FFmpeg не найден. Установите его или положите ffmpeg рядом с janmp, чтобы играли APE, WMA, Opus, DTS, AC3, WavPack, DSD и трекерные модули.",
    add_url: "Добавить URL (интернет-радио)…",
    url_hint: "http://… адрес потока или плейлиста",
    radio: "Интернет-радио",
    connecting: "Подключение…",
    record: "Записывать поток",
    stop_recording: "Остановить запись",
    recording_saved: "Запись сохранена",
    recording_error: "Ошибка записи",
    record_section: "Запись радио",
    record_format: "Формат",
    record_original: "Исходный поток (без потерь для MP3/AAC)",
    record_dir: "Папка",
    record_split: "Новый файл для каждой песни",
    browse: "Обзор…",
};

#[cfg(test)]
mod tests {
    use super::Lang;

    #[test]
    fn russian_plurals() {
        assert_eq!(Lang::Ru.tracks(1), "1 трек");
        assert_eq!(Lang::Ru.tracks(3), "3 трека");
        assert_eq!(Lang::Ru.tracks(11), "11 треков");
        assert_eq!(Lang::Ru.tracks(21), "21 трек");
        assert_eq!(Lang::Ru.tracks(112), "112 треков");
    }
}
