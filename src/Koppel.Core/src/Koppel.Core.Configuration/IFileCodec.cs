using System.Text;

namespace Koppel.Core.Configuration;

/// <summary>Turns a file's JSON text into the bytes on disk and back. Lets a config file be encrypted.</summary>
public interface IFileCodec
{
    byte[] Encode(string json);

    string Decode(byte[] bytes);
}

/// <summary>Plain UTF-8, for files people may read and edit by hand.</summary>
public sealed class PlainTextCodec : IFileCodec
{
    public static PlainTextCodec Instance { get; } = new();

    public byte[] Encode(string json) => Encoding.UTF8.GetBytes(json);

    public string Decode(byte[] bytes) => Encoding.UTF8.GetString(bytes);
}
