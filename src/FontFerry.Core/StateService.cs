using System.Text.Json;
using FontFerry.Core.Models;

namespace FontFerry.Core;

public sealed class StateService
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        WriteIndented = true
    };

    private readonly AppPaths _paths;
    private readonly SemaphoreSlim _gate = new(1, 1);

    public StateService(AppPaths paths) => _paths = paths;

    public async Task<FontFerryState> ReadAsync(CancellationToken cancellationToken = default)
    {
        await _gate.WaitAsync(cancellationToken);
        try
        {
            if (!File.Exists(_paths.StateFile))
                return new FontFerryState();

            await using var stream = File.OpenRead(_paths.StateFile);
            return await JsonSerializer.DeserializeAsync<FontFerryState>(
                       stream, JsonOptions, cancellationToken)
                   ?? new FontFerryState();
        }
        finally
        {
            _gate.Release();
        }
    }

    public async Task WriteAsync(
        FontFerryState state,
        CancellationToken cancellationToken = default)
    {
        await _gate.WaitAsync(cancellationToken);
        try
        {
            var temporaryPath = _paths.StateFile + ".tmp";
            await using (var stream = File.Create(temporaryPath))
            {
                await JsonSerializer.SerializeAsync(stream, state, JsonOptions, cancellationToken);
            }

            File.Move(temporaryPath, _paths.StateFile, true);
        }
        finally
        {
            _gate.Release();
        }
    }
}
