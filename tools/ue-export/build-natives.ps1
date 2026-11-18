# Builds CUE4Parse-Natives.dll (ACL anim decompression only) with MSVC into native/. Needs git + Visual Studio (2019+) C++ tools.
$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw "Visual Studio C++ build tools not found (install 'Desktop development with C++' or the Build Tools workload)" }
$src = Join-Path $env:TEMP "cue4src"
if (-not (Test-Path $src)) { git clone --depth 1 --recurse-submodules --shallow-submodules https://github.com/FabianFG/CUE4Parse.git $src }
$n = Join-Path $src "CUE4Parse-Natives"
@"
call "$vs\VC\Auxiliary\Build\vcvars64.bat"
cd /d $n
cl /nologo /O2 /std:c++17 /EHsc /LD /DWITH_ACL /DWIN_EXPORT /IACL\external\acl\includes /IACL\external\acl\external\rtm\includes /Icommon ACL\ACL.cpp Features.cpp /Fe:CUE4Parse-Natives.dll
"@ | Set-Content "$n\build.bat" -Encoding ascii
cmd /c "$n\build.bat"
New-Item -ItemType Directory -Force "$PSScriptRoot\native" | Out-Null
Copy-Item "$n\CUE4Parse-Natives.dll" "$PSScriptRoot\native\" -Force
