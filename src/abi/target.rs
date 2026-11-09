//! Target descriptions: the machine facts a layout or calling-convention model consumes.
//!
//! Each [`TargetDesc`] records what the platform ABI documents for one target triple: data
//! model, endianness, the size and in-aggregate alignment of every C scalar class, the C++ ABI
//! family, the tail-padding reuse rule and the bitfield allocation rules. Frontends never pick
//! scalar widths themselves; they name a C scalar class (`long`, `wchar_t`, `long double`) and
//! the target answers.

/// Byte order of multi-byte scalars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endianness {
    Little,
    Big,
}

/// C data model: the widths of `int`, `long` and pointers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataModel {
    /// 32-bit `int`, `long` and pointers.
    Ilp32,
    /// 32-bit `int`, 64-bit `long` and pointers (Unix 64-bit).
    Lp64,
    /// 32-bit `int` and `long`, 64-bit `long long` and pointers (Windows 64-bit).
    Llp64,
}

/// Instruction-set architecture; selects the calling-convention classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86,
    X86_64,
    Arm,
    AArch64,
    RiscV32,
    RiscV64,
    PowerPC64,
    S390x,
    Wasm32,
}

/// Operating system family; selects OS-specific ABI variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Linux,
    Windows,
    Darwin,
    Unknown,
}

/// C++ ABI family used for record layout, vtables and member pointers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CxxAbi {
    /// Generic Itanium C++ ABI (x86, x86-64, RISC-V, PowerPC, SystemZ).
    Itanium,
    /// Itanium with the ARM member-function-pointer encoding (virtual bit in `adj`):
    /// 32-bit ARM, AArch64 and WebAssembly.
    ItaniumArm,
    /// Microsoft Visual C++ ABI.
    Microsoft,
}

/// When a derived class may allocate members in the tail padding of a base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TailPadding {
    /// Reuse tail padding unless the base is POD in the C++03 sense (generic Itanium).
    UnlessPod03,
    /// Reuse tail padding unless the base is trivial and standard-layout (Apple ARM64,
    /// WebAssembly).
    UnlessPod11,
}

/// Bitfield allocation discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitfieldLayout {
    /// System V / GCC: a bitfield shares any storage unit of its declared type it fits in.
    Itanium,
    /// MSVC: consecutive bitfields share a unit only when their declared types have equal size.
    Microsoft,
}

/// Representation of `long double`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LongDoubleFormat {
    IeeeDouble,
    X87Extended,
    IeeeQuad,
    IbmDoubleDouble,
}

/// Size and in-aggregate (ABI) alignment of a scalar, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScalarLayout {
    pub size: u64,
    pub align: u64,
}

impl ScalarLayout {
    pub const fn new(size: u64, align: u64) -> Self {
        Self { size, align }
    }
}

/// C scalar classes whose representation is defined by the target, not by the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CScalar {
    Bool,
    Char,
    SChar,
    UChar,
    WChar,
    Char8,
    Char16,
    Char32,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
    Int128,
    UInt128,
    Half,
    Float,
    Double,
    LongDouble,
    Float128,
    Pointer,
    SizeT,
    PtrDiffT,
    NullPtrT,
}

impl CScalar {
    pub fn is_floating(self) -> bool {
        matches!(
            self,
            CScalar::Half | CScalar::Float | CScalar::Double | CScalar::LongDouble | CScalar::Float128
        )
    }

    /// Signedness of integer classes on `target`; `None` for floating and pointer classes.
    pub fn is_signed(self, target: &TargetDesc) -> Option<bool> {
        Some(match self {
            CScalar::Char => target.char_signed,
            CScalar::WChar => target.wchar_signed,
            CScalar::SChar
            | CScalar::Short
            | CScalar::Int
            | CScalar::Long
            | CScalar::LongLong
            | CScalar::Int128
            | CScalar::PtrDiffT => true,
            CScalar::Bool
            | CScalar::UChar
            | CScalar::Char8
            | CScalar::Char16
            | CScalar::Char32
            | CScalar::UShort
            | CScalar::UInt
            | CScalar::ULong
            | CScalar::ULongLong
            | CScalar::UInt128
            | CScalar::SizeT => false,
            _ => return None,
        })
    }
}

