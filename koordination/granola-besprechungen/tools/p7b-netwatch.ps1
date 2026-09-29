<#
.SYNOPSIS
    P7b / QG5: read-only watcher of the network sockets of the app's process tree.

.DESCRIPTION
    Every -IntervalMs it takes all processes whose executable is -AppExe plus all their
    descendants (llama-server, msedgewebview2, ...) and records every TCP connection and UDP
    endpoint they own (Get-NetTCPConnection / Get-NetUDPEndpoint, no admin needed). Runs
    until -StopFile exists. Writes
      <Out>.csv   one row per distinct socket (first/last seen, ticks)
      <Out>.json  summary: ticks, processes seen, sockets, non-loopback remotes
    Limit: polling can miss a connection that opens and closes between two ticks; the
    logging proxy (p7b_proxy_log.py) covers HTTP clients for that gap.
    ASCII only.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$AppExe,
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$StopFile,
    [int]$IntervalMs = 400
)
$ErrorActionPreference = 'Continue'
$AppExe = (Resolve-Path $AppExe).Path
$sockets = @{}
$procsSeen = @{}
$ticks = 0
$treeTicks = 0
$started = Get-Date

function Test-Loopback([string]$addr) {
    return ($addr -eq '127.0.0.1' -or $addr -eq '::1' -or $addr -eq '0.0.0.0' -or $addr -eq '::' -or
            $addr -like '127.*' -or $addr -eq '' -or $addr -eq '*')
}

while (-not (Test-Path $StopFile)) {
    $tickStart = Get-Date
    $ticks++
    $all = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, ExecutablePath)
    $children = @{}
    foreach ($p in $all) {
        $pp = [int]$p.ParentProcessId
        if (-not $children.ContainsKey($pp)) { $children[$pp] = New-Object System.Collections.ArrayList }
        [void]$children[$pp].Add($p)
    }
    $tree = @{}
    $queue = New-Object System.Collections.Queue
    foreach ($p in $all) { if ($p.ExecutablePath -eq $AppExe) { $queue.Enqueue($p) } }
    while ($queue.Count -gt 0) {
        $p = $queue.Dequeue()
        $id = [int]$p.ProcessId
        if ($tree.ContainsKey($id)) { continue }
        $tree[$id] = $p.Name
        if ($children.ContainsKey($id)) { foreach ($c in $children[$id]) { $queue.Enqueue($c) } }
    }
    if ($tree.Count -gt 0) {
        $treeTicks++
        foreach ($id in $tree.Keys) {
            $k = "$id|$($tree[$id])"
            if (-not $procsSeen.ContainsKey($k)) { $procsSeen[$k] = [ordered]@{ pid = $id; name = $tree[$id]; first_seen = (Get-Date).ToString('HH:mm:ss.fff') } }
        }
        $rows = @()
        $rows += @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $tree.ContainsKey([int]$_.OwningProcess) } |
            ForEach-Object { [pscustomobject]@{ proto = 'tcp'; pid = [int]$_.OwningProcess; local = "$($_.LocalAddress):$($_.LocalPort)"; remote = "$($_.RemoteAddress):$($_.RemotePort)"; raddr = [string]$_.RemoteAddress; state = [string]$_.State } })
        $rows += @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Where-Object { $tree.ContainsKey([int]$_.OwningProcess) } |
            ForEach-Object { [pscustomobject]@{ proto = 'udp'; pid = [int]$_.OwningProcess; local = "$($_.LocalAddress):$($_.LocalPort)"; remote = '*:*'; raddr = '*'; state = 'bound' } })
        foreach ($r in $rows) {
            $key = "$($r.proto)|$($r.pid)|$($r.local)|$($r.remote)"
            $now = (Get-Date).ToString('HH:mm:ss.fff')
            if ($sockets.ContainsKey($key)) {
                $sockets[$key].last_seen = $now
                $sockets[$key].ticks++
                if ($sockets[$key].states -notcontains $r.state) { $sockets[$key].states += $r.state }
            } else {
                $laddr = ($r.local -replace ':\d+$', '')
                $sockets[$key] = [ordered]@{
                    proto = $r.proto; pid = $r.pid; process = $tree[$r.pid]; local = $r.local; remote = $r.remote
                    states = @($r.state); first_seen = $now; last_seen = $now; ticks = 1
                    loopback_only = (Test-Loopback $r.raddr) -and (Test-Loopback $laddr)
                }
            }
        }
    }
    $elapsed = ((Get-Date) - $tickStart).TotalMilliseconds
    $wait = $IntervalMs - [int]$elapsed
    if ($wait -gt 0) { Start-Sleep -Milliseconds $wait }
}

$list = @($sockets.Values | ForEach-Object { [pscustomobject]$_ })
$list | Select-Object proto, pid, process, local, remote, @{n = 'states'; e = { $_.states -join '/' } }, first_seen, last_seen, ticks, loopback_only |
    Export-Csv -Path "$Out.csv" -NoTypeInformation -Encoding UTF8
$external = @($list | Where-Object { -not $_.loopback_only })
$summary = [ordered]@{
    app_exe            = $AppExe
    started            = $started.ToString('s')
    finished           = (Get-Date).ToString('s')
    interval_ms        = $IntervalMs
    ticks              = $ticks
    ticks_with_tree    = $treeTicks
    processes_seen     = @($procsSeen.Values)
    sockets            = $list.Count
    tcp_sockets        = @($list | Where-Object { $_.proto -eq 'tcp' }).Count
    udp_endpoints      = @($list | Where-Object { $_.proto -eq 'udp' }).Count
    non_loopback       = $external
    non_loopback_count = $external.Count
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -Path "$Out.json" -Encoding UTF8
