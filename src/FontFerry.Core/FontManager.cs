using System.Text.RegularExpressions;
using FontFerry.Core.Installers;
using FontFerry.Core.Models;

namespace FontFerry.Core;

public sealed class FontManager
{
    private readonly CatalogService _catalog;
    private readonly GitHubReleaseClient _github;
    private readonly DownloadService _downloads;
    private readonly SafeArchiveExtractor _extractor;
    private readonly FontInspector _inspector;
    private readonly FontInstallerFactory _installers;
    private readonly StateService _stateService;
    private readonly AppPaths _paths;
    private readonly SemaphoreSlim _operationGate = new(1, 1);

    public FontManager(
        CatalogService catalog,
        GitHubReleaseClient github,
        DownloadService downloads,
        SafeArchiveExtractor extractor,
        FontInspector inspector,
        FontInstallerFactory installers,
        StateService stateService,
        AppPaths paths)
    {
        _catalog = catalog;
        _github = github;
        _downloads = downloads;
        _extractor = extractor;
        _inspector = inspector;
        _installers = installers;
        _stateService = stateService;
        _paths = paths;
    }

    public Task<FontFerryState> GetStateAsync(CancellationToken cancellationToken = default) =>
        _stateService.ReadAsync(cancellationToken);

    public async Task<IReadOnlyList<FontRelease>> GetReleasesAsync(
        string catalogId,
        CancellationToken cancellationToken = default)
    {
        var entry = await _catalog.GetRequiredAsync(catalogId, cancellationToken);
        if (entry.Source.Type == FontSourceType.StaticUrl)
            return [];
        return await _github.GetReleasesAsync(
            entry.Source.Repository ?? string.Empty, cancellationToken: cancellationToken);
    }

    public async Task<InstallFontResult> InstallAsync(
        string catalogId,
        InstallFontRequest request,
        CancellationToken cancellationToken = default)
    {
        await _operationGate.WaitAsync(cancellationToken);
        try
        {
            var entry = await _catalog.GetRequiredAsync(catalogId, cancellationToken);
            if (entry.License.RequiresAcceptance && !request.AcceptLicense)
                throw new InvalidOperationException(
                    $"The {entry.License.Name} license must be accepted before installation.");

            var resolved = entry.Source.Type switch
            {
                FontSourceType.GitHubRelease =>
                    await ResolveGitHubAssetsAsync(entry, request, cancellationToken),
                FontSourceType.StaticUrl =>
                    await ResolveStaticAssetsAsync(entry, request, cancellationToken),
                _ => throw new NotSupportedException($"Unsupported source '{entry.Source.Type}'.")
            };

            var stagingRoot = Path.Combine(
                _paths.CacheRoot, "staging", $"{entry.Id}-{Guid.NewGuid():N}");
            Directory.CreateDirectory(stagingRoot);

            try
            {
                var candidates = await PrepareCandidatesAsync(
                    entry.Id, resolved.Version, resolved.Assets, stagingRoot, cancellationToken);
                ValidateCandidates(candidates);

                var state = await _stateService.ReadAsync(cancellationToken);
                state.Installed.TryGetValue(entry.Id, out var previous);
                var backupPath = await BackupPreviousAsync(entry.Id, previous, cancellationToken);

                var platformResult = await _installers.GetCurrent().InstallAsync(
                    entry.Id, resolved.Version, candidates, previous, cancellationToken);

                state.Installed[entry.Id] = new InstalledFontRecord
                {
                    CatalogId = entry.Id,
                    Version = resolved.Version,
                    InstalledAt = DateTimeOffset.UtcNow,
                    Assets = resolved.Selection,
                    Files = platformResult.Files,
                    PreviousVersion = previous?.Version,
                    BackupPath = backupPath,
                    RestartRecommended = platformResult.RestartRecommended
                };
                await _stateService.WriteAsync(state, cancellationToken);

                return new InstallFontResult(
                    entry.Id,
                    resolved.Version,
                    platformResult.Files.Count,
                    platformResult.RestartRecommended,
                    platformResult.Warnings);
            }
            finally
            {
                DeleteStagingDirectory(stagingRoot);
            }
        }
        finally
        {
            _operationGate.Release();
        }
    }

