param([Parameter(Mandatory=$true)][string]$Source, [Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference = 'Stop'
# Fixed helper; caller values are arguments, never interpolated into source.
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class OpenBandiZipIsoStream {
    public static void Save(object value, string path) {
        IStream stream = (IStream)value;
        IntPtr read = Marshal.AllocHGlobal(4);
        try {
            using (var output = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.None)) {
                byte[] buffer = new byte[1024 * 1024];
                while (true) {
                    Marshal.WriteInt32(read, 0);
                    stream.Read(buffer, buffer.Length, read);
                    int count = Marshal.ReadInt32(read);
                    if (count == 0) break;
                    output.Write(buffer, 0, count);
                }
                output.Flush(true);
            }
        } finally { Marshal.FreeHGlobal(read); }
    }
}
'@
$image = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
try {
    $image.ChooseImageDefaultsForMediaType(12)
    $image.FileSystemsToCreate = 3 # ISO9660 + Joliet
    $image.FreeMediaBlocks = 2147483647
    $image.VolumeName = 'OpenBandiZip'
    $image.Root.AddTree($Source, $false)
    $result = $image.CreateResultImage()
    [OpenBandiZipIsoStream]::Save($result.ImageStream, $Output)
} finally {
    if ($null -ne $image) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($image) }
}
