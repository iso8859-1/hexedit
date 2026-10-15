// PNG image: signature, IHDR and the following chunks.
// Root: "(template declarations)", offset 0.
#pragma pack(1)
#pragma endian big

enum ColorType : uint8_t {
    GRAYSCALE = 0, RGB = 2, PALETTE = 3, GRAYSCALE_ALPHA = 4, RGBA = 6
};

struct IHDR {
    uint32_t width;
    uint32_t height;
    uint8_t  bitDepth;
    enum ColorType colorType;
    uint8_t  compression;
    uint8_t  filter;
    uint8_t  interlace;
};

struct Chunk {
    uint32_t length;
    char     type[4];
    uint8_t  data[length];
    uint32_t crc;
};

uint8_t signature[8];            // 89 50 4E 47 0D 0A 1A 0A
struct {
    uint32_t    length;
    char        type[4];         // "IHDR"
    struct IHDR ihdr;
    uint32_t    crc;
} header;
struct Chunk chunk1;
struct Chunk chunk2;
struct Chunk chunk3;
struct Chunk chunk4;
