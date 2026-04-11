use chrono::Local;

pub(crate) fn default_recording_name() -> String {
    Local::now().format("%Y-%m-%d_%H-%M-%S.wav").to_string()
}

pub(crate) fn default_recording_cursor(name: &str) -> usize {
    name.strip_suffix(".wav")
        .map(|stem| stem.chars().count())
        .unwrap_or_else(|| name.chars().count())
}

pub(crate) fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(text.len())
}

pub(crate) fn format_duration(seconds: f32) -> String {
    let total_seconds = seconds.max(0.0).round() as u64;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

pub(crate) fn format_sample_rate(sample_rate: f64) -> String {
    format!("{:.1} kHz", sample_rate / 1000.0)
}
