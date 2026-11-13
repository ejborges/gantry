<#
.SYNOPSIS
  Puts the Gantry development machine in your VMware Workstation library and opens it.

.DESCRIPTION
  Run once from the project folder after `vagrant up`:

      powershell -ExecutionPolicy Bypass -File dev\vm-open.ps1

  Workstation adds the machine to its library ("gantry-dev") and opens its window. From then
  on you can start it from Workstation like any other machine, without Vagrant, and sign in to
  the desktop as "vagrant" (password "vagrant" unless GANTRY_VM_PASSWORD was set for
  `vagrant up`). Vagrant still manages the same machine: `vagrant halt`, `vagrant ssh` and
  `vagrant up` keep working, and `vagrant destroy` deletes it (the library entry then
  points at nothing; remove it in Workstation).

  When started from Workstation, use Firefox inside the machine (http://localhost:8600), or
  from your computer the machine's own address (shown by `hostname -I` in the machine, and
  in .vagrant\gantry-ports.json). Ports forwarded to localhost are set up by `vagrant up`.

.PARAMETER NoStart
  Add to the library and open the tab, but do not power the machine on.
#>
param([switch]$NoStart)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

# The machine's .vmx: recorded by the Vagrantfile after `vagrant up`, or found on disk.
$vmx = $null
$ports = Join-Path $root ".vagrant\gantry-ports.json"
if (Test-Path $ports) { $vmx = (Get-Content $ports -Raw | ConvertFrom-Json).vmx }
if (-not $vmx -or -not (Test-Path $vmx)) {
  $vmx = Get-ChildItem -Path (Join-Path $root ".vagrant\machines") -Recurse -Filter *.vmx -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
}
if (-not $vmx) { throw "No VMware machine found under .vagrant. Run 'vagrant up --provider vmware_desktop' first." }

# Workstation's install folder, from the registry, or the usual places.
$dir = $null
foreach ($key in "HKLM:\SOFTWARE\WOW6432Node\VMware, Inc.\VMware Workstation", "HKLM:\SOFTWARE\VMware, Inc.\VMware Workstation") {
  if (-not $dir -and (Test-Path $key)) { $dir = (Get-ItemProperty $key -ErrorAction SilentlyContinue).InstallPath }
}
if (-not $dir) {
  $dir = @("${env:ProgramFiles(x86)}\VMware\VMware Workstation", "$env:ProgramFiles\VMware\VMware Workstation") |
    Where-Object { Test-Path (Join-Path $_ "vmware.exe") } | Select-Object -First 1
}
$exe = if ($dir) { Join-Path $dir "vmware.exe" }
if (-not $exe -or -not (Test-Path $exe)) { throw "VMware Workstation (vmware.exe) was not found. Open $vmx from Workstation with File > Open." }

Write-Host "Opening $vmx in VMware Workstation; it stays in the library as 'gantry-dev'."
if ($NoStart) {
  Start-Process -FilePath $exe -ArgumentList "`"$vmx`""
} else {
  # -x powers the machine on as it opens (if it is not running already).
  Start-Process -FilePath $exe -ArgumentList "-x", "`"$vmx`""
}
Write-Host "Sign in as 'vagrant'. Inside the machine: cd /vagrant && dev/run-local.sh, then Firefox at http://localhost:8600."
