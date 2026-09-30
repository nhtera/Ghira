-- SPDX-License-Identifier: Apache-2.0
-- Schema v2 (phase 6, notes engine). Action items can be AI-written: a
-- regenerate replaces AI items that are neither done nor edited, like AI note
-- blocks that aren't pinned. They keep the due date as spoken ("thứ Sáu") and
-- every citation, not just the first.
ALTER TABLE action_items ADD COLUMN provenance TEXT NOT NULL DEFAULT 'user'
    CHECK (provenance IN ('user', 'ai', 'ai_edited'));
ALTER TABLE action_items ADD COLUMN due_text_ct BLOB;
ALTER TABLE action_items ADD COLUMN anchors_json TEXT NOT NULL DEFAULT '[]';
-- v1 items keep their one citation in the new list too.
UPDATE action_items SET anchors_json = '[' || anchor_json || ']' WHERE anchor_json IS NOT NULL;
