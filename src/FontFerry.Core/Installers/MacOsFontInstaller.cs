using FontFerry.Core.Models;

namespace FontFerry.Core.Installers;

public sealed class MacOsFontInstaller : IFontInstaller
{
    public async Task<PlatformInstallResult> InstallAsync(
        string catalogId,
        string version,
        IReadOnlyList<FontCandidate> candidates,
        InstalledFontRecord? previous,
        CancellationToken cancellationToken = default)
    {
        if (!OperatingSystem.IsMacOS())
            throw new PlatformNotSupportedException();

        var fontDirectory = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
            "Library", "Fonts");
        Directory.CreateDirectory(fontDirectory);
        var installed = new List<InstalledFontFile>();

        foreach (var candidate in candidates)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var targetName = $"FontFerry-{SafeSegment(catalogId)}-{Path.GetFileName(candidate.SourceName)}";
            var targetPath = Path.Combine(fontDirectory, targetName);
            var temporaryPath = targetPath + $".{Guid.NewGuid():N}.tmp";

            await using (var input = File.OpenRead(candidate.SourcePath))
            await using (var output = new FileStream(
                             temporaryPath, FileMode.CreateNew, FileAccess.Write, FileShare.None,
                             1024 * 128, FileOptions.Asynchronous))
            {
                await input.CopyToAsync(output, cancellationToken);
            }

            File.Move(temporaryPath, targetPath, true);
            var primaryFace = candidate.Faces[0];
            installed.Add(new InstalledFontFile(
                candidate.SourceName,
                targetPath,
                string.Empty,
                primaryFace.Family,
                primaryFace.Subfamily,
                primaryFace.Version,
                primaryFace.PostScriptName,
                candidate.Sha256));
        }

        CleanupPreviousFiles(previous, installed.Select(file => file.InstalledPath).ToHashSet(
            StringComparer.Ordinal));
        return new PlatformInstallResult(
            installed,
            previous is not null,
            previous is null ? [] : ["Restart applications that use this font."]);
    }

    public Task UninstallAsync(
        InstalledFontRecord record,
        CancellationToken cancellationToken = default)
    {
        if (!OperatingSystem.IsMacOS())
            throw new PlatformNotSupportedException();

        foreach (var file in record.Files)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (File.Exists(file.InstalledPath))
                File.Delete(file.InstalledPath);
        }

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
            if (!retained.Contains(file.InstalledPath) && File.Exists(file.InstalledPath))
                File.Delete(file.InstalledPath);
        }
    }

    private static string SafeSegment(string value) =>
        string.Concat(value.Select(character =>
            char.IsAsciiLetterOrDigit(character) || character is '-' or '.'
                ? character
                : '_'));
}