/// Everything the ABI models need to know about one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDesc {
    pub triple: String,
    pub arch: Arch,
    pub os: Os,
    pub endianness: Endianness,
    pub data_model: DataModel,
    pub pointer: ScalarLayout,
    pub short: ScalarLayout,
    pub int: ScalarLayout,
    pub long: ScalarLayout,
    pub long_long: ScalarLayout,
    /// `__int128`, when the target provides it.
    pub int128: Option<ScalarLayout>,
    pub half: ScalarLayout,
    pub float: ScalarLayout,
    pub double: ScalarLayout,
    pub long_double: ScalarLayout,
    pub long_double_format: LongDoubleFormat,
    /// `__float128` / `_Float128`, when the target provides it.
    pub float128: Option<ScalarLayout>,
    pub wchar: ScalarLayout,
    pub wchar_signed: bool,
    pub char_signed: bool,
    /// `alignof(max_align_t)`.
    pub max_align: u64,
    pub cxx_abi: CxxAbi,
    pub tail_padding: TailPadding,
    pub bitfields: BitfieldLayout,
    /// A zero-width bitfield raises the alignment of the enclosing record (AAPCS).
    pub zero_width_bitfield_aligns_record: bool,
    /// An unnamed non-zero-width bitfield contributes its type's alignment to the record (AAPCS).
    pub unnamed_bitfield_aligns_record: bool,
}

impl TargetDesc {
    pub fn pointer_size(&self) -> u64 {
        self.pointer.size
    }

    pub fn is_64bit(&self) -> bool {
        self.pointer.size == 8
    }

    /// Size and alignment of a C scalar class; `None` when the target lacks the type.
    pub fn c_scalar(&self, scalar: CScalar) -> Option<ScalarLayout> {
        let byte = ScalarLayout::new(1, 1);
        Some(match scalar {
            CScalar::Bool | CScalar::Char | CScalar::SChar | CScalar::UChar | CScalar::Char8 => byte,
            CScalar::WChar => self.wchar,
            CScalar::Char16 => self.short,
            CScalar::Char32 => self.int,
            CScalar::Short | CScalar::UShort => self.short,
            CScalar::Int | CScalar::UInt => self.int,
            CScalar::Long | CScalar::ULong => self.long,
            CScalar::LongLong | CScalar::ULongLong => self.long_long,
            CScalar::Int128 | CScalar::UInt128 => return self.int128,
            CScalar::Half => self.half,
            CScalar::Float => self.float,
            CScalar::Double => self.double,
            CScalar::LongDouble => self.long_double,
            CScalar::Float128 => return self.float128,
            CScalar::Pointer | CScalar::SizeT | CScalar::PtrDiffT | CScalar::NullPtrT => self.pointer,
        })
    }

    /// Layout of a two's-complement integer of `bits` width, taken from the C integer class
    /// of that width. Widths without a C class (e.g. `_BitInt(N)`) yield `None`.
    pub fn int_of_bits(&self, bits: u16) -> Option<ScalarLayout> {
        let candidates = [
            ScalarLayout::new(1, 1),
            self.short,
            self.int,
            self.long,
            self.long_long,
        ];
        let size = u64::from(bits) / 8;
        if u64::from(bits) % 8 != 0 {
            return None;
        }
        if let Some(found) = candidates.iter().find(|c| c.size == size) {
            return Some(*found);
        }
        self.int128.filter(|l| l.size == size)
    }

