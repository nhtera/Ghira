-- SPDX-License-Identifier: Apache-2.0
-- Schema v5 (phase 14b, semantic search).

-- One vector per ~60 s transcript chunk, per embedding model. The vector
-- encodes what was said, so it is sealed under the meeting DEK like the text
-- (crypto-shredded with it) and removed with the meeting (cascade). Derived
-- data: rebuilt when the transcript version or the model changes, never synced.
CREATE TABLE embeddings (
    meeting_id         INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    chunk              INTEGER NOT NULL,
    t0_ms              INTEGER NOT NULL,
    t1_ms              INTEGER NOT NULL,
    transcript_version INTEGER NOT NULL,
    model              TEXT NOT NULL,
    dim                INTEGER NOT NULL,
    -- f32 little-endian, sealed; AAD binds meeting gid, chunk and model.
    vec_ct             BLOB NOT NULL,
    PRIMARY KEY (meeting_id, chunk, model)
) WITHOUT ROWID;
CREATE INDEX embeddings_model ON embeddings(model, meeting_id);
