<#
  Excel COM 探测脚本。
  用 UTF-8 BOM 保存以避免 Windows PowerShell 5.1 的 ANSI 码页破坏中文。
  用法：powershell -NoProfile -ExecutionPolicy Bypass -File scripts/diag-excel.ps1
#>
$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
[Console]::InputEncoding  = [Text.Encoding]::UTF8

$root = Split-Path -Parent $PSScriptRoot
$src  = Join-Path $root 'fixtures\销售数据.xlsx'
$dst  = Join-Path $root 'out\sales.pdf'

Write-Host "源文件: $src"
Write-Host "存在: $(Test-Path -LiteralPath $src)"

$e = New-Object -ComObject Excel.Application
$e.Visible = $false
$e.DisplayAlerts = $false
$e.ScreenUpdating = $false
$e.EnableEvents = $false
Write-Host "Excel 版本: $($e.Version)"
Write-Host "启动后 Workbooks.Count = $($e.Workbooks.Count)"

function Attempt([string]$label, [scriptblock]$body) {
    try {
        $wb = & $body
        Write-Host "  [OK]   $label  sheets=$($wb.Sheets.Count)"
        return $wb
    } catch {
        Write-Host "  [FAIL] $label  HR=$([int]$_.Exception.HResult)  $($_.Exception.Message)"
        return $null
    }
}

Write-Host ''
Write-Host 'A) Open(FileName)'
$wbA = Attempt 'A' { $e.Workbooks.Open($src) }

Write-Host 'B) Open(FileName, Missing, true)'
$wbB = Attempt 'B' { $e.Workbooks.Open($src, [Type]::Missing, $true) }

Write-Host 'C) Open 全参数 Missing + ReadOnly'
$wbC = Attempt 'C' {
    $m = [Type]::Missing
    $e.Workbooks.Open($src, $m, $true, $m, $m, $m, $m, $m, $m, $m, $m, $m, $m, $m, $m)
}

Write-Host ''
foreach ($pair in @(@('A', $wbA), @('B', $wbB), @('C', $wbC))) {
    $wb = $pair[1]
    if ($null -eq $wb) { continue }
    try {
        $wb.ExportAsFixedFormat(0, $dst)
        Write-Host "  导出成功（走 $($pair[0]) 路径）: $(Test-Path -LiteralPath $dst)"
        $wb.Close($false)
    } catch {
        Write-Host "  导出失败（$($pair[0])）: $($_.Exception.Message)"
    }
}

$e.Quit()
[GC]::Collect()
[GC]::WaitForPendingFinalizers()