    /// Layout of a binary floating type of `bits` width.
    pub fn float_of_bits(&self, bits: u16) -> Option<ScalarLayout> {
        match bits {
            16 => Some(self.half),
            32 => Some(self.float),
            64 => Some(self.double),
            80 if self.long_double_format == LongDoubleFormat::X87Extended => Some(self.long_double),
            128 => self.float128.or(match self.long_double_format {
                LongDoubleFormat::IeeeQuad | LongDoubleFormat::IbmDoubleDouble => Some(self.long_double),
                _ => None,
            }),
            _ => None,
        }
    }

    /// Integer C classes in ascending width, used to pick the storage type of wide bitfields.
    pub fn integer_ladder(&self) -> Vec<ScalarLayout> {
        let mut ladder = vec![ScalarLayout::new(1, 1), self.short, self.int, self.long, self.long_long];
        if let Some(i128) = self.int128 {
            ladder.push(i128);
        }
        ladder.sort_by_key(|l| l.size);
        ladder.dedup_by_key(|l| l.size);
        ladder
    }

    /// Looks up a target by triple. Vendor fields are ignored; the environment suffix
    /// distinguishes MSVC from GNU on Windows.
    pub fn from_triple(triple: &str) -> Option<TargetDesc> {
        let parts: Vec<&str> = triple.split('-').collect();
        let arch = *parts.first()?;
        let rest = parts[1..].join("-");
        let windows = rest.contains("windows");
        let msvc = windows && !rest.contains("gnu");
        let darwin = rest.contains("darwin") || rest.contains("apple") || rest.contains("macos");
        let target = match arch {
            "x86_64" | "amd64" if msvc => Self::x86_64_windows_msvc(),
            "x86_64" | "amd64" if windows => Self::x86_64_windows_gnu(),
            "x86_64" | "amd64" if darwin => Self::x86_64_apple_darwin(),
            "x86_64" | "amd64" => Self::x86_64_linux_gnu(),
            "i386" | "i486" | "i586" | "i686" if msvc => Self::i686_windows_msvc(),
            "i386" | "i486" | "i586" | "i686" => Self::i686_linux_gnu(),
            "aarch64" | "arm64" if msvc => Self::aarch64_windows_msvc(),
            "aarch64" | "arm64" if darwin => Self::aarch64_apple_darwin(),
            "aarch64" | "arm64" => Self::aarch64_linux_gnu(),
            a if a.starts_with("armv7") || a == "arm" => Self::armv7_linux_gnueabihf(),
            a if a.starts_with("riscv64") => Self::riscv64_linux_gnu(),
            a if a.starts_with("riscv32") => Self::riscv32_unknown_elf(),
            "powerpc64le" | "ppc64le" => Self::powerpc64le_linux_gnu(),
            "s390x" => Self::s390x_linux_gnu(),
            "wasm32" => Self::wasm32_unknown(),
            _ => return None,
        };
        Some(target)
    }

    /// All predefined targets.
    pub fn known() -> Vec<TargetDesc> {
        vec![
            Self::x86_64_linux_gnu(),
            Self::x86_64_apple_darwin(),
            Self::x86_64_windows_msvc(),
            Self::x86_64_windows_gnu(),
            Self::i686_linux_gnu(),
            Self::i686_windows_msvc(),
            Self::aarch64_linux_gnu(),
            Self::aarch64_apple_darwin(),
            Self::aarch64_windows_msvc(),
            Self::armv7_linux_gnueabihf(),
            Self::riscv64_linux_gnu(),
            Self::riscv32_unknown_elf(),
            Self::powerpc64le_linux_gnu(),
            Self::s390x_linux_gnu(),
            Self::wasm32_unknown(),
        ]
    }

