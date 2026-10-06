# Format Forge - Office COM resident worker
#
# Started by the Rust side as:
#   powershell -NoProfile -ExecutionPolicy Bypass -File <this> -App word
# Keeps one Application COM instance alive for the life of the process and speaks
# a line-delimited JSON protocol over stdin/stdout.
#
# Why it is shaped this way:
#   1. The COM instance is created once. Word cold-starts in 2-4s; rebuilding it
#      per file makes a batch an order of magnitude slower.
#   2. Every protocol line carries the @@FF@@ prefix. Office writes noise to
#      stdout, so anything without the prefix is discarded as noise.
#   3. Every dialog must be suppressed. Missing one hangs the whole pipeline
#      until the job timeout fires.
#   4. Documents are always opened read-only. Source files are never modified.
#
# Two deliberate constraints on this file's contents:
#
#   * It is pure ASCII. Windows PowerShell 5.1 reads .ps1 files using the system
#     ANSI codepage unless they carry a UTF-8 BOM, which silently corrupts
#     non-ASCII source. Keeping it ASCII sidesteps that entirely.
#   * It does no error classification and holds no user-facing text. It reports
#     the raw COM HRESULT and message; Rust maps those onto status codes and
#     produces every string the user sees. ConvertTo-Json escapes non-ASCII to
#     \uXXXX, so a localized COM message crosses the pipe intact.

param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('word', 'excel', 'powerpoint')]
    [string]$App
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Protocol I/O must be raw UTF-8 in BOTH directions. Input matters as much as
# output: Console.InputEncoding defaults to the OEM codepage (936 on a
# Chinese Windows), which would mangle a UTF-8 command line carrying a CJK
# path. Rust always writes UTF-8, so this must match.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
$script:Out = [Console]::Out

$PROTO = '@@FF@@'
$script:Quit = $false
$script:StartTime = Get-Date
$script:Debug = ($env:FF_DEBUG -eq '1')

function Write-Dbg([string]$Msg) {
    if ($script:Debug) { [Console]::Error.WriteLine("[ff-dbg] $Msg") }
}

function Send([hashtable]$Payload) {
    $json = $Payload | ConvertTo-Json -Compress -Depth 6
    $script:Out.WriteLine("$PROTO$json")
    $script:Out.Flush()
}

# Report a failure verbatim. HRESULT is the machine-readable part; the message
# is passed through for the human.
function SendErr([string]$Src, [System.Exception]$Ex) {
    $hr = 0
    try { $hr = [int]$Ex.HResult } catch { }
    Send @{
        op       = 'err'
        src      = $Src
        hresult  = $hr
        msg      = "$($Ex.Message)"
        inner    = "$($Ex.InnerException.Message)"
    }
}

# ---------------------------------------------------------------- adapters

function New-WordApp {
    $w = New-Object -ComObject Word.Application
    $w.Visible = $false
    $w.DisplayAlerts = 0            # wdAlertsNone
    $w.AutomationSecurity = 3       # msoAutomationSecurityForceDisable - no macros
    try { $w.Options.SaveInterval = 0 } catch { }
    try { $w.Options.WarnBeforeSavingPrintingSendingMarkup = $false } catch { }
    try { $w.Options.UpdateLinksAtOpen = $false } catch { }
    return $w
}

