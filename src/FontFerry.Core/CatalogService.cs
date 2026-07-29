using System.Text.Json;
using System.Text.RegularExpressions;
using FontFerry.Core.Models;

namespace FontFerry.Core;

public sealed partial class CatalogService
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        WriteIndented = true
    };

    private readonly string _builtInCatalogRoot;
    private readonly AppPaths _paths;

    public CatalogService(string builtInCatalogRoot, AppPaths paths)
    {
        _builtInCatalogRoot = builtInCatalogRoot;
        _paths = paths;
    }

    public async Task<IReadOnlyList<FontCatalogEntry>> GetAllAsync(
        CancellationToken cancellationToken = default)
    {
        var entries = new Dictionary<string, FontCatalogEntry>(StringComparer.OrdinalIgnoreCase);
        await LoadDirectoryAsync(_builtInCatalogRoot, true, entries, cancellationToken);
        await LoadDirectoryAsync(_paths.UserCatalogRoot, false, entries, cancellationToken);
        return entries.Values.OrderBy(entry => entry.Name, StringComparer.OrdinalIgnoreCase).ToArray();
    }

    public async Task<FontCatalogEntry> GetRequiredAsync(
        string id,
        CancellationToken cancellationToken = default)
    {
        var entry = (await GetAllAsync(cancellationToken))
            .FirstOrDefault(item => item.Id.Equals(id, StringComparison.OrdinalIgnoreCase));
        return entry ?? throw new KeyNotFoundException($"Unknown font catalog entry '{id}'.");
    }

    public async Task<FontCatalogEntry> AddGitHubAsync(
        AddGitHubCatalogRequest request,
        CancellationToken cancellationToken = default)
    {
        ValidateId(request.Id);
        ValidateRepository(request.Repository);

        var entry = new FontCatalogEntry
        {
            Id = request.Id,
            Name = request.Name.Trim(),
            Homepage = request.Homepage.Trim(),
            License = new FontLicense
            {
                Name = request.LicenseName.Trim(),
                Url = request.LicenseUrl.Trim(),
                Redistribution = false
            },
            Source = new FontSource
            {
                Type = FontSourceType.GitHubRelease,
                Repository = request.Repository.Trim(),
                Channel = request.Channel
            }
        };

        var path = Path.Combine(_paths.UserCatalogRoot, $"{entry.Id}.json");
        await using var stream = File.Create(path);
        await JsonSerializer.SerializeAsync(stream, entry, JsonOptions, cancellationToken);
        return entry;
    }

    public static void ValidateId(string id)
    {
        if (!IdPattern().IsMatch(id))
            throw new ArgumentException(
                "Font ID must contain 2-64 lowercase ASCII letters, numbers, or hyphens.");
    }

    public static void ValidateRepository(string repository)
    {
        if (!RepositoryPattern().IsMatch(repository))
            throw new ArgumentException("Repository must use the 'owner/name' format.");
    }

    private static async Task LoadDirectoryAsync(
        string directory,
        bool builtIn,
        IDictionary<string, FontCatalogEntry> entries,
        CancellationToken cancellationToken)
    {
        if (!Directory.Exists(directory))
            return;

        foreach (var path in Directory.EnumerateFiles(directory, "*.json").Order())
        {
            await using var stream = File.OpenRead(path);
            var entry = await JsonSerializer.DeserializeAsync<FontCatalogEntry>(
                stream, JsonOptions, cancellationToken);
            if (entry is null)
                throw new InvalidDataException($"Catalog file '{path}' is empty.");

            ValidateId(entry.Id);
            if (entry.Source.Type == FontSourceType.GitHubRelease)
                ValidateRepository(entry.Source.Repository ?? string.Empty);

            entry.BuiltIn = builtIn;
            entries[entry.Id] = entry;
        }
    }

    [GeneratedRegex("^[a-z0-9][a-z0-9-]{1,63}$", RegexOptions.CultureInvariant)]
    private static partial Regex IdPattern();

    [GeneratedRegex("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$", RegexOptions.CultureInvariant)]
    private static partial Regex RepositoryPattern();
}

