// Windows bitmap (.bmp)
// Root: "(template declarations)", offset 0.
#pragma pack(1)
#pragma endian little

typedef struct {
    char     bfType[2];          // "BM"
    uint32_t bfSize;
    uint16_t bfReserved1;
    uint16_t bfReserved2;
    uint32_t bfOffBits;          // start of the pixel data
} BITMAPFILEHEADER;

enum Compression : uint32_t {
    BI_RGB = 0, BI_RLE8 = 1, BI_RLE4 = 2, BI_BITFIELDS = 3, BI_JPEG = 4, BI_PNG = 5
};

typedef struct {
    uint32_t biSize;
    int32_t  biWidth;
    int32_t  biHeight;           // negative: top-down bitmap
    uint16_t biPlanes;
    uint16_t biBitCount;
    enum Compression biCompression;
    uint32_t biSizeImage;
    int32_t  biXPelsPerMeter;
    int32_t  biYPelsPerMeter;
    uint32_t biClrUsed;
    uint32_t biClrImportant;
} BITMAPINFOHEADER;

typedef struct {
    uint8_t blue, green, red, reserved;
} RGBQUAD;

BITMAPFILEHEADER file;
BITMAPINFOHEADER info;
// palette only for <= 8 bpp; biClrUsed == 0 means "all colors"
RGBQUAD palette[info.biBitCount <= 8 ? (info.biClrUsed ? info.biClrUsed : 1 << info.biBitCount) : 0];
uint8_t pixels[file.bfSize - file.bfOffBits] @ file.bfOffBits;