    public async Task UninstallAsync(
        string catalogId,
        CancellationToken cancellationToken = default)
    {
        await _operationGate.WaitAsync(cancellationToken);
        try
        {
            var state = await _stateService.ReadAsync(cancellationToken);
            if (!state.Installed.Remove(catalogId, out var record))
                return;
            await _installers.GetCurrent().UninstallAsync(record, cancellationToken);
            await _stateService.WriteAsync(state, cancellationToken);
        }
        finally
        {
            _operationGate.Release();
        }
    }

    public async Task<InstallFontResult> RollbackAsync(
        string catalogId,
        CancellationToken cancellationToken = default)
    {
        await _operationGate.WaitAsync(cancellationToken);
        try
        {
            var state = await _stateService.ReadAsync(cancellationToken);
            if (!state.Installed.TryGetValue(catalogId, out var current) ||
                string.IsNullOrWhiteSpace(current.PreviousVersion) ||
                string.IsNullOrWhiteSpace(current.BackupPath) ||
                !Directory.Exists(current.BackupPath))
            {
                throw new InvalidOperationException("No rollback version is available.");
            }

            var fontPaths = Directory.EnumerateFiles(
                    current.BackupPath, "*", SearchOption.AllDirectories)
                .Where(path => FontExtensions.Contains(Path.GetExtension(path)))
                .ToArray();
            if (fontPaths.Length == 0)
                throw new InvalidDataException("The rollback backup contains no font files.");

            var candidates = new List<FontCandidate>();
            foreach (var path in fontPaths)
            {
                candidates.Add(new FontCandidate(
                    path,
                    Path.GetFileName(path),
                    await DownloadService.ComputeSha256Async(path, cancellationToken),
                    _inspector.Inspect(path)));
            }
            ValidateCandidates(candidates);

            var currentBackup = await BackupPreviousAsync(catalogId, current, cancellationToken);
            var result = await _installers.GetCurrent().InstallAsync(
                catalogId, current.PreviousVersion, candidates, current, cancellationToken);

            state.Installed[catalogId] = new InstalledFontRecord
            {
                CatalogId = catalogId,
                Version = current.PreviousVersion,
                InstalledAt = DateTimeOffset.UtcNow,
                Assets = current.Assets,
                Files = result.Files,
                PreviousVersion = current.Version,
                BackupPath = currentBackup,
                RestartRecommended = result.RestartRecommended
            };
            await _stateService.WriteAsync(state, cancellationToken);

            return new InstallFontResult(
                catalogId,
                current.PreviousVersion,
                result.Files.Count,
                result.RestartRecommended,
                result.Warnings);
        }
        finally
        {
            _operationGate.Release();
        }
    }

    private static readonly HashSet<string> FontExtensions =
        new(StringComparer.OrdinalIgnoreCase) { ".ttf", ".otf", ".ttc", ".otc" };

    public async Task<IReadOnlyList<UpdateFontResult>> UpdateAllAsync(
        CancellationToken cancellationToken = default)
    {
        var state = await _stateService.ReadAsync(cancellationToken);
        var results = new List<UpdateFontResult>();
        foreach (var record in state.Installed.Values.ToArray())
        {
            try
            {
                var result = await InstallAsync(
                    record.CatalogId,
                    new InstallFontRequest(null, record.Assets, true),
                    cancellationToken);
                results.Add(new UpdateFontResult(record.CatalogId, true, result.Version, null));
            }
            catch (Exception exception) when (exception is not OperationCanceledException)
            {
                results.Add(new UpdateFontResult(
                    record.CatalogId, false, record.Version, exception.Message));
            }
        }

        return results;
    }

