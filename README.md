# asak

A cross-platform audio recording/playback CLI tool with TUI, written in Rust. The goal is to be an audio Swiss Army Knife (asak), like SoX but more interative and fun.

## install

> You need to have `cargo` installed, see [here](https://doc.rust-lang.org/cargo/getting-started/installation.html).

### step 1

```sh
cargo install asak
```

### step 2

```sh
asak --help
```

## usage

### record

```sh
asak rec hello
```

If no output name is provided, it will default to `YYMMDD-HHMMSS.wav`.

### playback

```sh
asak play 1.wav
```

## temp roadmap

- [x] record audio
- [x] basic audio playback
- [ ] rec device, dur, sr, ch, fmt
- [ ] play device, dur, sr, ch, fmt
- [ ] playback live pos control
- [ ] live amp + fx (reverb, delay, etc)
- [ ] passthru + live fx

## contribution

Just open an issue or PR, I'm happy to discuss and collaborate.
