using System.Net;
using System.Text;
using FontFerry.Core;

namespace FontFerry.Tests;

public sealed class GitHubReleaseClientTests
{
    [Fact]
    public async Task GetReleasesAsync_MapsStableAndPrereleaseAssets()
    {
        const string json = """
                            [
                              {
                                "tag_name": "v2.0",
                                "name": "Version 2",
                                "draft": false,
                                "prerelease": true,
                                "published_at": "2026-01-01T00:00:00Z",
                                "assets": [
                                  {
                                    "name": "font.zip",
                                    "browser_download_url": "https://example.test/font.zip",
                                    "size": 42,
                                    "digest": "sha256:abc",
                                    "content_type": "application/zip"
                                  }
                                ]
                              }
                            ]
                            """;
        var handler = new StaticHandler(json);
        var client = new GitHubReleaseClient(new HttpClient(handler));

        var release = Assert.Single(await client.GetReleasesAsync("owner/repository"));

        Assert.True(release.Prerelease);
        Assert.Equal("v2.0", release.Tag);
        Assert.Equal("sha256:abc", Assert.Single(release.Assets).Digest);
    }

    private sealed class StaticHandler(string json) : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request,
            CancellationToken cancellationToken) =>
            Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK)
            {
                Content = new StringContent(json, Encoding.UTF8, "application/json")
            });
    }
}
