# C-2/M-7 sonrasi bu script'in ayri kopyasi gerekmiyor: start-local.ps1
# rastgele-kalici anahtarlari (.local-runtime/local-keys.env) uretip set
# ediyor. Geriye uyumluluk icin ayni davranisa yonlendirir.
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
& (Join-Path -Path $PSScriptRoot -ChildPath 'start-local.ps1') @PSBoundParameters
exit $LASTEXITCODE
