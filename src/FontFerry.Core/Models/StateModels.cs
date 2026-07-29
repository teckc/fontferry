namespace FontFerry.Core.Models;

public sealed record FontFerryState
{
    public Dictionary<string, InstalledFontRecord> Installed { get; init; } =
        new(StringComparer.OrdinalIgnoreCase);
}

public sealed record InstalledFontRecord
{
    public required string CatalogId { get; init; }
    public required string Version { get; init; }
    public required DateTimeOffset InstalledAt { get; init; }
    public required IReadOnlyList<string> Assets { get; init; }
    public required IReadOnlyList<InstalledFontFile> Files { get; init; }
    public string? PreviousVersion { get; init; }
    public string? BackupPath { get; init; }
    public bool RestartRecommended { get; init; }
}

public sealed record InstalledFontFile(
    string SourceName,
    string InstalledPath,
    string RegistryName,
    string Family,
    string Subfamily,
    string Version,
    string? PostScriptName,
    string Sha256);

public sealed record InstallFontRequest(
    string? Version,
    IReadOnlyList<string> Assets,
    bool AcceptLicense = false);

public sealed record InstallFontResult(
    string CatalogId,
    string Version,
    int FileCount,
    bool RestartRecommended,
    IReadOnlyList<string> Warnings);