    /// Common LP64 Unix skeleton, specialised by the constructors below.
    fn lp64(triple: &str, arch: Arch, os: Os) -> TargetDesc {
        TargetDesc {
            triple: triple.to_string(),
            arch,
            os,
            endianness: Endianness::Little,
            data_model: DataModel::Lp64,
            pointer: ScalarLayout::new(8, 8),
            short: ScalarLayout::new(2, 2),
            int: ScalarLayout::new(4, 4),
            long: ScalarLayout::new(8, 8),
            long_long: ScalarLayout::new(8, 8),
            int128: Some(ScalarLayout::new(16, 16)),
            half: ScalarLayout::new(2, 2),
            float: ScalarLayout::new(4, 4),
            double: ScalarLayout::new(8, 8),
            long_double: ScalarLayout::new(16, 16),
            long_double_format: LongDoubleFormat::IeeeQuad,
            float128: Some(ScalarLayout::new(16, 16)),
            wchar: ScalarLayout::new(4, 4),
            wchar_signed: true,
            char_signed: true,
            max_align: 16,
            cxx_abi: CxxAbi::Itanium,
            tail_padding: TailPadding::UnlessPod03,
            bitfields: BitfieldLayout::Itanium,
            zero_width_bitfield_aligns_record: false,
            unnamed_bitfield_aligns_record: false,
        }
    }

    /// Common ILP32 skeleton.
    fn ilp32(triple: &str, arch: Arch, os: Os) -> TargetDesc {
        TargetDesc {
            data_model: DataModel::Ilp32,
            pointer: ScalarLayout::new(4, 4),
            long: ScalarLayout::new(4, 4),
            int128: None,
            ..Self::lp64(triple, arch, os)
        }
    }

    /// Common Windows LLP64 skeleton (Microsoft C++ ABI).
    fn llp64_msvc(triple: &str, arch: Arch) -> TargetDesc {
        TargetDesc {
            data_model: DataModel::Llp64,
            long: ScalarLayout::new(4, 4),
            int128: None,
            long_double: ScalarLayout::new(8, 8),
            long_double_format: LongDoubleFormat::IeeeDouble,
            float128: None,
            wchar: ScalarLayout::new(2, 2),
            wchar_signed: false,
            max_align: 8,
            cxx_abi: CxxAbi::Microsoft,
            bitfields: BitfieldLayout::Microsoft,
            ..Self::lp64(triple, arch, Os::Windows)
        }
    }

    pub fn x86_64_linux_gnu() -> TargetDesc {
        TargetDesc {
            long_double_format: LongDoubleFormat::X87Extended,
            ..Self::lp64("x86_64-unknown-linux-gnu", Arch::X86_64, Os::Linux)
        }
    }

    pub fn x86_64_apple_darwin() -> TargetDesc {
        TargetDesc {
            long_double_format: LongDoubleFormat::X87Extended,
            ..Self::lp64("x86_64-apple-darwin", Arch::X86_64, Os::Darwin)
        }
    }

    pub fn x86_64_windows_msvc() -> TargetDesc {
        Self::llp64_msvc("x86_64-pc-windows-msvc", Arch::X86_64)
    }

    /// MinGW-w64: LLP64 with the Itanium C++ ABI, x87 `long double` and MS bitfields.
    pub fn x86_64_windows_gnu() -> TargetDesc {
        TargetDesc {
            data_model: DataModel::Llp64,
            long: ScalarLayout::new(4, 4),
            long_double_format: LongDoubleFormat::X87Extended,
            wchar: ScalarLayout::new(2, 2),
            wchar_signed: false,
            bitfields: BitfieldLayout::Microsoft,
            ..Self::lp64("x86_64-w64-windows-gnu", Arch::X86_64, Os::Windows)
        }
    }

    /// i386 System V: 8-byte scalars are 4-aligned inside aggregates; 12-byte x87 `long double`.
    pub fn i686_linux_gnu() -> TargetDesc {
        TargetDesc {
            long_long: ScalarLayout::new(8, 4),
            double: ScalarLayout::new(8, 4),
            long_double: ScalarLayout::new(12, 4),
            long_double_format: LongDoubleFormat::X87Extended,
            float128: Some(ScalarLayout::new(16, 16)),
            ..Self::ilp32("i686-unknown-linux-gnu", Arch::X86, Os::Linux)
        }
    }

