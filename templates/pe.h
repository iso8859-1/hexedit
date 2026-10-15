// Windows executable (PE32 / PE32+): DOS header, NT headers and section table.
// Root: "(template declarations)", offset 0.
#pragma pack(1)

typedef struct {
    char     e_magic[2];         // "MZ"
    uint16_t e_cblp, e_cp, e_crlc, e_cparhdr, e_minalloc, e_maxalloc;
    uint16_t e_ss, e_sp, e_csum, e_ip, e_cs, e_lfarlc, e_ovno;
    uint16_t e_res[4];
    uint16_t e_oemid, e_oeminfo;
    uint16_t e_res2[10];
    int32_t  e_lfanew;           // offset of the NT headers
} IMAGE_DOS_HEADER;

enum Machine : uint16_t {
    UNKNOWN = 0, I386 = 0x14c, ARM = 0x1c0, ARMNT = 0x1c4, IA64 = 0x200, AMD64 = 0x8664, ARM64 = 0xaa64
};

enum FileCharacteristics : uint16_t {
    RELOCS_STRIPPED = 0x1, EXECUTABLE_IMAGE = 0x2, LINE_NUMS_STRIPPED = 0x4, LOCAL_SYMS_STRIPPED = 0x8,
    LARGE_ADDRESS_AWARE = 0x20, MACHINE_32BIT = 0x100, DEBUG_STRIPPED = 0x200, SYSTEM = 0x1000, DLL = 0x2000
};

typedef struct {
    enum Machine Machine;
    uint16_t NumberOfSections;
    uint32_t TimeDateStamp;
    uint32_t PointerToSymbolTable;
    uint32_t NumberOfSymbols;
    uint16_t SizeOfOptionalHeader;
    enum FileCharacteristics Characteristics;
} IMAGE_FILE_HEADER;

typedef struct {
    uint32_t VirtualAddress;
    uint32_t Size;
} IMAGE_DATA_DIRECTORY;

enum Subsystem : uint16_t {
    NATIVE = 1, WINDOWS_GUI = 2, WINDOWS_CUI = 3, EFI_APPLICATION = 10
};

typedef struct { uint32_t StackReserve, StackCommit, HeapReserve, HeapCommit; } SIZES32;
typedef struct { uint64_t StackReserve, StackCommit, HeapReserve, HeapCommit; } SIZES64;

#define PE32      0x10b
#define PE32PLUS  0x20b

typedef struct {
    uint16_t Magic;
    uint8_t  MajorLinkerVersion, MinorLinkerVersion;
    uint32_t SizeOfCode, SizeOfInitializedData, SizeOfUninitializedData;
    uint32_t AddressOfEntryPoint;
    uint32_t BaseOfCode;
    // arrays of size 0 or 1 act as conditional fields
    uint32_t BaseOfData[Magic == PE32];
    uint32_t ImageBase32[Magic == PE32];
    uint64_t ImageBase64[Magic == PE32PLUS];
    uint32_t SectionAlignment, FileAlignment;
    uint16_t MajorOperatingSystemVersion, MinorOperatingSystemVersion;
    uint16_t MajorImageVersion, MinorImageVersion;
    uint16_t MajorSubsystemVersion, MinorSubsystemVersion;
    uint32_t Win32VersionValue;
    uint32_t SizeOfImage, SizeOfHeaders, CheckSum;
    enum Subsystem Subsystem;
    uint16_t DllCharacteristics;
    SIZES32  Sizes32[Magic == PE32];
    SIZES64  Sizes64[Magic == PE32PLUS];
    uint32_t LoaderFlags;
    uint32_t NumberOfRvaAndSizes;
    IMAGE_DATA_DIRECTORY DataDirectory[NumberOfRvaAndSizes];
} IMAGE_OPTIONAL_HEADER;

typedef struct {
    char     Name[8];
    uint32_t VirtualSize, VirtualAddress;
    uint32_t SizeOfRawData, PointerToRawData;
    uint32_t PointerToRelocations, PointerToLinenumbers;
    uint16_t NumberOfRelocations, NumberOfLinenumbers;
    uint32_t Characteristics;
} IMAGE_SECTION_HEADER;

IMAGE_DOS_HEADER dos;
struct {
    char Signature[4];           // "PE\0\0"
    IMAGE_FILE_HEADER FileHeader;
    IMAGE_OPTIONAL_HEADER OptionalHeader;
} nt @ dos.e_lfanew;
IMAGE_SECTION_HEADER sections[nt.FileHeader.NumberOfSections]
    @ dos.e_lfanew + 24 + nt.FileHeader.SizeOfOptionalHeader;
