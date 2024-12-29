# asak

A cross-platform audio recording/playback CLI tool with TUI, written in Rust. The goal is to be an audio Swiss Army Knife (asak), like SoX but more interactive and fun.

![Asak](./asak.gif)

## install

> You need to have `cargo` installed, see [here](https://doc.rust-lang.org/cargo/getting-started/installation.html).

### step 1

```sh
cargo install asak
```

Note: Make sure the [JACK Audio Connection Kit](https://jackaudio.org) is installed on your machine prior to installing `asak`. For instance, on Ubuntu/Mint, if nothing is returned when running `sudo dpkg -l | grep libjack`, you will need to `sudo apt install libjack-dev`.

### step 2

```sh
asak --help
```

## usage

### record

```sh
asak rec hello
```

> If no output name is provided, a prompt will come for you to input output file name. UTC format such as `2024-04-14T09:17:40Z.wav` will be provided as initial file name.

### playback

```sh
asak play hello.wav
```

> If no input name is provided, it will search current directory for `.wav` files and open an interactive menu.

### monitor

```sh
asak monitor
```

> Reminder: ⚠️ Watch your volume when play the video below❗️

https://github.com/chaosprint/asak/assets/35621141/f0876503-4dc7-4c92-b324-c36ec5b747d0



> Known issue: you need to select the same output device as the one in your current system settings.

## roadmap?

- [x] record audio
- [x] basic audio playback
- [x] monitoring an input device with an output device
- [ ] rec device, dur, sr, ch, fmt
- [ ] play device, dur, sr, ch, fmt
- [ ] playback live pos control
- [ ] live amp + fx (reverb, delay, etc)
- [ ] passthru + live fx

## contribution

Just open an issue or PR, I'm happy to discuss and collaborate.