    pub fn i686_windows_msvc() -> TargetDesc {
        TargetDesc {
            data_model: DataModel::Ilp32,
            pointer: ScalarLayout::new(4, 4),
            ..Self::llp64_msvc("i686-pc-windows-msvc", Arch::X86)
        }
    }

    pub fn aarch64_linux_gnu() -> TargetDesc {
        TargetDesc {
            wchar_signed: false,
            char_signed: false,
            cxx_abi: CxxAbi::ItaniumArm,
            zero_width_bitfield_aligns_record: true,
            unnamed_bitfield_aligns_record: true,
            ..Self::lp64("aarch64-unknown-linux-gnu", Arch::AArch64, Os::Linux)
        }
    }

    pub fn aarch64_apple_darwin() -> TargetDesc {
        TargetDesc {
            long_double: ScalarLayout::new(8, 8),
            long_double_format: LongDoubleFormat::IeeeDouble,
            float128: None,
            max_align: 8,
            cxx_abi: CxxAbi::ItaniumArm,
            tail_padding: TailPadding::UnlessPod11,
            unnamed_bitfield_aligns_record: true,
            ..Self::lp64("aarch64-apple-darwin", Arch::AArch64, Os::Darwin)
        }
    }

    pub fn aarch64_windows_msvc() -> TargetDesc {
        Self::llp64_msvc("aarch64-pc-windows-msvc", Arch::AArch64)
    }

    /// AAPCS (hard float): 8-byte scalars are 8-aligned; `long double` is `double`.
    pub fn armv7_linux_gnueabihf() -> TargetDesc {
        TargetDesc {
            long_double: ScalarLayout::new(8, 8),
            long_double_format: LongDoubleFormat::IeeeDouble,
            float128: None,
            wchar_signed: false,
            char_signed: false,
            max_align: 8,
            cxx_abi: CxxAbi::ItaniumArm,
            zero_width_bitfield_aligns_record: true,
            unnamed_bitfield_aligns_record: true,
            ..Self::ilp32("armv7-unknown-linux-gnueabihf", Arch::Arm, Os::Linux)
        }
    }

    pub fn riscv64_linux_gnu() -> TargetDesc {
        TargetDesc {
            char_signed: false,
            ..Self::lp64("riscv64gc-unknown-linux-gnu", Arch::RiscV64, Os::Linux)
        }
    }

    pub fn riscv32_unknown_elf() -> TargetDesc {
        TargetDesc {
            char_signed: false,
            float128: None,
            ..Self::ilp32("riscv32imac-unknown-none-elf", Arch::RiscV32, Os::Unknown)
        }
    }

    pub fn powerpc64le_linux_gnu() -> TargetDesc {
        TargetDesc {
            long_double_format: LongDoubleFormat::IbmDoubleDouble,
            char_signed: false,
            ..Self::lp64("powerpc64le-unknown-linux-gnu", Arch::PowerPC64, Os::Linux)
        }
    }

    /// z/Architecture: big-endian; 16-byte `long double` and `__int128` are 8-aligned.
    pub fn s390x_linux_gnu() -> TargetDesc {
        TargetDesc {
            endianness: Endianness::Big,
            int128: Some(ScalarLayout::new(16, 8)),
            long_double: ScalarLayout::new(16, 8),
            float128: None,
            char_signed: false,
            max_align: 8,
            ..Self::lp64("s390x-unknown-linux-gnu", Arch::S390x, Os::Linux)
        }
    }

    pub fn wasm32_unknown() -> TargetDesc {
        TargetDesc {
            int128: Some(ScalarLayout::new(16, 16)),
            float128: None,
            cxx_abi: CxxAbi::ItaniumArm,
            tail_padding: TailPadding::UnlessPod11,
            ..Self::ilp32("wasm32-unknown-unknown", Arch::Wasm32, Os::Unknown)
        }
    }
}
