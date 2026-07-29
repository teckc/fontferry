using System.IO.Compression;
using FontFerry.Core;

namespace FontFerry.Tests;

public sealed class SafeArchiveExtractorTests : IDisposable
{
    private readonly string _root =
        Path.Combine(Path.GetTempPath(), "FontFerry.Tests", Guid.NewGuid().ToString("N"));

    [Fact]
    public async Task ExtractFontsAsync_RejectsPathTraversal()
    {
        Directory.CreateDirectory(_root);
        var archivePath = Path.Combine(_root, "unsafe.zip");
        using (var archive = ZipFile.Open(archivePath, ZipArchiveMode.Create))
        {
            var entry = archive.CreateEntry("../escaped.ttf");
            await using var stream = entry.Open();
            await stream.WriteAsync(new byte[] { 0, 1, 2, 3 });
        }

        var extractor = new SafeArchiveExtractor();
        await Assert.ThrowsAsync<InvalidDataException>(() =>
            extractor.ExtractFontsAsync(
                new DownloadedAsset("unsafe.zip", archivePath, "", 4),
                Path.Combine(_root, "out")));
        Assert.False(File.Exists(Path.Combine(_root, "escaped.ttf")));
    }

    [Fact]
    public async Task ExtractFontsAsync_ExtractsOnlyFontFiles()
    {
        Directory.CreateDirectory(_root);
        var archivePath = Path.Combine(_root, "fonts.zip");
        using (var archive = ZipFile.Open(archivePath, ZipArchiveMode.Create))
        {
            await WriteEntryAsync(archive, "fonts/example.ttf", [0, 1, 2, 3]);
            await WriteEntryAsync(archive, "README.txt", [4, 5, 6]);
        }

        var extractor = new SafeArchiveExtractor();
        var files = await extractor.ExtractFontsAsync(
            new DownloadedAsset("fonts.zip", archivePath, "", 7),
            Path.Combine(_root, "out"));

        Assert.Single(files);
        Assert.EndsWith("example.ttf", files[0]);
        Assert.False(File.Exists(Path.Combine(_root, "out", "README.txt")));
    }

    private static async Task WriteEntryAsync(
        ZipArchive archive,
        string name,
        byte[] contents)
    {
        var entry = archive.CreateEntry(name);
        await using var stream = entry.Open();
        await stream.WriteAsync(contents);
    }

    public void Dispose()
    {
        if (Directory.Exists(_root))
            Directory.Delete(_root, true);
    }
}
