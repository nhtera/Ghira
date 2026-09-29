# SPDX-License-Identifier: Apache-2.0
"""Exception types shared across the harness."""


class HarnessError(Exception):
    """A problem the user can fix (bad dataset, bad input); the CLI prints it and exits 1."""


class ManifestError(HarnessError):
    """manifest.yaml is missing or structurally invalid."""


class AdapterError(HarnessError):
    """The system under test failed on one input (exit 1/2, bad JSON, crash)."""


class InvalidOutput(AdapterError):
    """The system returned a document that does not match its schema."""

    def __init__(self, message: str, raw: object | None = None):
        super().__init__(message)
        self.raw = raw


class NotSupported(HarnessError):
    """The system does not implement a task (ghi exit code 3, or an adapter without it)."""

    def __init__(self, task: str, reason: str = ""):
        super().__init__(f"system does not support {task} yet" + (f" ({reason})" if reason else ""))
        self.task = task
        self.reason = reason


class PrivacyLeak(HarnessError):
    """The privacy lint refused a report."""

    def __init__(self, findings: list[dict]):
        kinds = sorted({f["kind"] for f in findings})
        super().__init__(f"privacy lint failed ({', '.join(kinds)}); report not written")
        self.findings = findings
