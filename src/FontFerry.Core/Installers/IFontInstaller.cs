using FontFerry.Core.Models;

namespace FontFerry.Core.Installers;

public interface IFontInstaller
{
    Task<PlatformInstallResult> InstallAsync(
        string catalogId,
        string version,
        IReadOnlyList<FontCandidate> candidates,
        InstalledFontRecord? previous,
        CancellationToken cancellationToken = default);

    Task UninstallAsync(
        InstalledFontRecord record,
        CancellationToken cancellationToken = default);
}

public sealed record FontCandidate(
    string SourcePath,
    string SourceName,
    string Sha256,
    IReadOnlyList<FontMetadata> Faces);

public sealed record PlatformInstallResult(
    IReadOnlyList<InstalledFontFile> Files,
    bool RestartRecommended,
    IReadOnlyList<string> Warnings);

