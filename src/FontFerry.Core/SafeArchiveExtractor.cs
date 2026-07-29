using SharpCompress.Archives;
using SharpCompress.Readers;

namespace FontFerry.Core;

public sealed class SafeArchiveExtractor
{
    private static readonly HashSet<string> FontExtensions =
        new(StringComparer.OrdinalIgnoreCase) { ".ttf", ".otf", ".ttc", ".otc" };

    public async Task<IReadOnlyList<string>> ExtractFontsAsync(
        DownloadedAsset asset,
        string destination,
        CancellationToken cancellationToken = default)
    {
        Directory.CreateDirectory(destination);
        var extension = Path.GetExtension(asset.Path);

        if (FontExtensions.Contains(extension))
        {
            var target = Path.Combine(destination, Path.GetFileName(asset.Path));
            File.Copy(asset.Path, target, true);
            return [target];
        }

        if (!extension.Equals(".zip", StringComparison.OrdinalIgnoreCase) &&
            !extension.Equals(".7z", StringComparison.OrdinalIgnoreCase))
        {
            throw new NotSupportedException(
                $"Asset '{asset.Name}' is not a supported font, ZIP, or 7z file.");
        }

        var root = Path.GetFullPath(destination) + Path.DirectorySeparatorChar;
        var extracted = new List<string>();
        using var archive = ArchiveFactory.OpenArchive(asset.Path, new ReaderOptions());

        foreach (var entry in archive.Entries.Where(item => !item.IsDirectory))
        {
            cancellationToken.ThrowIfCancellationRequested();
            var entryKey = entry.Key
                ?? throw new InvalidDataException("Archive contains an entry with no name.");
            var key = entryKey.Replace('/', Path.DirectorySeparatorChar);
            var target = Path.GetFullPath(Path.Combine(destination, key));
            if (!target.StartsWith(root, StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException($"Archive entry '{entryKey}' escapes its destination.");

            if (!FontExtensions.Contains(Path.GetExtension(target)))
                continue;

            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            await using var input = entry.OpenEntryStream();
            await using var output = new FileStream(
                target, FileMode.Create, FileAccess.Write, FileShare.None,
                1024 * 128, FileOptions.Asynchronous);
            await input.CopyToAsync(output, cancellationToken);
            extracted.Add(target);
        }

        if (extracted.Count == 0)
            throw new InvalidDataException($"Asset '{asset.Name}' contains no supported font files.");

        return extracted;
    }
}
