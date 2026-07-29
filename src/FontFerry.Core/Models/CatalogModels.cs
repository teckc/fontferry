using System.Text.Json.Serialization;

namespace FontFerry.Core.Models;

public sealed record FontCatalogEntry
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public string? Description { get; init; }
    public required string Homepage { get; init; }
    public required FontLicense License { get; init; }
    public required FontSource Source { get; init; }
    public IReadOnlyList<FontPreset> Presets { get; init; } = [];
    public bool BuiltIn { get; set; }
}

public sealed record FontLicense
{
    public required string Name { get; init; }
    public string? Spdx { get; init; }
    public required string Url { get; init; }
    public bool Redistribution { get; init; }
    public bool RequiresAcceptance { get; init; }
}

public sealed record FontSource
{
    [JsonConverter(typeof(JsonStringEnumConverter<FontSourceType>))]
    public required FontSourceType Type { get; init; }

    public string? Repository { get; init; }

    [JsonConverter(typeof(JsonStringEnumConverter<ReleaseChannel>))]
    public ReleaseChannel Channel { get; init; } = ReleaseChannel.Stable;

    public string? Url { get; init; }
    public string? Version { get; init; }
}

public sealed record FontPreset
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public string? Description { get; init; }
    public required string AssetPattern { get; init; }
    public string? Url { get; init; }
    public IReadOnlyList<string> Include { get; init; } = ["**/*"];
    public bool Default { get; init; }
}

public enum FontSourceType
{
    GitHubRelease,
    StaticUrl
}

public enum ReleaseChannel
{
    Stable,
    Prerelease
}

public sealed record AddGitHubCatalogRequest(
    string Id,
    string Name,
    string Repository,
    string Homepage,
    ReleaseChannel Channel,
    string LicenseName,
    string LicenseUrl);
