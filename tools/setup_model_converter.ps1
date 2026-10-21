param(
    [Parameter(Mandatory = $true)]
    [string]$PythonExe
)

$ErrorActionPreference = 'Stop'
$base = Join-Path $env:LOCALAPPDATA 'BaslerCameraTest\model-converter'
if (-not (Test-Path -LiteralPath $PythonExe -PathType Leaf)) {
    throw "Python executable not found: $PythonExe"
}
& $PythonExe -m venv $base
if ($LASTEXITCODE -ne 0) { throw 'Could not create the model converter environment' }
$venvPython = Join-Path $base 'Scripts\python.exe'
& $venvPython -m pip install --disable-pip-version-check 'torch==2.5.1' 'torchvision==0.20.1' --index-url 'https://download.pytorch.org/whl/cpu'
if ($LASTEXITCODE -ne 0) { throw 'Could not install CPU PyTorch' }
& $venvPython -m pip install --disable-pip-version-check 'ultralytics==8.3.176' 'onnx>=1.16,<2'
if ($LASTEXITCODE -ne 0) { throw 'Could not install Ultralytics/ONNX' }
Write-Host "Model converter ready: $venvPython"
