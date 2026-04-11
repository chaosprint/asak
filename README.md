# asak

`asak` is a terminal audio tool written in Rust. It gives you a keyboard-driven TUI for browsing audio files, previewing waveforms, recording new `.wav` files, and selecting playback and recording devices.

<img width="912" height="740" alt="image" src="https://github.com/user-attachments/assets/d9e62893-40de-424b-9277-08fea11fa52a" />


## What It Does

- Browse and preview `wav`, `mp3`, and `ogg` files
- Record `.wav` files with a live monitor waveform
- Show a full-take timeline while recording
- Display playback waveform, time, and channel meters
- Choose system input and output devices from the terminal

## Install

You need a Rust toolchain with `cargo` installed.

Install from crates.io:

```sh
cargo install asak
```

Install from this repository:

```sh
cargo install --path .
```

On Linux, you will usually need audio development packages for `cpal`:

```sh
sudo apt install libasound2-dev libjack-dev
```

If you want JACK support explicitly:

```sh
cargo install --path . --features jack
```

## Start

Run the app:

```sh
asak
```

Run it from source:

```sh
cargo run
```

## Interface

When `asak` starts, you choose between three modes:

- `Play`
- `Rec`
- `Settings`

### Play

`Play` opens a file browser rooted at the current working directory.

- Browse folders and supported audio files
- Open a file to preview it immediately
- See the full waveform with a left-to-right playhead
- See playback time, sample rate, channels, and meters

Keys:

- `Up` / `Down`: move through the browser
- `Enter`: open folder or preview file
- `Backspace`: go to parent directory
- `Space`: pause or resume playback
- `Esc`: stop playback and return to mode selection

### Rec

`Rec` records a `.wav` file into the current working directory.

- Type a file name in the left panel
- Press `Enter` to start recording
- Press `Enter` again to stop
- Watch the live monitor waveform while recording
- See the full-take timeline update as the recording grows

After saving, `asak` returns to the mode selector and refreshes the `Play` browser so the new file is ready to preview.

Keys before recording starts:

- `Left` / `Right`: move cursor in the filename
- `Home` / `End`: jump cursor
- `Backspace` / `Delete`: edit filename
- `Enter`: start recording

Keys while recording:

- `Enter`: stop recording
- `Esc`: leave recording mode

### Settings

`Settings` lets you choose playback and recording devices.

- Select `Playback Device` or `Recording Device`
- Open the device list with `Enter`
- Move through devices with `Up` / `Down`
- Confirm and return with `Enter` or `Backspace`
- Refresh device discovery with `r`

The system default devices are used as the default selection.

## Global Keys

- `Up` / `Down`: move between modes on the mode selector
- `Enter`: open the selected mode
- `Esc`: return to the mode selector
- `q`: quit

## Development

Format the project:

```sh
cargo fmt
```

Run Clippy with warnings denied:

```sh
cargo clippy --all-targets -- -D warnings
```

Build a release binary:

```sh
cargo build --release
```

The build script also generates shell completions and man pages into `target/completions` and `target/man`.

## Notes

- `asak` uses the current working directory as the initial browser root
- Recordings are saved into the current working directory by default
- The app runs in an alternate terminal screen while active

## Contributing

Issues and pull requests are welcome.
