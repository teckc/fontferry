using FontFerry.Core;
using FontFerry.Core.Models;

namespace FontFerry.Tests;

public sealed class CatalogServiceTests : IDisposable
{
    private readonly string _root =
        Path.Combine(Path.GetTempPath(), "FontFerry.Tests", Guid.NewGuid().ToString("N"));

    [Fact]
    public void ValidateId_RejectsUnsafeValues()
    {
        Assert.Throws<ArgumentException>(() => CatalogService.ValidateId("../font"));
        Assert.Throws<ArgumentException>(() => CatalogService.ValidateId("UPPERCASE"));
        CatalogService.ValidateId("maple-mono");
    }

    [Theory]
    [InlineData("owner/repository")]
    [InlineData("adobe-fonts/source-han-sans")]
    public void ValidateRepository_AcceptsOwnerAndName(string repository) =>
        CatalogService.ValidateRepository(repository);

    [Fact]
    public async Task AddGitHubAsync_RoundTripsUserEntry()
    {
        var builtIn = Path.Combine(_root, "built-in");
        Directory.CreateDirectory(builtIn);
        var paths = new AppPaths(Path.Combine(_root, "data"));
        var service = new CatalogService(builtIn, paths);

        await service.AddGitHubAsync(new AddGitHubCatalogRequest(
            "test-font",
            "Test Font",
            "owner/repository",
            "https://github.com/owner/repository",
            ReleaseChannel.Stable,
            "OFL-1.1",
            "https://example.test/license"));

        var entry = Assert.Single(await service.GetAllAsync());
        Assert.Equal("test-font", entry.Id);
        Assert.False(entry.BuiltIn);
    }

    public void Dispose()
    {
        if (Directory.Exists(_root))
            Directory.Delete(_root, true);
    }
}

