// ZIP archive: the central directory, with lazy links to each file's local header.
// Root: "(template declarations)" at offset 0, or "LocalFileHeader" at any "PK\3\4".
// Click a localHeader value in the tree to decode that file; "←" goes back.
// Assumes an archive without a trailing comment (the end record is the last 22 bytes).
#pragma pack(1)

enum Method : uint16_t {
    STORED = 0, SHRUNK = 1, IMPLODED = 6, DEFLATED = 8, DEFLATE64 = 9, BZIP2 = 12, LZMA = 14,
    ZSTD = 93, MP3 = 94, XZ = 95, JPEG = 96, WAVPACK = 97, PPMD = 98, AES = 99
};

typedef struct {
    uint16_t encrypted         : 1;
    uint16_t compressionOption : 2;
    uint16_t dataDescriptor    : 1;
    uint16_t enhancedDeflate   : 1;
    uint16_t patched           : 1;
    uint16_t strongEncryption  : 1;
    uint16_t                   : 4;
    uint16_t utf8              : 1;
    uint16_t                   : 4;
} GeneralFlags;

typedef struct {
    uint16_t seconds2 : 5;       // seconds / 2
    uint16_t minute   : 6;
    uint16_t hour     : 5;
} DosTime;

typedef struct {
    uint16_t day       : 5;
    uint16_t month     : 4;
    uint16_t year1980  : 7;      // years since 1980
} DosDate;

typedef struct {
    uint32_t     signature;      // 0x04034b50
    uint16_t     version;
    GeneralFlags flags;
    enum Method  method;
    DosTime      modTime;
    DosDate      modDate;
    uint32_t     crc32;
    uint32_t     compressedSize;
    uint32_t     uncompressedSize;
    uint16_t     nameLength;
    uint16_t     extraLength;
    char         name[nameLength];
    uint8_t      extra[extraLength];
    uint8_t      data[compressedSize];
} LocalFileHeader;

typedef struct {
    uint32_t     signature;      // 0x02014b50
    uint16_t     versionMadeBy;
    uint16_t     versionNeeded;
    GeneralFlags flags;
    enum Method  method;
    DosTime      modTime;
    DosDate      modDate;
    uint32_t     crc32;
    uint32_t     compressedSize;
    uint32_t     uncompressedSize;
    uint16_t     nameLength;
    uint16_t     extraLength;
    uint16_t     commentLength;
    uint16_t     diskStart;
    uint16_t     internalAttributes;
    uint32_t     externalAttributes;
    // decoded on click; "@ localHeader" keeps offset 0 a link (plain "->" treats 0 as null)
    uint32_t     localHeader -> LocalFileHeader @ localHeader;
    char         name[nameLength];
    uint8_t      extra[extraLength];
    char         comment[commentLength];
} CentralDirEntry;

typedef struct {
    uint32_t signature;          // 0x06054b50
    uint16_t disk;
    uint16_t dirDisk;
    uint16_t entriesOnDisk;
    uint16_t entries;
    uint32_t dirSize;
    uint32_t dirOffset -> CentralDirEntry;
    uint16_t commentLength;
    char     comment[commentLength];
} EndOfCentralDir;

LocalFileHeader first;
EndOfCentralDir end @ FILE_SIZE - 22;
CentralDirEntry dir[end.entries] @ end.dirOffset;
