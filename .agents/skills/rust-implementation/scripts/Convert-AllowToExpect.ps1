<#
.SYNOPSIS
  Migrates `#[allow(...)]` and `#![allow(...)]` attributes in *.rs files to `expect` with a reason, then lists unfulfilled expectations.
.DESCRIPTION
  The reason is the existing `reason = "..."` (kept), else the `//` comment directly above the attribute, else
  "legacy suppression, migrated from allow". `target/` and generated files (a `@generated` or "automatically generated" marker in the
  first 20 lines, `*.generated.rs`, `*.pb.rs`) are skipped. Afterwards `cargo clippy --all-targets --all-features` runs in -Path and every
  `unfulfilled_lint_expectations` is listed with the advice to remove the attribute or put `allow` back with a comment. The script reports
  and never edits Cargo.toml or the listed attributes.
.PARAMETER Path
  Root directory of the Rust repository.
.PARAMETER WhatIf
  Lists the changes; changes nothing and runs no clippy.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$Path,
  [switch]$WhatIf
)
$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw "Path not found: $Path" }
$root = (Resolve-Path -LiteralPath $Path).Path
$default = 'legacy suppression, migrated from allow'
$sep = [regex]::Escape([IO.Path]::DirectorySeparatorChar)

function Test-Generated([string]$File, [string]$Text) {
  if ($File -match '\.generated\.rs$|\.pb\.rs$') { return $true }
  $head = ($Text -split "`n" | Select-Object -First 20) -join "`n"
  return $head -match '@generated|automatically generated|auto-generated'
}

$files = Get-ChildItem -LiteralPath $root -Recurse -File -Filter *.rs |
  Where-Object { $_.FullName.Substring($root.Length) -notmatch "(^|$sep)(target|\.git|node_modules)$sep" }

$changed = 0
$total = 0
foreach ($f in $files) {
  $text = [IO.File]::ReadAllText($f.FullName)
  if (Test-Generated $f.Name $text) { continue }
  $rx = [regex]'(?<bang>#!?)\[allow\((?<args>[^\]]*?)\)\]'
  $ms = @($rx.Matches($text))
  if ($ms.Count -eq 0) { continue }
  $rel = [IO.Path]::GetRelativePath($root, $f.FullName)
  for ($i = $ms.Count - 1; $i -ge 0; $i--) {
    $m = $ms[$i]
    $args0 = $m.Groups['args'].Value
    if ($args0 -match '\s*\n\s*') { $args0 = ($args0 -replace '\s+', ' ') }
    $args0 = $args0.Trim().TrimEnd(',').Trim()
    if (-not $args0) { continue }
    if ($args0 -match '\breason\s*=') {
      $new = "$($m.Groups['bang'].Value)[expect($args0)]"
    } else {
      $reason = $default
      $before = $text.Substring(0, $m.Index)
      $lineStart = $before.LastIndexOf("`n") + 1
      if ($before.Substring($lineStart).Trim() -eq '' -and $lineStart -gt 0) {
        $prev = ($before.Substring(0, $lineStart - 1) -split "`n")[-1].Trim()
        if ($prev -match '^//+[!/]?\s*(?<c>\S.*)$') { $reason = $Matches['c'].Trim() }
      }
      $reason = $reason.Replace('\', '\\').Replace('"', '\"')
      $new = "$($m.Groups['bang'].Value)[expect($args0, reason = `"$reason`")]"
    }
    $line = ($text.Substring(0, $m.Index) -split "`n").Count
    Write-Output "${rel}:${line}: $($m.Value -replace '\s+', ' ') -> $new"
    $text = $text.Substring(0, $m.Index) + $new + $text.Substring($m.Index + $m.Length)
    $total++
  }
  $changed++
  if (-not $WhatIf) { [IO.File]::WriteAllText($f.FullName, $text) }
}

if ($WhatIf) { Write-Output "whatif: $total attributes in $changed files would change"; return }
Write-Output "converted: $total attributes in $changed files"
if ($total -eq 0) { return }
if (-not (Test-Path -LiteralPath (Join-Path $root 'Cargo.toml'))) { Write-Warning 'no Cargo.toml in Path: clippy not run'; return }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Warning 'cargo not found: clippy not run'; return }

Push-Location $root
try { $out = & cargo clippy --all-targets --all-features 2>&1 | ForEach-Object { "$_" } } finally { Pop-Location }
$found = 0
for ($i = 0; $i -lt $out.Count; $i++) {
  if ($out[$i] -notmatch 'this lint expectation is unfulfilled') { continue }
  for ($j = $i + 1; $j -lt [Math]::Min($i + 6, $out.Count); $j++) {
    if ($out[$j] -match '-->\s*(?<loc>\S+)') {
      Write-Output "unfulfilled_lint_expectations: $($Matches['loc']): remove the attribute, or put allow back with a comment"
      $found++
      break
    }
  }
}
Write-Output "unfulfilled expectations: $found"
