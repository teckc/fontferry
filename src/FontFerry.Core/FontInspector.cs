using System.Buffers.Binary;
using System.Text;

namespace FontFerry.Core;

public sealed class FontInspector
{
    private static readonly byte[] TtcTag = "ttcf"u8.ToArray();

    public IReadOnlyList<FontMetadata> Inspect(string path)
    {
        using var stream = File.OpenRead(path);
        Span<byte> tag = stackalloc byte[4];
        ReadExactly(stream, tag);
        stream.Position = 0;

        if (tag.SequenceEqual(TtcTag))
            return InspectCollection(stream);

        return [InspectFace(stream, 0)];
    }

    private static IReadOnlyList<FontMetadata> InspectCollection(Stream stream)
    {
        Span<byte> header = stackalloc byte[12];
        ReadExactly(stream, header);
        var count = ReadUInt32(header[8..12]);
        if (count is 0 or > 256)
            throw new InvalidDataException($"Invalid font collection count '{count}'.");

        var offsets = new uint[count];
        Span<byte> offsetBuffer = stackalloc byte[4];
        for (var index = 0; index < count; index++)
        {
            ReadExactly(stream, offsetBuffer);
            offsets[index] = ReadUInt32(offsetBuffer);
        }

        return offsets.Select(offset => InspectFace(stream, offset)).ToArray();
    }

    private static FontMetadata InspectFace(Stream stream, long faceOffset)
    {
        stream.Position = faceOffset;
        Span<byte> header = stackalloc byte[12];
        ReadExactly(stream, header);
        var tableCount = ReadUInt16(header[4..6]);
        if (tableCount is 0 or > 512)
            throw new InvalidDataException($"Invalid OpenType table count '{tableCount}'.");

        uint? nameOffset = null;
        uint? nameLength = null;
        Span<byte> table = stackalloc byte[16];
        for (var index = 0; index < tableCount; index++)
        {
            ReadExactly(stream, table);
            if (table[..4].SequenceEqual("name"u8))
            {
                nameOffset = ReadUInt32(table[8..12]);
                nameLength = ReadUInt32(table[12..16]);
            }
        }

        if (nameOffset is null || nameLength is null || nameLength < 6)
            throw new InvalidDataException("Font has no valid OpenType name table.");

        return ReadNames(stream, nameOffset.Value, nameLength.Value);
    }

    private static FontMetadata ReadNames(Stream stream, uint offset, uint length)
    {
        if (offset + length > stream.Length)
            throw new InvalidDataException("Font name table is outside the file.");

        stream.Position = offset;
        Span<byte> header = stackalloc byte[6];
        ReadExactly(stream, header);
        var count = ReadUInt16(header[2..4]);
        var stringsOffset = ReadUInt16(header[4..6]);
        if (count > 4096)
            throw new InvalidDataException("Font contains too many name records.");

        var records = new List<NameRecord>(count);
        Span<byte> record = stackalloc byte[12];
        for (var index = 0; index < count; index++)
        {
            ReadExactly(stream, record);
            records.Add(new NameRecord(
                ReadUInt16(record[0..2]),
                ReadUInt16(record[2..4]),
                ReadUInt16(record[4..6]),
                ReadUInt16(record[6..8]),
                ReadUInt16(record[8..10]),
                ReadUInt16(record[10..12])));
        }

        string ReadName(ushort nameId, string fallback)
        {
            var candidates = records
                .Where(item => item.NameId == nameId)
                .OrderByDescending(item => item.PlatformId is 0 or 3)
                .ThenByDescending(item => item.LanguageId == 0x0409)
                .ToArray();

            foreach (var item in candidates)
            {
                var stringPosition = (long)offset + stringsOffset + item.Offset;
                if (stringPosition < offset || stringPosition + item.Length > offset + length)
                    continue;

                stream.Position = stringPosition;
                var bytes = new byte[item.Length];
                ReadExactly(stream, bytes);
                var value = DecodeName(item.PlatformId, bytes).Trim('\0', ' ');
                if (!string.IsNullOrWhiteSpace(value))
                    return value;
            }

            return fallback;
        }

        var family = ReadName(1, "Unknown family");
        var subfamily = ReadName(2, "Regular");
        return new FontMetadata(
            family,
            subfamily,
            ReadName(4, $"{family} {subfamily}"),
            ReadName(5, "Unknown version"),
            ReadName(6, string.Empty) is { Length: > 0 } postScript ? postScript : null);
    }

    private static string DecodeName(ushort platformId, byte[] bytes)
    {
        if (platformId is 0 or 3)
        {
            for (var left = 0; left + 1 < bytes.Length; left += 2)
                (bytes[left], bytes[left + 1]) = (bytes[left + 1], bytes[left]);
            return Encoding.Unicode.GetString(bytes);
        }

        return Encoding.Latin1.GetString(bytes);
    }

    private static ushort ReadUInt16(ReadOnlySpan<byte> value) =>
        BinaryPrimitives.ReadUInt16BigEndian(value);

    private static uint ReadUInt32(ReadOnlySpan<byte> value) =>
        BinaryPrimitives.ReadUInt32BigEndian(value);

    private static void ReadExactly(Stream stream, Span<byte> buffer)
    {
        var read = 0;
        while (read < buffer.Length)
        {
            var count = stream.Read(buffer[read..]);
            if (count == 0)
                throw new EndOfStreamException("Unexpected end of font file.");
            read += count;
        }
    }

    private sealed record NameRecord(
        ushort PlatformId,
        ushort EncodingId,
        ushort LanguageId,
        ushort NameId,
        ushort Length,
        ushort Offset);
}

public sealed record FontMetadata(
    string Family,
    string Subfamily,
    string FullName,
    string Version,
    string? PostScriptName);
