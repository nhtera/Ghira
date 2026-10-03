// SPDX-License-Identifier: Apache-2.0
// What the phone app ships, by name and license: the pinned models
// (crates/ghi-models/registry.toml), the speech runtime and the main libraries.
// The desktop's generated per-package notices (tools/scripts/gen-licenses.mjs)
// are Mac-only for now; a generated mobile list replaces this in 16-K.
export const LICENSES: { name: string; license: string }[] = [
  { name: "nvidia/nemotron-3.5-asr-streaming-0.6b", license: "OpenMDW-1.1" },
  { name: "nvidia/Nemotron-3-Diarization", license: "OpenMDW-1.1" },
  {
    name: "csukuangfj/speaker-embedding-models (CAM++)",
    license: "Apache-2.0",
  },
  { name: "NeMo-Speech.cpp", license: "Apache-2.0" },
  { name: "Tauri", license: "MIT OR Apache-2.0" },
  { name: "React", license: "MIT" },
  { name: "TanStack Router", license: "MIT" },
  { name: "Radix UI", license: "MIT" },
  { name: "Be Vietnam Pro", license: "OFL-1.1" },
  { name: "Material Symbols", license: "Apache-2.0" },
];
