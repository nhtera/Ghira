// SPDX-License-Identifier: Apache-2.0
//! NeMo-Speech.cpp FFI and sherpa-onnx wrappers behind the Asr/Diarizer/Embedder traits.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