    private async Task<ResolvedAssets> ResolveGitHubAssetsAsync(
        FontCatalogEntry entry,
        InstallFontRequest request,
        CancellationToken cancellationToken)
    {
        var release = await _github.GetSelectedReleaseAsync(
            entry, request.Version, cancellationToken);
        var selection = request.Assets.Count > 0
            ? request.Assets
            : entry.Presets.Where(preset => preset.Default).Select(preset => preset.Id).ToArray();
        var names = ResolveAssetNames(entry, release.Assets, selection);
        if (names.Count == 0)
            throw new InvalidOperationException("Select at least one release asset.");

        var selected = names.Select(name =>
                release.Assets.FirstOrDefault(asset =>
                    asset.Name.Equals(name, StringComparison.Ordinal)))
            .ToArray();
        if (selected.Any(asset => asset is null))
            throw new InvalidOperationException("One or more selected assets do not exist in the release.");
        var canonicalSelection = names.Select(name =>
        {
            var matchingPresets = entry.Presets.Where(preset =>
                    Regex.IsMatch(name, preset.AssetPattern, RegexOptions.CultureInvariant))
                .Select(preset => preset.Id)
                .ToArray();
            return matchingPresets.Length == 1 ? matchingPresets[0] : name;
        }).ToArray();
        return new ResolvedAssets(release.Tag, selected!, canonicalSelection);
    }

    private async Task<ResolvedAssets> ResolveStaticAssetsAsync(
        FontCatalogEntry entry,
        InstallFontRequest request,
        CancellationToken cancellationToken)
    {
        var selectedPresets = entry.Presets
            .Where(preset => request.Assets.Count == 0
                ? preset.Default
                : request.Assets.Contains(preset.Id, StringComparer.OrdinalIgnoreCase) ||
                  request.Assets.Contains(preset.AssetPattern, StringComparer.OrdinalIgnoreCase))
            .ToArray();

        var primaryStaticUrl = entry.Source.Url;
        (string Name, string? Url)[] urls = selectedPresets.Length > 0
            ? selectedPresets.Select(preset => (
                Name: preset.AssetPattern,
                Url: preset.Url ?? primaryStaticUrl)).ToArray()
            : new[] { (
                Name: Path.GetFileName(new Uri(primaryStaticUrl
                    ?? throw new InvalidDataException("Static source has no URL.")).LocalPath),
                Url: (string?)primaryStaticUrl) };

        var assets = new List<ReleaseAsset>();
        var versionParts = new List<string>();
        foreach (var item in urls)
        {
            if (string.IsNullOrWhiteSpace(item.Url))
                throw new InvalidDataException($"Static asset '{item.Name}' has no URL.");
            var info = await _downloads.InspectStaticAsync(item.Url, cancellationToken);
            if (info.Size is > DownloadService.DefaultMaximumBytes)
                throw new InvalidDataException($"Static asset '{item.Name}' is too large.");
            assets.Add(new ReleaseAsset(
                item.Name, item.Url, info.Size ?? 0, null, "application/octet-stream"));
            versionParts.Add(info.ETag?.Trim('"')
                             ?? info.LastModified?.UtcDateTime.ToString("yyyyMMddHHmmss")
                             ?? entry.Source.Version
                             ?? "unknown");
        }

        var selection = selectedPresets.Select(preset => preset.Id).ToArray();
        return new ResolvedAssets(
            entry.Source.Version ?? string.Join('-', versionParts),
            assets,
            selection.Length > 0 ? selection : assets.Select(asset => asset.Name).ToArray());
    }

