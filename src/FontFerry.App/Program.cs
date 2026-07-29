using System.Diagnostics;
using System.Text.Json;
using System.Text.Json.Serialization;
using FontFerry.Core;
using FontFerry.Core.Installers;
using FontFerry.Core.Models;

var builder = WebApplication.CreateBuilder(args);
builder.WebHost.UseUrls("http://127.0.0.1:43717");

var appPaths = new AppPaths(Environment.GetEnvironmentVariable("FONTFERRY_DATA_DIR"));
builder.Services.ConfigureHttpJsonOptions(options =>
{
    options.SerializerOptions.PropertyNamingPolicy = JsonNamingPolicy.CamelCase;
    options.SerializerOptions.Converters.Add(new JsonStringEnumConverter());
});
builder.Services.AddSingleton(appPaths);
builder.Services.AddSingleton(provider => new CatalogService(
    Path.Combine(AppContext.BaseDirectory, "catalog"),
    provider.GetRequiredService<AppPaths>()));
builder.Services.AddSingleton<StateService>();
builder.Services.AddHttpClient<GitHubReleaseClient>();
builder.Services.AddHttpClient<DownloadService>();
builder.Services.AddSingleton<SafeArchiveExtractor>();
builder.Services.AddSingleton<FontInspector>();
builder.Services.AddSingleton<WindowsFontInstaller>();
builder.Services.AddSingleton<MacOsFontInstaller>();
builder.Services.AddSingleton<FontInstallerFactory>();
builder.Services.AddSingleton<FontManager>();
builder.Services.AddSingleton<ScheduleService>();

var app = builder.Build();
var command = args.FirstOrDefault(argument => !argument.StartsWith('-')) ?? "serve";

if (command.Equals("update-all", StringComparison.OrdinalIgnoreCase))
{
    var manager = app.Services.GetRequiredService<FontManager>();
    var results = await manager.UpdateAllAsync();
    Console.WriteLine(JsonSerializer.Serialize(results, new JsonSerializerOptions
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase
    }));
    Environment.ExitCode = results.All(result => result.Success) ? 0 : 1;
    return;
}

app.UseExceptionHandler(exceptionApp =>
{
    exceptionApp.Run(async context =>
    {
        var exception = context.Features
            .Get<Microsoft.AspNetCore.Diagnostics.IExceptionHandlerFeature>()?.Error;
        context.Response.StatusCode = exception switch
        {
            KeyNotFoundException => StatusCodes.Status404NotFound,
            ArgumentException or InvalidDataException or InvalidOperationException =>
                StatusCodes.Status400BadRequest,
            _ => StatusCodes.Status500InternalServerError
        };
        await Results.Problem(
            title: "FontFerry request failed",
            detail: exception?.Message,
            statusCode: context.Response.StatusCode).ExecuteAsync(context);
    });
});

app.Use(async (context, next) =>
{
    if (context.Request.Method is not ("GET" or "HEAD" or "OPTIONS") &&
        context.Request.Headers["X-FontFerry-Request"] != "1")
    {
        context.Response.StatusCode = StatusCodes.Status403Forbidden;
        await context.Response.WriteAsJsonAsync(new { error = "Missing local request header." });
        return;
    }
    await next();
});

app.UseDefaultFiles();
app.UseStaticFiles();

app.MapGet("/api/system", (AppPaths paths) => new
{
    platform = OperatingSystem.IsWindows() ? "windows" :
        OperatingSystem.IsMacOS() ? "macos" : "unsupported",
    version = typeof(Program).Assembly.GetName().Version?.ToString() ?? "dev",
    dataRoot = paths.DataRoot
});

app.MapGet("/api/catalog", async (
    CatalogService catalog,
    CancellationToken cancellationToken) =>
    await catalog.GetAllAsync(cancellationToken));

app.MapPost("/api/catalog", async (
    AddGitHubCatalogRequest request,
    CatalogService catalog,
    GitHubReleaseClient github,
    CancellationToken cancellationToken) =>
{
    _ = await github.GetReleasesAsync(request.Repository, cancellationToken: cancellationToken);
    return Results.Created(
        $"/api/catalog/{request.Id}",
        await catalog.AddGitHubAsync(request, cancellationToken));
});

app.MapGet("/api/catalog/{id}/releases", async (
    string id,
    FontManager manager,
    CancellationToken cancellationToken) =>
    await manager.GetReleasesAsync(id, cancellationToken));

app.MapGet("/api/state", async (
    FontManager manager,
    CancellationToken cancellationToken) =>
    await manager.GetStateAsync(cancellationToken));

app.MapPost("/api/catalog/{id}/install", async (
    string id,
    InstallFontRequest request,
    FontManager manager,
    CancellationToken cancellationToken) =>
    await manager.InstallAsync(id, request, cancellationToken));

app.MapDelete("/api/catalog/{id}/install", async (
    string id,
    FontManager manager,
    CancellationToken cancellationToken) =>
{
    await manager.UninstallAsync(id, cancellationToken);
    return Results.NoContent();
});

app.MapPost("/api/catalog/{id}/rollback", async (
    string id,
    FontManager manager,
    CancellationToken cancellationToken) =>
    await manager.RollbackAsync(id, cancellationToken));

app.MapPost("/api/update-all", async (
    FontManager manager,
    CancellationToken cancellationToken) =>
    await manager.UpdateAllAsync(cancellationToken));

app.MapPost("/api/schedule", async (
    ScheduleRequest request,
    ScheduleService schedule,
    CancellationToken cancellationToken) =>
{
    await schedule.ConfigureAsync(request.Enabled, cancellationToken);
    return Results.Ok(new { request.Enabled });
});

app.MapGet("/api/schedule", async (
    ScheduleService schedule,
    CancellationToken cancellationToken) =>
    new { enabled = await schedule.IsEnabledAsync(cancellationToken) });

await app.StartAsync();
if (!args.Contains("--no-browser", StringComparer.OrdinalIgnoreCase))
    OpenBrowser("http://127.0.0.1:43717");
await app.WaitForShutdownAsync();

static void OpenBrowser(string url)
{
    try
    {
        if (OperatingSystem.IsWindows())
        {
            Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });
        }
        else if (OperatingSystem.IsMacOS())
        {
            Process.Start("open", url);
        }
    }
    catch
    {
        Console.WriteLine($"Open {url} in a browser.");
    }
}

public sealed record ScheduleRequest(bool Enabled);
