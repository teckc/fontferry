using System.Net;
using System.Security.Cryptography;
using FontFerry.Core.Models;

namespace FontFerry.Core;

public sealed class DownloadService
{
    public const long DefaultMaximumBytes = 2L * 1024 * 1024 * 1024;

    private readonly HttpClient _httpClient;
    private readonly AppPaths _paths;

    public DownloadService(HttpClient httpClient, AppPaths paths)
    {
        _httpClient = httpClient;
        _paths = paths;
        _httpClient.DefaultRequestHeaders.UserAgent.ParseAdd("FontFerry/0.1");
        _httpClient.Timeout = TimeSpan.FromHours(2);
    }

    public async Task<DownloadedAsset> DownloadAsync(
        string catalogId,
        string version,
        ReleaseAsset asset,
        CancellationToken cancellationToken = default)
    {
        if (asset.Size < 0 || asset.Size > DefaultMaximumBytes)
            throw new InvalidDataException(
                $"Asset '{asset.Name}' exceeds the {DefaultMaximumBytes} byte safety limit.");

        var safeName = GetSafeFileName(asset.Name);
        var directory = Path.Combine(
            _paths.CacheRoot, SafeSegment(catalogId), SafeSegment(version));
        Directory.CreateDirectory(directory);

        var targetPath = Path.Combine(directory, safeName);
        var partialPath = targetPath + ".partial";
        var existingLength = File.Exists(partialPath) ? new FileInfo(partialPath).Length : 0;

        using var request = new HttpRequestMessage(HttpMethod.Get, asset.DownloadUrl);
        if (existingLength > 0)
            request.Headers.Range = new System.Net.Http.Headers.RangeHeaderValue(existingLength, null);

        using var response = await _httpClient.SendAsync(
            request, HttpCompletionOption.ResponseHeadersRead, cancellationToken);

        if (existingLength > 0 && response.StatusCode == HttpStatusCode.OK)
        {
            existingLength = 0;
            File.Delete(partialPath);
        }

        response.EnsureSuccessStatusCode();
        var reportedLength = response.Content.Headers.ContentLength;
        if (reportedLength is > DefaultMaximumBytes ||
            existingLength + reportedLength.GetValueOrDefault() > DefaultMaximumBytes)
        {
            throw new InvalidDataException($"Asset '{asset.Name}' is too large.");
        }

        await using (var output = new FileStream(
                         partialPath,
                         existingLength > 0 ? FileMode.Append : FileMode.Create,
                         FileAccess.Write,
                         FileShare.None,
                         1024 * 128,
                         FileOptions.Asynchronous | FileOptions.SequentialScan))
        await using (var input = await response.Content.ReadAsStreamAsync(cancellationToken))
        {
            await input.CopyToAsync(output, cancellationToken);
        }

        var finalLength = new FileInfo(partialPath).Length;
        if (asset.Size > 0 && finalLength != asset.Size)
            throw new InvalidDataException(
                $"Downloaded size mismatch for '{asset.Name}': expected {asset.Size}, got {finalLength}.");

        var sha256 = await ComputeSha256Async(partialPath, cancellationToken);
        VerifyDigest(asset, sha256);
        File.Move(partialPath, targetPath, true);
        return new DownloadedAsset(asset.Name, targetPath, sha256, finalLength);
    }

    public async Task<StaticAssetInfo> InspectStaticAsync(
        string url,
        CancellationToken cancellationToken = default)
    {
        using var request = new HttpRequestMessage(HttpMethod.Head, url);
        using var response = await _httpClient.SendAsync(request, cancellationToken);
        response.EnsureSuccessStatusCode();

        return new StaticAssetInfo(
            GetSafeFileName(new Uri(url).LocalPath),
            url,
            response.Content.Headers.ContentLength,
            response.Headers.ETag?.Tag,
            response.Content.Headers.LastModified);
    }

    public static async Task<string> ComputeSha256Async(
        string path,
        CancellationToken cancellationToken = default)
    {
        await using var stream = new FileStream(
            path, FileMode.Open, FileAccess.Read, FileShare.Read,
            1024 * 128, FileOptions.Asynchronous | FileOptions.SequentialScan);
        var digest = await SHA256.HashDataAsync(stream, cancellationToken);
        return Convert.ToHexStringLower(digest);
    }

    private static void VerifyDigest(ReleaseAsset asset, string sha256)
    {
        if (string.IsNullOrWhiteSpace(asset.Digest))
            return;

        var pieces = asset.Digest.Split(':', 2);
        if (pieces.Length != 2 || !pieces[0].Equals("sha256", StringComparison.OrdinalIgnoreCase))
            return;

        if (!pieces[1].Equals(sha256, StringComparison.OrdinalIgnoreCase))
            throw new CryptographicException($"SHA-256 verification failed for '{asset.Name}'.");
    }

    private static string GetSafeFileName(string name)
    {
        var value = Path.GetFileName(name);
        if (string.IsNullOrWhiteSpace(value) || value.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0)
            throw new InvalidDataException($"Unsafe asset name '{name}'.");
        return value;
    }

    private static string SafeSegment(string value) =>
        string.Concat(value.Select(character =>
            char.IsAsciiLetterOrDigit(character) || character is '-' or '.' ? character : '_'));
}

public sealed record DownloadedAsset(string Name, string Path, string Sha256, long Size);

