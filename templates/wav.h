// RIFF WAVE audio (.wav)
// Root: "(template declarations)", offset 0.
#pragma pack(1)

enum AudioFormat : uint16_t {
    PCM = 1, ADPCM = 2, IEEE_FLOAT = 3, ALAW = 6, MULAW = 7, EXTENSIBLE = 0xFFFE
};

struct RiffHeader {
    char     id[4];              // "RIFF"
    uint32_t size;
    char     format[4];          // "WAVE"
};

struct FmtChunk {
    char     id[4];              // "fmt "
    uint32_t size;
    enum AudioFormat format;
    uint16_t channels;
    uint32_t sampleRate;
    uint32_t byteRate;
    uint16_t blockAlign;
    uint16_t bitsPerSample;
    uint8_t  extra[size - 16];
};

// chunks are padded to an even size
struct Chunk {
    char     id[4];
    uint32_t size;
    uint8_t  data[size + (size & 1)];
};

struct RiffHeader riff;
struct FmtChunk   fmt;
struct Chunk      chunk1;
struct Chunk      chunk2;