function Convert-Word($app, [string]$src, [string]$dst) {
    # Optional parameters must be [Type]::Missing, not ''. Passing an empty
    # string for PasswordDocument / PasswordTemplate makes Word reject the file
    # with the nonsense error "这是一个无效文件名" (HRESULT 80004005), which
    # looks like a path problem and is not one.
    #
    # Open(FileName, ConfirmConversions=false, ReadOnly=true, AddToRecentFiles=false,
    #      PasswordDocument, PasswordTemplate, Revert, WritePasswordDocument)
    $doc = $app.Documents.Open(
        $src, $false, $true, $false,
        [Type]::Missing, [Type]::Missing, $false
    )
    try {
        # ExportAsFixedFormat(OutputFileName, ExportFormat=17 (wdExportFormatPDF))
        $doc.ExportAsFixedFormat($dst, 17)
    }
    finally {
        $doc.Close(0)               # SaveChanges=0 (wdDoNotSaveChanges)
        [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($doc)
    }
}

function New-ExcelApp {
    $e = New-Object -ComObject Excel.Application
    $e.Visible = $false
    $e.DisplayAlerts = $false
    $e.ScreenUpdating = $false
    $e.EnableEvents = $false
    $e.AutomationSecurity = 3       # msoAutomationSecurityForceDisable
    return $e
}

function Convert-Excel($app, [string]$src, [string]$dst) {
    # Optional parameters must be [Type]::Missing rather than $null or ''. With
    # $null, Excel's late-bound IDispatch refuses the call outright with
    # error 1004 "不能取得类 Workbooks 的 Open 属性" -- a dispatch failure that
    # reads like a permissions problem and is not one.
    #
    # Open(FileName, UpdateLinks=0, ReadOnly=true)
    $wb = $app.Workbooks.Open($src, 0, $true)
    try {
        # ExportAsFixedFormat(Type=0 (xlTypePDF), Filename)
        $wb.ExportAsFixedFormat(0, $dst)
    }
    finally {
        $wb.Close($false)           # do not save
        [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($wb)
    }
}

function New-PowerPointApp {
    $p = New-Object -ComObject PowerPoint.Application
    # PowerPoint rejects Visible=$false, so it runs as a minimized window. It
    # does not steal focus but may flash in the taskbar.
    try { $p.Visible = $true } catch { }
    try { $p.DisplayAlerts = 1 } catch { }   # ppAlertsNone
    try { $p.AutomationSecurity = 3 } catch { }
    return $p
}

function Convert-PowerPoint($app, [string]$src, [string]$dst) {
    # Open(FileName, ReadOnly=msoTrue(-1), Untitled=msoFalse(0), WithWindow=msoFalse(0))
    # WithWindow=false is what actually keeps it headless.
    $pres = $app.Presentations.Open($src, -1, 0, 0)
    try {
        # SaveAs(FileName, FileFormat=32 (ppSaveAsPDF))
        $pres.SaveAs($dst, 32)
    }
    finally {
        $pres.Close()
        [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($pres)
    }
}

# ---------------------------------------------------------------- main loop

$script:Com = $null
$convert = $null

try {
    switch ($App) {
        'word'       { $script:Com = New-WordApp;       $convert = ${function:Convert-Word} }
        'excel'      { $script:Com = New-ExcelApp;      $convert = ${function:Convert-Excel} }
        'powerpoint' { $script:Com = New-PowerPointApp; $convert = ${function:Convert-PowerPoint} }
    }
}
catch {
    $hr = 0
    try { $hr = [int]$_.Exception.HResult } catch { }
    Send @{ op = 'fatal'; hresult = $hr; msg = "$($_.Exception.Message)" }
    exit 1
}

Send @{ op = 'ready'; app = $App }

# Read commands line by line. Rust closing stdin (EOF) ends the loop.
while ($null -ne ($line = [Console]::In.ReadLine())) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }

    $req = $null
    try {
        $req = $line | ConvertFrom-Json
    }
    catch {
        Send @{ op = 'err'; src = ''; hresult = 0; msg = 'malformed command json'; inner = '' }
        continue
    }

    $op = $null
    if ($req -is [psobject]) { $op = [string]$req.op }

    switch ($op) {
        'ping' {
            Send @{ op = 'pong'; app = $App }
        }

        'convert' {
            $src = [string]$req.src
            $dst = [string]$req.dst
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            try {
                Write-Dbg "convert src=[$src] dst=[$dst] exists=[$(Test-Path -LiteralPath $src)]"
                & $convert $script:Com $src $dst
                $sw.Stop()

                if (-not (Test-Path -LiteralPath $dst)) {
                    Send @{
                        op      = 'err'
                        src     = $src
                        hresult = 0
                        msg     = 'conversion finished but output file is missing'
                        inner   = ''
                    }
                }
                else {
                    Send @{ op = 'ok'; src = $src; dst = $dst; ms = [int]$sw.ElapsedMilliseconds }
                }
            }
            catch {
                $sw.Stop()
                SendErr $src $_.Exception
            }
        }

        'quit' {
            # A break inside switch only leaves the switch, so raise a flag.
            $script:Quit = $true
        }

        default {
            Send @{ op = 'err'; src = ''; hresult = 0; msg = "unknown op: $op"; inner = '' }
        }
    }

    if ($script:Quit) { break }
}

# ---------------------------------------------------------------- shutdown
# Quit explicitly or WINWORD.EXE / EXCEL.EXE survive as orphan processes.
if ($null -ne $script:Com) {
    try { $script:Com.Quit() } catch { }
    try { [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($script:Com) } catch { }
    $script:Com = $null
}

[GC]::Collect()
[GC]::WaitForPendingFinalizers()
[GC]::Collect()
[GC]::WaitForPendingFinalizers()

# A fresh Word instance spawns a hatted sibling process (the one that owns the
# frame); Quit() alone can leave it behind. SIGKILL the whole family: any
# WINWORD/EXCEL/POWERPNT older than this worker was started by this worker,
# since we require the app to be closed before the worker starts.
try {
    $cutoff = $script:StartTime
    $names = @{ 'word' = 'WINWORD'; 'excel' = 'EXCEL'; 'powerpoint' = 'POWERPNT' }
    $procName = $names[$App]
    if ($procName) {
        Get-Process -Name $procName -ErrorAction SilentlyContinue |
            Where-Object { $_.StartTime -ge $cutoff } |
            ForEach-Object { try { $_.Kill() } catch { } }
    }
}
catch { }

exit 0
