namespace FontFerry.Core.Models;

public sealed record FontRelease(
    string Tag,
    string Name,
    DateTimeOffset PublishedAt,
    bool Prerelease,
    IReadOnlyList<ReleaseAsset> Assets);

public sealed record ReleaseAsset(
    string Name,
    string DownloadUrl,
    long Size,
    string? Digest,
    string ContentType);

public sealed record StaticAssetInfo(
    string Name,
    string Url,
    long? Size,
    string? ETag,
    DateTimeOffset? LastModified);

