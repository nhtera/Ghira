# SPDX-License-Identifier: Apache-2.0
import copy
import json

import pytest
from conftest import CLI_GOLDEN

from ghi_eval.contract import KINDS, is_valid, validation_errors

GOLDEN = {
    "transcript": "transcript.json",
    "diarization": "diarization.json",
    "notes": "notes.json",
    "bench": "bench.json",
    "version": "version.json",
    "error": "error.json",
}


def load(name):
    return json.loads((CLI_GOLDEN / name).read_text(encoding="utf-8"))


@pytest.mark.parametrize("kind", GOLDEN)
def test_golden_documents_are_valid(kind):
    assert validation_errors(load(GOLDEN[kind]), kind) == []


def test_golden_events_are_valid():
    lines = (CLI_GOLDEN / "events.ndjson").read_text(encoding="utf-8").splitlines()
    assert len(lines) == 3
    for line in lines:
        assert validation_errors(json.loads(line), "event") == []


def test_all_schemas_load():
    for kind in KINDS:
        validation_errors({}, kind)


def test_malformed_variants_fail():
    doc = load("transcript.json")
    bad = copy.deepcopy(doc)
    del bad["segments"]
    assert not is_valid(bad, "transcript")
    bad = copy.deepcopy(doc)
    bad["segments"][0]["start"] = "1.2"
    assert not is_valid(bad, "transcript")
    bad = copy.deepcopy(doc)
    bad["schema"] = "ghi.transcript/2"
    assert not is_valid(bad, "transcript")
    bad = copy.deepcopy(doc)
    bad["segments"][1]["id"] = 0  # duplicate id
    assert "not unique" in " ".join(validation_errors(bad, "transcript"))
    bad = copy.deepcopy(load("notes.json"))
    bad["action_items"][0]["citations"] = ["a"]
    assert not is_valid(bad, "notes")
    ev = json.loads((CLI_GOLDEN / "events.ndjson").read_text(encoding="utf-8").splitlines()[0])
    ev["type"] = "middle"
    assert not is_valid(ev, "event")


def test_error_messages_do_not_echo_values():
    doc = load("transcript.json")
    doc["segments"][0]["start"] = "SECRET-TEXT"
    assert "SECRET-TEXT" not in " ".join(validation_errors(doc, "transcript"))
