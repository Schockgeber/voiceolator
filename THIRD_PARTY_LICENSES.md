# Third-party components

Voiceolator itself is licensed MIT OR Apache-2.0. It bundles or depends on the following.
Everything is permissively licensed, except the Symphonia audio decoders (MPL-2.0, a
file-level copyleft: they are used unmodified and their source is on crates.io and
https://github.com/pdeljanov/Symphonia). Nothing requires source disclosure of Voiceolator.

## Models and runtimes shipped next to the executable

| Component | Version | License | Source |
|---|---|---|---|
| DPDFNet `dpdfnet2_48khz_hr.onnx` (Ceva) | HF snapshot 2026-10 | Apache-2.0 | https://huggingface.co/Ceva-IP/DPDFNet |
| ONNX Runtime `onnxruntime.dll` / `libonnxruntime.so` (Microsoft) | 1.31.0 (from the official `onnxruntime` PyPI wheels) | MIT | https://github.com/microsoft/onnxruntime |

## Compiled into the executable

| Component | License | Source |
|---|---|---|
| DeepFilterNet3 model + libDF (Hendrik Schröter), commit d375b2d | MIT OR Apache-2.0 | https://github.com/Rikorose/DeepFilterNet |

## Rust crates (statically linked)

All permissive; where a crate offers a choice (e.g. "Apache-2.0 OR GPL-2.0-only"), Voiceolator uses it under the permissive option.

| Crate | Version | License |
|---|---|---|
| accesskit | 0.25.1 | MIT OR Apache-2.0 |
| addr2line | 0.21.0 | Apache-2.0 OR MIT |
| adler | 1.0.2 | 0BSD OR MIT OR Apache-2.0 |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| ahash | 0.8.12 | MIT OR Apache-2.0 |
| aho-corasick | 1.1.3 | Unlicense OR MIT |
| anyhow | 1.0.82 | MIT OR Apache-2.0 |
| arc-swap | 1.9.3 | MIT OR Apache-2.0 |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| as-raw-xcb-connection | 1.0.1 | MIT OR Apache-2.0 |
| async-broadcast | 0.7.2 | MIT OR Apache-2.0 |
| async-channel | 2.5.0 | Apache-2.0 OR MIT |
| async-executor | 1.14.0 | Apache-2.0 OR MIT |
| async-io | 2.6.0 | Apache-2.0 OR MIT |
| async-lock | 3.4.2 | Apache-2.0 OR MIT |
| async-process | 2.5.0 | Apache-2.0 OR MIT |
| async-recursion | 1.2.0 | MIT OR Apache-2.0 |
| async-signal | 0.2.14 | Apache-2.0 OR MIT |
| async-task | 4.7.1 | Apache-2.0 OR MIT |
| async-trait | 0.1.92 | MIT OR Apache-2.0 |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT |
| backtrace | 0.3.71 | MIT OR Apache-2.0 |
| base64 | 0.22.1 | MIT OR Apache-2.0 |
| bit-set | 0.5.3 | MIT/Apache-2.0 |
| bit-vec | 0.6.3 | MIT/Apache-2.0 |
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| blocking | 1.7.0 | Apache-2.0 OR MIT |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT |
| byteorder | 1.5.0 | Unlicense OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| bytes | 1.6.0 | MIT |
| calloop | 0.13.0 | MIT |
| cfg-if | 1.0.0 | MIT/Apache-2.0 |
| color | 0.3.3 | Apache-2.0 OR MIT |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT |
| const-random | 0.1.18 | MIT OR Apache-2.0 |
| const-random-macro | 0.1.16 | MIT OR Apache-2.0 |
| core_detect | 1.0.0 | MIT/Apache-2.0 |
| cpufeatures | 0.2.12 | MIT OR Apache-2.0 |
| crc | 3.4.0 | MIT OR Apache-2.0 |
| crc-catalog | 2.5.0 | MIT OR Apache-2.0 |
| crc32fast | 1.4.0 | MIT OR Apache-2.0 |
| crossbeam-channel | 0.5.12 | MIT OR Apache-2.0 |
| crossbeam-queue | 0.3.14 | MIT OR Apache-2.0 |
| crossbeam-utils | 0.8.19 | MIT OR Apache-2.0 |
| crunchy | 0.2.2 | MIT |
| crypto-common | 0.1.6 | MIT OR Apache-2.0 |
| ctrlc | 3.5.2 | MIT/Apache-2.0 |
| cursor-icon | 1.2.0 | MIT OR Apache-2.0 OR Zlib |
| deep_filter v0.5.7-pre | (https://github.com/Rikorose/DeepFilterNet?rev=d375b2d8309e0935d165700c91da9de862a99c31#d375b2d8) | MIT/Apache-2.0 |
| derive-new | 0.5.9 | MIT |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 |
| dlib | 0.5.3 | MIT |
| dlv-list | 0.5.2 | MIT OR Apache-2.0 |
| document-features | 0.2.12 | MIT OR Apache-2.0 |
| downcast-rs | 1.2.1 | MIT/Apache-2.0 |
| dpi | 0.1.2 | Apache-2.0 AND MIT |
| dyn-clone | 1.0.17 | MIT OR Apache-2.0 |
| ecolor | 0.37.0 | MIT OR Apache-2.0 |
| eframe | 0.37.0 | MIT OR Apache-2.0 |
| egui | 0.37.0 | MIT OR Apache-2.0 |
| egui-winit | 0.37.0 | MIT OR Apache-2.0 |
| egui_glow | 0.37.0 | MIT OR Apache-2.0 |
| either | 1.11.0 | MIT OR Apache-2.0 |
| emath | 0.37.0 | MIT OR Apache-2.0 |
| encoding_rs | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| endi | 1.1.1 | MIT |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 |
| env_filter | 2.0.0 | MIT OR Apache-2.0 |
| env_logger | 0.11.11 | MIT OR Apache-2.0 |
| epaint | 0.37.0 | MIT OR Apache-2.0 |
| epaint_default_fonts | 0.37.0 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 |
| equivalent | 1.0.3 | Apache-2.0 OR MIT |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| euclid | 0.22.14 | MIT OR Apache-2.0 |
| event-listener | 5.4.2 | Apache-2.0 OR MIT |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT |
| extended | 0.1.0 | MIT |
| fastrand | 2.5.0 | Apache-2.0 OR MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| fearless_simd | 0.4.1 | Apache-2.0 OR MIT |
| filetime | 0.2.23 | MIT/Apache-2.0 |
| flate2 | 1.1.10 | MIT OR Apache-2.0 |
| foldhash | 0.2.0 | Zlib |
| font-types | 0.12.7 | MIT OR Apache-2.0 |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 |
| futures-core | 0.3.34 | MIT OR Apache-2.0 |
| futures-io | 0.3.34 | MIT OR Apache-2.0 |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 |
| futures-task | 0.3.34 | MIT OR Apache-2.0 |
| futures-util | 0.3.34 | MIT OR Apache-2.0 |
| generic-array | 0.14.7 | MIT |
| gethostname | 1.1.0 | Apache-2.0 |
| getrandom | 0.2.14 | MIT OR Apache-2.0 |
| gimli | 0.28.1 | MIT OR Apache-2.0 |
| glow | 0.17.0 | MIT OR Apache-2.0 OR Zlib |
| glutin | 0.32.3 | Apache-2.0 |
| glutin-winit | 0.5.0 | MIT |
| glutin_egl_sys | 0.7.1 | Apache-2.0 |
| glutin_glx_sys | 0.6.1 | Apache-2.0 |
| glutin_wgl_sys | 0.6.1 | Apache-2.0 |
| guillotiere | 0.7.0 | MIT/Apache-2.0 |
| half | 2.7.1 | MIT OR Apache-2.0 |
| harfrust | 0.12.0 | MIT |
| hashbrown | 0.14.5 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| hex | 0.4.3 | MIT OR Apache-2.0 |
| hound | 3.5.1 | Apache-2.0 |
| icu_collections | 2.4.0 | Unicode-3.0 |
| icu_locale_core | 2.4.0 | Unicode-3.0 |
| icu_normalizer | 2.4.0 | Unicode-3.0 |
| icu_normalizer_data | 2.4.0 | Unicode-3.0 |
| icu_properties | 2.4.0 | Unicode-3.0 |
| icu_properties_data | 2.4.0 | Unicode-3.0 |
| icu_provider | 2.4.0 | Unicode-3.0 |
| idna | 1.1.0 | MIT OR Apache-2.0 |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT |
| image | 0.25.10 | MIT OR Apache-2.0 |
| indexmap | 2.14.2 | Apache-2.0 OR MIT |
| itertools | 0.10.5 | MIT/Apache-2.0 |
| itertools | 0.12.1 | MIT OR Apache-2.0 |
| itertools | 0.15.0 | MIT OR Apache-2.0 |
| itoa | 1.0.11 | MIT OR Apache-2.0 |
| jiff | 0.2.38 | Unlicense OR MIT |
| jiff-core | 0.1.1 | Unlicense OR MIT |
| keyboard-types | 0.8.3 | MIT OR Apache-2.0 |
| ksni | 0.3.7 | Unlicense |
| kurbo | 0.13.1 | Apache-2.0 OR MIT |
| lazy_static | 1.4.0 | MIT/Apache-2.0 |
| libc | 0.2.191 | MIT OR Apache-2.0 |
| libloading | 0.8.9 | ISC |
| libloading | 0.9.0 | ISC |
| libm | 0.2.8 | MIT OR Apache-2.0 |
| libpulse-binding | 2.30.1 | MIT OR Apache-2.0 |
| libpulse-simple-binding | 2.29.0 | MIT OR Apache-2.0 |
| libpulse-simple-sys | 1.22.0 | MIT OR Apache-2.0 |
| libpulse-sys | 1.23.0 | MIT OR Apache-2.0 |
| linebender_resource_handle | 0.1.1 | Apache-2.0 OR MIT |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| linux-raw-sys | 0.4.13 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| litemap | 0.8.3 | Unicode-3.0 |
| litrs | 1.0.1 | MIT OR Apache-2.0 |
| lock_api | 0.4.14 | MIT OR Apache-2.0 |
| lockfree-object-pool | 0.1.6 | BSL-1.0 |
| log | 0.4.34 | MIT OR Apache-2.0 |
| lzma-rs | 0.3.0 | MIT |
| maplit | 1.0.2 | MIT/Apache-2.0 |
| matrixmultiply | 0.3.8 | MIT/Apache-2.0 |
| memchr | 2.7.2 | Unlicense OR MIT |
| memmap2 | 0.9.4 | MIT OR Apache-2.0 |
| minimal-lexical | 0.2.1 | MIT/Apache-2.0 |
| miniz_oxide | 0.7.2 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| muda | 0.21.2 | Apache-2.0 OR MIT |
| multiversion_no_op | 1.0.0 | Apache-2.0 OR MIT |
| ndarray | 0.15.6 | MIT OR Apache-2.0 |
| nix | 0.31.3 | MIT |
| nohash-hasher | 0.2.0 | Apache-2.0 OR MIT |
| nom | 7.1.3 | MIT |
| num-complex | 0.4.5 | MIT OR Apache-2.0 |
| num-derive | 0.4.2 | MIT OR Apache-2.0 |
| num-integer | 0.1.46 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| object | 0.32.2 | Apache-2.0 OR MIT |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| openssl-probe | 0.1.6 | MIT/Apache-2.0 |
| ordered-multimap | 0.7.3 | MIT |
| ordered-stream | 0.2.0 | MIT OR Apache-2.0 |
| ort | 2.0.0-rc.15 | MIT OR Apache-2.0 |
| ort-sys | 2.0.0-rc.15 | MIT OR Apache-2.0 |
| parking | 2.2.1 | Apache-2.0 OR MIT |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 |
| paste | 1.0.14 | MIT OR Apache-2.0 |
| pastey | 0.2.3 | MIT OR Apache-2.0 |
| peniko | 0.6.1 | Apache-2.0 OR MIT |
| percent-encoding | 2.3.1 | MIT OR Apache-2.0 |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| piper | 0.2.5 | MIT OR Apache-2.0 |
| png | 0.17.16 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| polling | 3.11.0 | Apache-2.0 OR MIT |
| pollster | 0.4.0 | Apache-2.0/MIT |
| polycool | 0.4.0 | MIT OR Apache-2.0 |
| potential_utf | 0.1.6 | Unicode-3.0 |
| ppv-lite86 | 0.2.17 | MIT/Apache-2.0 |
| primal-check | 0.3.3 | MIT OR Apache-2.0 |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| profiling | 1.0.18 | MIT OR Apache-2.0 |
| prost | 0.11.9 | Apache-2.0 |
| prost-derive | 0.11.9 | Apache-2.0 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| rand | 0.8.5 | MIT OR Apache-2.0 |
| rand_chacha | 0.3.1 | MIT OR Apache-2.0 |
| rand_core | 0.6.4 | MIT OR Apache-2.0 |
| rand_distr | 0.4.3 | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| rawpointer | 0.2.1 | MIT/Apache-2.0 |
| read-fonts | 0.41.0 | MIT OR Apache-2.0 |
| realfft | 3.3.0 | MIT |
| regex | 1.13.1 | MIT OR Apache-2.0 |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 |
| rfd | 0.17.2 | MIT |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| ringbuf | 0.4.8 | MIT/Apache-2.0 |
| rubato | 0.14.1 | MIT |
| rubato | 0.16.2 | MIT |
| rust-ini | 0.21.0 | MIT |
| rustc-demangle | 0.1.23 | MIT/Apache-2.0 |
| rustfft | 6.2.0 | MIT OR Apache-2.0 |
| rustix | 0.38.34 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustix | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT |
| rustls-native-certs | 0.7.3 | Apache-2.0 OR ISC OR MIT |
| rustls-pemfile | 2.2.0 | Apache-2.0 OR ISC OR MIT |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 |
| rustls-webpki | 0.103.16 | ISC |
| rustversion | 1.0.23 | MIT OR Apache-2.0 |
| same-file | 1.0.6 | Unlicense/MIT |
| scan_fmt | 0.2.6 | MIT |
| schannel | 0.1.30 | MIT |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| self_cell | 1.3.0 | Apache-2.0 OR GPL-2.0-only |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 |
| sha2 | 0.10.8 | MIT OR Apache-2.0 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| simd-adler32 | 0.3.10 | MIT |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 |
| skrifa | 0.44.0 | MIT OR Apache-2.0 |
| slab | 0.4.12 | MIT |
| smallvec | 1.16.4 | MIT OR Apache-2.0 |
| smol_str | 0.2.2 | MIT OR Apache-2.0 |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 |
| strength_reduce | 0.2.4 | MIT OR Apache-2.0 |
| string-interner | 0.15.0 | MIT/Apache-2.0 |
| subtle | 2.6.1 | BSD-3-Clause |
| symphonia | 0.5.5 | MPL-2.0 |
| symphonia-bundle-flac | 0.5.5 | MPL-2.0 |
| symphonia-bundle-mp3 | 0.5.5 | MPL-2.0 |
| symphonia-codec-aac | 0.5.5 | MPL-2.0 |
| symphonia-codec-adpcm | 0.5.5 | MPL-2.0 |
| symphonia-codec-alac | 0.5.5 | MPL-2.0 |
| symphonia-codec-pcm | 0.5.5 | MPL-2.0 |
| symphonia-codec-vorbis | 0.5.5 | MPL-2.0 |
| symphonia-core | 0.5.5 | MPL-2.0 |
| symphonia-format-isomp4 | 0.5.5 | MPL-2.0 |
| symphonia-format-mkv | 0.5.5 | MPL-2.0 |
| symphonia-format-ogg | 0.5.5 | MPL-2.0 |
| symphonia-format-riff | 0.5.5 | MPL-2.0 |
| symphonia-metadata | 0.5.5 | MPL-2.0 |
| symphonia-utils-xiph | 0.5.5 | MPL-2.0 |
| syn | 1.0.109 | MIT OR Apache-2.0 |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.9 | MIT OR Apache-2.0 |
| synstructure | 0.14.0 | MIT |
| tar | 0.4.40 | MIT/Apache-2.0 |
| task-local | 0.1.1 | MIT OR Apache-2.0 |
| thiserror | 1.0.59 | MIT OR Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.59 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 |
| tiny-keccak | 2.0.2 | CC0-1.0 |
| tinystr | 0.8.5 | Unicode-3.0 |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_edit | 0.25.17+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| tracing | 0.1.44 | MIT |
| tracing-attributes | 0.1.31 | MIT |
| tracing-core | 0.1.36 | MIT |
| tract-core | 0.21.4 | MIT OR Apache-2.0 |
| tract-data | 0.21.4 | MIT OR Apache-2.0 |
| tract-hir | 0.21.4 | MIT OR Apache-2.0 |
| tract-linalg | 0.21.4 | MIT OR Apache-2.0 |
| tract-nnef | 0.21.4 | MIT OR Apache-2.0 |
| tract-onnx | 0.21.4 | MIT OR Apache-2.0 |
| tract-onnx-opl | 0.21.4 | MIT OR Apache-2.0 |
| tract-pulse | 0.21.4 | MIT OR Apache-2.0 |
| tract-pulse-opl | 0.21.4 | MIT OR Apache-2.0 |
| transpose | 0.2.3 | MIT OR Apache-2.0 |
| tray-icon | 0.26.1 | MIT OR Apache-2.0 |
| trim-in-place | 0.1.7 | MIT |
| typenum | 1.17.0 | MIT OR Apache-2.0 |
| unicode-general-category | 1.1.0 | Apache-2.0 |
| unicode-ident | 1.0.12 | (MIT OR Apache-2.0) AND Unicode-DFS-2016 |
| unicode-properties | 0.1.4 | MIT/Apache-2.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| untrusted | 0.9.0 | ISC |
| ureq | 2.12.1 | MIT OR Apache-2.0 |
| url | 2.5.4 | MIT OR Apache-2.0 |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT |
| uuid | 1.29.0 | Apache-2.0 OR MIT |
| vello_common | 0.2.0 | Apache-2.0 OR MIT |
| vello_cpu | 0.2.0 | Apache-2.0 OR MIT |
| walkdir | 2.5.0 | Unlicense/MIT |
| wasapi | 0.26.0 | MIT |
| web-time | 1.1.0 | MIT OR Apache-2.0 |
| webbrowser | 1.2.5 | MIT OR Apache-2.0 |
| webpki-roots | 0.26.11 | CDLA-Permissive-2.0 |
| webpki-roots | 1.0.11 | CDLA-Permissive-2.0 |
| winapi-util | 0.1.8 | Unlicense OR MIT |
| windows | 0.62.2 | MIT OR Apache-2.0 |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 |
| windows-core | 0.62.2 | MIT OR Apache-2.0 |
| windows-future | 0.3.2 | MIT OR Apache-2.0 |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 |
| windows-link | 0.2.1 | MIT OR Apache-2.0 |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 |
| windows-result | 0.4.1 | MIT OR Apache-2.0 |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 |
| windows-targets | 0.52.5 | MIT OR Apache-2.0 |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 |
| windows_x86_64_gnu | 0.52.5 | MIT OR Apache-2.0 |
| winit | 0.30.13 | Apache-2.0 |
| winnow | 1.0.4 | MIT |
| writeable | 0.6.4 | Unicode-3.0 |
| x11-dl | 2.21.0 | MIT |
| x11rb | 0.13.2 | MIT OR Apache-2.0 |
| x11rb-protocol | 0.13.2 | MIT OR Apache-2.0 |
| xattr | 1.3.1 | MIT/Apache-2.0 |
| xkbcommon-dl | 0.4.2 | MIT |
| xkeysym | 0.2.1 | MIT OR Apache-2.0 OR Zlib |
| yoke | 0.8.4 | Unicode-3.0 |
| yoke-derive | 0.8.4 | Unicode-3.0 |
| zbus | 5.19.0 | MIT |
| zbus_macros | 5.19.0 | MIT |
| zbus_names | 4.3.5 | MIT |
| zcheapstr | 1.1.0 | MIT |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerofrom | 0.1.8 | Unicode-3.0 |
| zerofrom-derive | 0.1.8 | Unicode-3.0 |
| zeroize | 1.9.1 | Apache-2.0 OR MIT |
| zerotrie | 0.2.5 | Unicode-3.0 |
| zerovec | 0.11.9 | Unicode-3.0 |
| zerovec-derive | 0.11.7 | Unicode-3.0 |
| zip | 2.1.1 | MIT |
| zmij | 1.0.23 | MIT |
| zopfli | 0.8.1 | Apache-2.0 |
| zvariant | 5.15.1 | MIT |
| zvariant_derive | 5.15.1 | MIT |
| zvariant_utils | 4.2.0 | MIT |

## Downloaded only when the user agrees (never shipped)

| Component | License | Notes |
|---|---|---|
| ffmpeg 9.0.2 "essentials" build for Windows (gyan.dev) | GPL-3.0 | Only for video files and MP3/M4A/FLAC output. Downloaded from www.gyan.dev after the user confirms, checked against a fixed SHA-256, stored in `%LOCALAPPDATA%\Voiceolator\ffmpeg`. A separate program that Voiceolator runs; it is not linked |
| ffmpeg 7.0.2 static build for Linux x86_64 (johnvansickle.com) | GPL-3.0 | Same, stored in `~/.local/share/voiceolator/ffmpeg`. A system ffmpeg (e.g. `sudo apt install ffmpeg`) is used instead when present |

## README graphics (docs/assets, not part of the program)

Text in the banner and download buttons is drawn from the outlines of the Inter typeface
(Rasmus Andersson, SIL Open Font License 1.1). The Linux logo is the "Linux" icon from
Simple Icons (CC0-1.0). See `tools/readme_graphics.py`.

## Fonts embedded in the GUI (via `epaint_default_fonts`)

Ubuntu-Light (Ubuntu Font Licence 1.0), Hack (MIT / Bitstream Vera), Noto Emoji (SIL OFL 1.1),
emoji-icon-font (SIL OFL 1.1 / MIT).

## Used from the system at runtime (not bundled)

| Component | License | Notes |
|---|---|---|
| PulseAudio client library `libpulse.so.0`, `libpulse-simple.so.0` (Linux) | LGPL-2.1+ | Dynamically linked from the user's system; present on every PulseAudio/PipeWire desktop |
| OpenGL, X11 libraries (Linux) / Windows system DLLs | system | Loaded from the operating system |

## Optional, installed by the user (not bundled)

| Component | License | Notes |
|---|---|---|
| VB-CABLE virtual audio device (VB-Audio) | donationware | Only for the optional virtual-cable routing mode |
| Microsoft Visual C++ 2015-2022 Redistributable (x64) | Microsoft redistributable | Needed by `onnxruntime.dll`; present on most gaming PCs. Without it Voiceolator falls back to DeepFilterNet3 |

## Evaluation data (used only locally for testing, never shipped)

EARS speech (CC BY-NC 4.0), MUSDB18 7-s previews (research use), ESC-50 (CC BY-NC 3.0),
Microsoft DNSMOS models (MIT, from the DNS-Challenge repository).
