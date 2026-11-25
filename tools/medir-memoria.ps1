# Mede a memoria (Working Set - Private, MB) do battery-bar: fechado, aberto e
# fechado depois de 10 ciclos abrir/fechar. Requer o battery-bar rodando.
param([string]$Exe = (Join-Path $PSScriptRoot '..\target\release\battery-bar.exe'))

function Get-Mem {
    # Mesmo contador "Working Set - Private" (independe do idioma do Windows).
    $p = Get-Process -Name battery-bar -ErrorAction Stop | Select-Object -First 1
    $w = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$($p.Id)"
    [math]::Round($w.WorkingSetPrivate / 1MB, 1)
}

if (-not (Get-Process -Name battery-bar -ErrorAction SilentlyContinue)) {
    Start-Process -FilePath $Exe
    Start-Sleep -Seconds 4
}
Start-Sleep -Seconds 2
$inicio = Get-Mem
& $Exe --painel abrir
Start-Sleep -Seconds 3
$aberto = Get-Mem
for ($i = 0; $i -lt 10; $i++) {
    & $Exe --painel fechar
    Start-Sleep -Seconds 1
    & $Exe --painel abrir
    Start-Sleep -Seconds 2
}
& $Exe --painel fechar
Start-Sleep -Seconds 3
$apos = Get-Mem
"fechado-inicio:   $inicio MB"
"aberto:           $aberto MB"
"fechado-apos-10:  $apos MB"
