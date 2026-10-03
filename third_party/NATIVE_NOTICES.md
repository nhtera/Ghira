# Native components (not visible to cargo-deny / cargo-about)

`cargo-about` only sees Rust crates. These C/C++ components are built by
`tools/scripts/build-nemo.sh` and shipped as shared libraries next to the app
(phase 12 bundles their license texts into About → Licenses). Update this list
when the NeMo-Speech.cpp pin or its build options change.

| Component | Version | License | License text after `build-nemo.sh` |
|---|---|---|---|
| NeMo-Speech.cpp (NVIDIA) | `third_party/NeMo-Speech.cpp` submodule pin | Apache-2.0, with NOTICE | `target/nemo/install/share/licenses/nemo-speech/{LICENSE,NOTICE}` |
| ggml, plus NVIDIA's ggml patches | NeMo-Speech.cpp's `ggml` submodule pin | MIT | `.../nemo-speech/third_party/ggml/LICENSE` |
| parakeet.cpp (derived code inside NeMo-Speech.cpp) | `1675ee5b` | MIT | `.../nemo-speech/THIRD_PARTY_NOTICES.md` |
| SentencePiece (static) | commit pinned in NeMo-Speech.cpp's `scripts/build_sentencepiece_static.sh` | Apache-2.0 | `target/nemo/sentencepiece-src/LICENSE` |
| Abseil (inside SentencePiece) | same | Apache-2.0 | `.../sentencepiece-src/third_party/absl/LICENSE` |
| protobuf-lite (inside SentencePiece) | same | BSD-3-Clause | `.../sentencepiece-src/third_party/protobuf-lite/LICENSE` |
| darts-clone (inside SentencePiece) | same | BSD-2-Clause | `.../sentencepiece-src/third_party/darts_clone/LICENSE` |

The texts of SentencePiece and its bundled libraries (also esaxx, MIT), SQLCipher,
libopus and OpenSSL are vendored in `third_party/licenses/` because their sources
only exist in the build tree or the cargo registry; `tools/scripts/gen-licenses.mjs`
reads them (and NeMo-Speech.cpp's `LICENSE` and `NOTICE` from the submodule) into
the About -> Licenses data of both apps. Update them with the versions above.

C code compiled into Rust crates by their build scripts (statically linked;
`cargo-about` lists the crate, not the C library inside it):

| Component | Version | License | License text |
|---|---|---|---|
| libopus (via `opusic-sys`, feature `bundled`, used by `ghi-audio`) | 1.6.1 (`opusic-sys` 0.7.5) | BSD-3-Clause | `opus/COPYING` in the `opusic-sys` crate source |
| SQLCipher, with SQLite inside (via `libsqlite3-sys`, `bundled-sqlcipher`, used by `ghi-store`) | 4.14.0 / SQLite 3.51 (`libsqlite3-sys` 0.38.2) | BSD-3-Clause-style (SQLCipher, Zetetic); SQLite is public domain | `sqlcipher/LICENSE` in the `libsqlite3-sys` crate source |
| OpenSSL 3 (via `openssl-src`, **non-Apple targets only**: SQLCipher's crypto provider; Apple targets use CommonCrypto) | 3.6 (`openssl-src` 300.6) | Apache-2.0 | `openssl/LICENSE.txt` in the `openssl-src` crate source |

`native/macos/GhiAudioMac` (our Swift capture package, Apache-2.0) is linked
statically into `ghi-audio` on macOS; it uses only Apple system frameworks.

Not built, and must stay out of shipped binaries: KenLM (LGPL), flashlight-text,
llama.cpp, cpp-httplib, OpenSSL, gRPC/Riva protos, Open JTalk, CppJieba, and
miniaudio (mic capture is off). `build-nemo.sh` asserts HTTP, gRPC and
Flashlight are OFF.

Models are downloaded, never bundled; their licenses are in
`crates/ghi-models/registry.toml` (Nemotron models: OpenMDW-1.1).
