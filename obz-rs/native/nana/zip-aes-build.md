# NanaZip ZIP AES compatibility build (x64)

Based on official NanaZip tag 7.0.1882.0, commit
`555a3ff0f0e6c754dbf93c528253afeb0444e9e1`; original source and license:
https://github.com/M2Team/NanaZip/tree/7.0.1882.0 . Apply
`zip-aes-acp.patch` at the source root. ZIP AES writes the Windows ANSI codepage bytes
only when they decode back to the original password; otherwise it retains
upstream UTF-8. Reader code remains unchanged: NanaZip tries UTF-8 then the
system ANSI codepage; 7-Zip uses the system ANSI codepage. Different systems
with different ANSI codepages may still disagree on non-ASCII passwords.
This is an x64-only change.

Build used Visual Studio 2022 Build Tools 17.14.41, MSVC 14.44.35207,
Release x64, with MSBuild SDK Mile.Project.Configurations 1.1.2116 and NuGet
packages Mile.Detours 1.0.2442, Mile.Json 1.0.1057, Mile.Mobility 1.1.678,
Mile.Windows.Helpers 1.0.1427, Mile.Windows.Internal 1.0.3729 and
Mile.Windows.UniCrt 1.2.410, fetched from https://api.nuget.org/v3/index.json .
For this toolchain, set `TreatWarningAsError` to `false` in the temporary
Mile.Project.Cpp.props (Brotli emits C4127), then run:

```
MSBuild NanaZip.Core/NanaZip.Core.vcxproj /restore /p:RestoreSources=https://api.nuget.org/v3/index.json /p:Configuration=Release /p:Platform=x64 /m
```

The rebuilt DLL is `Output/Binaries/Release/x64/NanaZip.Core.dll`. Its build
contains time-dependent metadata; a fresh build is not asserted byte-identical.
The tested binary SHA-256 is recorded in runtime/nanazip/provenance.json.
The rebuilt PE declares OS and subsystem 10.00; testing on the oldest supported
Windows release remains outstanding. The other five DLL/console files are from
the upstream package identified in the provenance record.