    private async Task<IReadOnlyList<FontCandidate>> PrepareCandidatesAsync(
        string catalogId,
        string version,
        IReadOnlyList<ReleaseAsset> assets,
        string stagingRoot,
        CancellationToken cancellationToken)
    {
        var candidates = new List<FontCandidate>();
        for (var assetIndex = 0; assetIndex < assets.Count; assetIndex++)
        {
            var asset = assets[assetIndex];
            var downloaded = await _downloads.DownloadAsync(
                catalogId, version, asset, cancellationToken);
            var extractionPath = Path.Combine(stagingRoot, assetIndex.ToString("D3"));
            var fontPaths = await _extractor.ExtractFontsAsync(
                downloaded, extractionPath, cancellationToken);

            foreach (var fontPath in fontPaths)
            {
                var sha256 = await DownloadService.ComputeSha256Async(fontPath, cancellationToken);
                candidates.Add(new FontCandidate(
                    fontPath,
                    Path.GetFileName(fontPath),
                    sha256,
                    _inspector.Inspect(fontPath)));
            }
        }

        return candidates;
    }

    private static IReadOnlyList<string> ResolveAssetNames(
        FontCatalogEntry entry,
        IReadOnlyList<ReleaseAsset> assets,
        IReadOnlyList<string> selection)
    {
        var names = new List<string>();
        foreach (var token in selection)
        {
            var exact = assets.FirstOrDefault(asset =>
                asset.Name.Equals(token, StringComparison.Ordinal));
            if (exact is not null)
            {
                names.Add(exact.Name);
                continue;
            }

            var preset = entry.Presets.FirstOrDefault(item =>
                item.Id.Equals(token, StringComparison.OrdinalIgnoreCase))
                ?? throw new InvalidOperationException(
                    $"Selection '{token}' is neither an asset nor a preset.");
            var matches = assets.Where(asset =>
                    Regex.IsMatch(asset.Name, preset.AssetPattern, RegexOptions.CultureInvariant))
                .Select(asset => asset.Name)
                .ToArray();
            if (matches.Length != 1)
                throw new InvalidOperationException(
                    $"Default preset '{preset.Id}' matched {matches.Length} release assets.");
            names.Add(matches[0]);
        }

        return names;
    }

    private static void ValidateCandidates(IReadOnlyList<FontCandidate> candidates)
    {
        if (candidates.Count == 0)
            throw new InvalidDataException("No fonts were selected for installation.");

        var duplicates = candidates
            .SelectMany(candidate => candidate.Faces)
            .GroupBy(face => face.PostScriptName ?? $"{face.Family}/{face.Subfamily}",
                StringComparer.OrdinalIgnoreCase)
            .Where(group => group.Count() > 1)
            .Select(group => group.Key)
            .ToArray();
        if (duplicates.Length > 0)
            throw new InvalidDataException(
                $"The selected assets contain duplicate font identities: {string.Join(", ", duplicates)}.");
    }

    private async Task<string?> BackupPreviousAsync(
        string catalogId,
        InstalledFontRecord? previous,
        CancellationToken cancellationToken)
    {
        if (previous is null)
            return null;

        var backupPath = Path.Combine(
            _paths.BackupRoot, catalogId, previous.Version, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(backupPath);

        foreach (var file in previous.Files.Where(file => File.Exists(file.InstalledPath)))
        {
            await using var input = File.OpenRead(file.InstalledPath);
            await using var output = new FileStream(
                Path.Combine(backupPath, Path.GetFileName(file.SourceName)),
                FileMode.Create, FileAccess.Write, FileShare.None,
                1024 * 128, FileOptions.Asynchronous);
            await input.CopyToAsync(output, cancellationToken);
        }

        return backupPath;
    }

    private static void DeleteStagingDirectory(string stagingRoot)
    {
        if (!Directory.Exists(stagingRoot))
            return;

        var fullPath = Path.GetFullPath(stagingRoot);
        var parentName = Path.GetFileName(Path.GetDirectoryName(fullPath));
        if (!string.Equals(parentName, "staging", StringComparison.OrdinalIgnoreCase))
        {
            throw new InvalidOperationException("Refusing to delete an unexpected staging path.");
        }

        Directory.Delete(fullPath, true);
    }

    private sealed record ResolvedAssets(
        string Version,
        IReadOnlyList<ReleaseAsset> Assets,
        IReadOnlyList<string> Selection);
}

public sealed record UpdateFontResult(
    string CatalogId,
    bool Success,
    string Version,
    string? Error);
