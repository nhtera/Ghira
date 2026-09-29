# SPDX-License-Identifier: Apache-2.0
# Install the Ghira eval kit (Windows PowerShell 5.1 or 7). Run from anywhere:
#   .\scripts\setup.ps1              basic kit
#   .\scripts\setup.ps1 -WithNemo    also the NeMo models used for labelling drafts (large)
#
# uv (the Python tool that installs everything) is required. If it is missing we show the
# official installer command and ask y/N before running it. We never run it silently.
param([switch]$WithNemo)
# Not 'Stop': Windows PowerShell 5.1 turns a native tool's stderr output (uv progress) into errors.
# We check $LASTEXITCODE instead.

Set-Location (Join-Path $PSScriptRoot '..')

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    $installer = 'irm https://astral.sh/uv/install.ps1 | iex'
    Write-Host 'uv is not installed. The official installer is:'
    Write-Host "  powershell -ExecutionPolicy ByPass -c `"$installer`""
    Write-Host '(It is published by Astral, https://docs.astral.sh/uv/getting-started/installation/)'
    $answer = Read-Host 'Run it now? [y/N]'
    if ($answer -notmatch '^(y|yes)$') {
        Write-Host 'Not installed. Run the command above yourself, then run this script again.'
        exit 1
    }
    powershell -ExecutionPolicy ByPass -c $installer
    $env:Path = "$env:USERPROFILE\.local\bin;$env:Path"
    if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
        Write-Host 'uv still not found. Open a new PowerShell window and retry.'
        exit 1
    }
}

if ($WithNemo) { uv sync --locked --extra nemo } else { uv sync --locked }
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

uv run ghi-eval --help | Out-Null
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host 'OK: the eval kit is installed. Next: follow docs\runbook.md, step 4 (dry run).'
