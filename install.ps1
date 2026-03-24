$isArm = (Get-WmiObject Win32_Processor).Architecture -eq 12
$arch = if ($isArm) { "aarch64" } else { "x86_64" }
$url = "https://github.com/avatardeejay/grove/releases/latest/download/grove-windows-$arch.exe"
$tmp = "$env:TEMP\grove-installer.exe"
Invoke-WebRequest -Uri $url -OutFile $tmp -ErrorAction Stop
Start-Process $tmp
