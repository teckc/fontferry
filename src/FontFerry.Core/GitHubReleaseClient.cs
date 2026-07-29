using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json.Serialization;
using FontFerry.Core.Models;

namespace FontFerry.Core;

public sealed class GitHubReleaseClient
{
    private readonly HttpClient _httpClient;

    public GitHubReleaseClient(HttpClient httpClient)
    {
        _httpClient = httpClient;
        _httpClient.DefaultRequestHeaders.UserAgent.ParseAdd("FontFerry/0.1");
        _httpClient.DefaultRequestHeaders.Accept.Add(
            new MediaTypeWithQualityHeaderValue("application/vnd.github+json"));
        _httpClient.DefaultRequestHeaders.TryAddWithoutValidation(
            "X-GitHub-Api-Version", "2022-11-28");

        var token = Environment.GetEnvironmentVariable("GITHUB_TOKEN");
        if (!string.IsNullOrWhiteSpace(token))
            _httpClient.DefaultRequestHeaders.Authorization =
                new AuthenticationHeaderValue("Bearer", token);
    }

    public async Task<IReadOnlyList<FontRelease>> GetReleasesAsync(
        string repository,
        int count = 10,
        CancellationToken cancellationToken = default)
    {
        CatalogService.ValidateRepository(repository);
        count = Math.Clamp(count, 1, 30);

        var releases = await _httpClient.GetFromJsonAsync<GitHubReleaseDto[]>(
            $"https://api.github.com/repos/{repository}/releases?per_page={count}",
            cancellationToken) ?? [];

        return releases
            .Where(release => !release.Draft)
            .Select(release => new FontRelease(
                release.TagName,
                string.IsNullOrWhiteSpace(release.Name) ? release.TagName : release.Name,
                release.PublishedAt,
                release.Prerelease,
                release.Assets.Select(asset => new ReleaseAsset(
                    asset.Name,
                    asset.BrowserDownloadUrl,
                    asset.Size,
                    asset.Digest,
                    asset.ContentType)).ToArray()))
            .ToArray();
    }

    public async Task<FontRelease> GetSelectedReleaseAsync(
        FontCatalogEntry entry,
        string? requestedTag,
        CancellationToken cancellationToken = default)
    {
        if (entry.Source.Type != FontSourceType.GitHubRelease)
            throw new InvalidOperationException($"'{entry.Id}' is not a GitHub Release source.");

        var releases = await GetReleasesAsync(
            entry.Source.Repository ?? string.Empty, cancellationToken: cancellationToken);

        FontRelease? selected;
        if (!string.IsNullOrWhiteSpace(requestedTag))
        {
            selected = releases.FirstOrDefault(release =>
                release.Tag.Equals(requestedTag, StringComparison.OrdinalIgnoreCase));
        }
        else if (entry.Source.Channel == ReleaseChannel.Prerelease)
        {
            selected = releases.FirstOrDefault();
        }
        else
        {
            selected = releases.FirstOrDefault(release => !release.Prerelease);
        }

        return selected ?? throw new InvalidOperationException(
            $"No matching release was found for '{entry.Source.Repository}'.");
    }

    private sealed record GitHubReleaseDto
    {
        [JsonPropertyName("tag_name")]
        public required string TagName { get; init; }

        [JsonPropertyName("name")]
        public string? Name { get; init; }

        [JsonPropertyName("draft")]
        public bool Draft { get; init; }

        [JsonPropertyName("prerelease")]
        public bool Prerelease { get; init; }

        [JsonPropertyName("published_at")]
        public DateTimeOffset PublishedAt { get; init; }

        [JsonPropertyName("assets")]
        public GitHubAssetDto[] Assets { get; init; } = [];
    }

    private sealed record GitHubAssetDto
    {
        [JsonPropertyName("name")]
        public required string Name { get; init; }

        [JsonPropertyName("browser_download_url")]
        public required string BrowserDownloadUrl { get; init; }

        [JsonPropertyName("size")]
        public long Size { get; init; }

        [JsonPropertyName("digest")]
        public string? Digest { get; init; }

        [JsonPropertyName("content_type")]
        public string ContentType { get; init; } = "application/octet-stream";
    }
}

