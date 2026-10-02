// SPDX-License-Identifier: Apache-2.0
// Types and lazy loader for licenses.json (Settings -> About -> Licenses).
// licenses.json is generated: run `pnpm gen:licenses` (tools/scripts/gen-licenses.mjs;
// needs cargo-about and a prior `pnpm build`). Do not edit it by hand.

export interface LicenseText {
  /** SPDX id, e.g. "MIT". */
  id: string;
  name: string;
  /** Text without per-project copyright lines (those are on each entry). */
  text: string;
}

/** A Rust crate or bundled npm package. */
export interface PackageLicense {
  name: string;
  version: string;
  /** SPDX expression as declared by the package. */
  license: string;
  /** Keys into `Licenses.licenses`. */
  licenseKeys: string[];
  copyright?: string[];
}

export interface ModelLicense {
  id: string;
  name: string;
  license: string;
  url: string;
  /** Empty when the license text is not in the repo; `url` is then the reference. */
  licenseKeys: string[];
}

export interface AssetLicense {
  name: string;
  license: string;
  notice: string;
  licenseKeys: string[];
}

export interface Licenses {
  licenses: Record<string, LicenseText>;
  rust: PackageLicense[];
  js: PackageLicense[];
  models: ModelLicense[];
  assets: AssetLicense[];
}

/** Loads the generated data as its own chunk. */
export async function loadLicenses(): Promise<Licenses> {
  const mod = await import("./licenses.json");
  return mod.default as Licenses;
}
