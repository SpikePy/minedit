#!/bin/sh
# Build and install minedit for the current Windows user (from WSL):
# %LOCALAPPDATA%\Programs\minedit\minedit.exe plus a Start menu shortcut.
# A running minedit is closed (its tabs are saved in the session) and started again.
set -e
cd "$(dirname "$0")"
cargo build --release --target x86_64-pc-windows-gnu
powershell.exe -NoProfile -Command '
$ErrorActionPreference = "Stop"
$dir = Join-Path $env:LOCALAPPDATA "Programs\minedit"
New-Item -ItemType Directory -Force $dir | Out-Null
$exe = Join-Path $dir "minedit.exe"
# Close running copies the normal way, so they save their session.
$running = @(Get-Process minedit -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dir\*" })
$running | ForEach-Object { [void]$_.CloseMainWindow() }
$closed = @($running | Where-Object { $_.WaitForExit(10000) }).Count -eq $running.Count
# A running exe cannot be overwritten but can be renamed; the old copy is removed on a later install.
Remove-Item "$exe.old" -ErrorAction SilentlyContinue
if (Test-Path $exe) { Move-Item -Force $exe "$exe.old" }
Copy-Item -Force "'"$(wslpath -w target/x86_64-pc-windows-gnu/release/minedit.exe)"'" $exe
Unblock-File $exe
$lnk = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path ([Environment]::GetFolderPath("Programs")) "minedit.lnk"))
$lnk.TargetPath = $exe
$lnk.WorkingDirectory = $dir
$lnk.Save()
# Explorer integration for this user: "Open with" for every file type and an "Edit with minedit" entry.
$open = "`"$exe`" `"%1`""
$reg = [Microsoft.Win32.Registry]::CurrentUser
function Set-Key($path, $values) {
    $k = $reg.CreateSubKey($path)
    $values.GetEnumerator() | ForEach-Object { $k.SetValue($_.Key, $_.Value) }
    $k.Close()
}
Set-Key "Software\Classes\Applications\minedit.exe" @{ FriendlyAppName = "minedit" }
Set-Key "Software\Classes\Applications\minedit.exe\shell\open\command" @{ "" = $open }
Set-Key "Software\Classes\*\OpenWithList\minedit.exe" @{}
Set-Key "Software\Classes\*\shell\minedit" @{ "" = "Edit with minedit"; Icon = "`"$exe`"" }
Set-Key "Software\Classes\*\shell\minedit\command" @{ "" = $open }
"installed $exe"
if ($running.Count -gt 0) {
    if ($closed) { Start-Process $exe -WorkingDirectory $dir; "restarted minedit" }
    else { "minedit did not close within 10 s; restart it yourself to use the new version" }
}
'
