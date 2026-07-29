using System.Runtime.InteropServices;
using FontFerry.Core.Models;
using Microsoft.Win32;

namespace FontFerry.Core.Installers;

public sealed class WindowsFontInstaller : IFontInstaller
{
    private const string RegistryPath =
        @"Software\Microsoft\Windows NT\CurrentVersion\Fonts";
    private const uint WmFontChange = 0x001D;
    private const uint SmtoAbortIfHung = 0x0002;

    public async Task<PlatformInstallResult> InstallAsync(
        string catalogId,
        string version,
        IReadOnlyList<FontCandidate> candidates,
        InstalledFontRecord? previous,
        CancellationToken cancellationToken = default)
    {
        if (!OperatingSystem.IsWindows())
            throw new PlatformNotSupportedException();

        var fontDirectory = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Microsoft", "Windows", "Fonts");
        Directory.CreateDirectory(fontDirectory);

        var installed = new List<InstalledFontFile>();
        using var fontsKey = Registry.CurrentUser.CreateSubKey(RegistryPath, true);

        foreach (var candidate in candidates)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var targetName =
                $"FontFerry-{SafeSegment(catalogId)}-{SafeSegment(version)}-{Path.GetFileName(candidate.SourceName)}";
            var targetPath = Path.Combine(fontDirectory, targetName);
            await CopyAsync(candidate.SourcePath, targetPath, cancellationToken);

            var primaryFace = candidate.Faces[0];
            var registryName = $"FontFerry:{catalogId}:{primaryFace.FullName}";
            fontsKey.SetValue(registryName, targetPath, RegistryValueKind.String);
            _ = AddFontResourceEx(targetPath, 0, 0);

            installed.Add(new InstalledFontFile(
                candidate.SourceName,
                targetPath,
                registryName,
                primaryFace.Family,
                primaryFace.Subfamily,
                primaryFace.Version,
                primaryFace.PostScriptName,
                candidate.Sha256));
        }

        BroadcastFontChange();
        CleanupPreviousFiles(previous, installed.Select(file => file.InstalledPath).ToHashSet(
            StringComparer.OrdinalIgnoreCase));

        return new PlatformInstallResult(
            installed,
            previous is not null,
            previous is null
                ? []
                : ["Restart applications that use this font; signing out may be required on Windows."]);
    }

    public Task UninstallAsync(
        InstalledFontRecord record,
        CancellationToken cancellationToken = default)
    {
        if (!OperatingSystem.IsWindows())
            throw new PlatformNotSupportedException();

        using var fontsKey = Registry.CurrentUser.CreateSubKey(RegistryPath, true);
        foreach (var file in record.Files)
        {
            cancellationToken.ThrowIfCancellationRequested();
            fontsKey.DeleteValue(file.RegistryName, false);
            _ = RemoveFontResourceEx(file.InstalledPath, 0, 0);
            TryDelete(file.InstalledPath);
        }

        BroadcastFontChange();
        return Task.CompletedTask;
    }

    private static void CleanupPreviousFiles(
        InstalledFontRecord? previous,
        IReadOnlySet<string> retained)
    {
        if (previous is null)
            return;

        foreach (var file in previous.Files)
        {
            if (retained.Contains(file.InstalledPath))
                continue;
            _ = RemoveFontResourceEx(file.InstalledPath, 0, 0);
            TryDelete(file.InstalledPath);
        }
    }

    private static async Task CopyAsync(
        string source,
        string target,
        CancellationToken cancellationToken)
    {
        await using var input = File.OpenRead(source);
        await using var output = new FileStream(
            target, FileMode.Create, FileAccess.Write, FileShare.Read,
            1024 * 128, FileOptions.Asynchronous);
        await input.CopyToAsync(output, cancellationToken);
    }

    private static void TryDelete(string path)
    {
        try
        {
            if (File.Exists(path))
                File.Delete(path);
        }
        catch (IOException)
        {
            // A running application can keep an old Windows font loaded.
        }
        catch (UnauthorizedAccessException)
        {
            // Leave the stale version in place; it is no longer registered.
        }
    }

    private static string SafeSegment(string value) =>
        string.Concat(value.Select(character =>
            char.IsAsciiLetterOrDigit(character) || character is '-' or '.'
                ? character
                : '_'));

    private static void BroadcastFontChange() =>
        _ = SendMessageTimeout(
            new IntPtr(0xffff), WmFontChange, IntPtr.Zero, IntPtr.Zero,
            SmtoAbortIfHung, 1000, out _);

    [DllImport("gdi32.dll", EntryPoint = "AddFontResourceExW",
        CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int AddFontResourceEx(string fileName, uint flags, nint reserved);

    [DllImport("gdi32.dll", EntryPoint = "RemoveFontResourceExW",
        CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool RemoveFontResourceEx(string fileName, uint flags, nint reserved);

    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW",
        CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern nint SendMessageTimeout(
        nint window,
        uint message,
        nint wordParameter,
        nint longParameter,
        uint flags,
        uint timeout,
        out nint result);
}
