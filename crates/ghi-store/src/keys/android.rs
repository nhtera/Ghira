// SPDX-License-Identifier: Apache-2.0
//! Android Keystore key store: not implemented until phase 17.

use super::{KeyRing, KeyStore, Protection};
use crate::StoreError;

const UNSUPPORTED: &str = "Android Keystore (phase 17)";

/// Placeholder that reports [`StoreError::Unsupported`] for every operation.
#[derive(Debug, Default)]
pub struct AndroidKeystore;

impl KeyStore for AndroidKeystore {
    fn load(&self) -> Result<Option<KeyRing>, StoreError> {
        Err(StoreError::Unsupported(UNSUPPORTED))
    }

    fn save(&self, _: &KeyRing, _: Protection) -> Result<(), StoreError> {
        Err(StoreError::Unsupported(UNSUPPORTED))
    }

    fn delete(&self) -> Result<(), StoreError> {
        Err(StoreError::Unsupported(UNSUPPORTED))
    }
}
