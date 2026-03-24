$repo = "YourUsername/grove"
$binary = "grove.exe"

# Detect architecture
$isArm = (Get-WmiObject Win32_Processor).Architecture -eq 12
$arch = if ($isArm) { "aarch64" } else { "x86_64" }

$url = "https://github.com/$repo/releases/latest/download/grove-windows-$arch.exe"
$dest = "$env:USERPROFILE\.local\bin\$binary"

New-Item -ItemType Directory -Force -Path (Split-Path $dest) | Out-Null

Write-Host "Downloading grove for windows/$arch..."
Invoke-WebRequest -Uri $url -OutFile $dest

Write-Host "grove installed to $dest"
Write-Host "Make sure $env:USERPROFILE\.local\bin is in your PATH."
