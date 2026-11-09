//! Pre-built C Standard Library Model (ISO C17 headers: types, tags, functions, objects and
//! macros). Entries are written as C declarations and parsed with the C-family grammar, so
//! every function carries its full prototype. Loaded at Phase 3 initialization with
//! symbol_ids >= 0xC1000000.

use crate::core::types::symbol::{SymbolKind, SymbolRecord, SymbolVisibility};
use crate::lang::c::semantics::{
    collect_type_names, lex_c_text, DeclContext, DeclError, DeclParser, MacroDefinition,
    NameSetOracle, ParsedDeclaration, SC_TYPEDEF,
};
use crate::lang::c::Dialect;
use crate::symbol::qual_name_table::QualifiedNameTable;
use std::collections::HashMap;

pub const C_STUB_BASE_ID: u32 = 0xC100_0000;

/// One standard header: its declarations and its macros (`"NAME"` or `"NAME(a, b)"`
/// with the replacement list).
#[derive(Debug, Clone, Copy)]
pub struct CHeader {
    pub name: &'static str,
    /// The C++ `<cxxx>` header that declares the same names in namespace `std`.
    pub cpp_name: Option<&'static str>,
    pub decls: &'static [&'static str],
    pub macros: &'static [(&'static str, &'static str)],
}

pub static C_HEADERS: &[CHeader] = &[
    CHeader {
        name: "assert.h",
        cpp_name: Some("cassert"),
        decls: &[],
        macros: &[("assert(expression)", "((void)0)"), ("static_assert", "_Static_assert")],
    },
    CHeader {
        name: "ctype.h",
        cpp_name: Some("cctype"),
        decls: &[
            "int isalnum(int c);",
            "int isalpha(int c);",
            "int isblank(int c);",
            "int iscntrl(int c);",
            "int isdigit(int c);",
            "int isgraph(int c);",
            "int islower(int c);",
            "int isprint(int c);",
            "int ispunct(int c);",
            "int isspace(int c);",
            "int isupper(int c);",
            "int isxdigit(int c);",
            "int tolower(int c);",
            "int toupper(int c);",
        ],
        macros: &[],
    },
    CHeader {
        name: "errno.h",
        cpp_name: Some("cerrno"),
        decls: &[],
        macros: &[
            ("errno", "(*__errno_location())"),
            ("EDOM", "33"),
            ("EILSEQ", "84"),
            ("ERANGE", "34"),
        ],
    },
    CHeader {
        name: "fenv.h",
        cpp_name: Some("cfenv"),
        decls: &[
            "typedef struct __fenv_t fenv_t;",
            "typedef unsigned short fexcept_t;",
            "int feclearexcept(int excepts);",
            "int fegetexceptflag(fexcept_t *flagp, int excepts);",
            "int feraiseexcept(int excepts);",
            "int fesetexceptflag(const fexcept_t *flagp, int excepts);",
            "int fetestexcept(int excepts);",
            "int fegetround(void);",
            "int fesetround(int round);",
            "int fegetenv(fenv_t *envp);",
            "int fesetenv(const fenv_t *envp);",
        ],
        macros: &[
            ("FE_DIVBYZERO", "4"),
            ("FE_INEXACT", "32"),
            ("FE_INVALID", "1"),
            ("FE_OVERFLOW", "8"),
            ("FE_UNDERFLOW", "16"),
            ("FE_ALL_EXCEPT", "61"),
            ("FE_DOWNWARD", "0x400"),
            ("FE_TONEAREST", "0"),
            ("FE_TOWARDZERO", "0xc00"),
            ("FE_UPWARD", "0x800"),
        ],
    },
    CHeader {
        name: "float.h",
        cpp_name: Some("cfloat"),
        decls: &[],
        macros: &[
            ("FLT_RADIX", "2"),
            ("FLT_DIG", "6"),
            ("DBL_DIG", "15"),
            ("LDBL_DIG", "18"),
            ("FLT_EPSILON", "1.19209290e-7F"),
            ("DBL_EPSILON", "2.2204460492503131e-16"),
            ("FLT_MIN", "1.17549435e-38F"),
            ("DBL_MIN", "2.2250738585072014e-308"),
            ("FLT_MAX", "3.40282347e+38F"),
            ("DBL_MAX", "1.7976931348623157e+308"),
            ("LDBL_MAX", "1.18973149535723176502e+4932L"),
            ("FLT_MANT_DIG", "24"),
            ("DBL_MANT_DIG", "53"),
        ],
    },
    CHeader {
        name: "inttypes.h",
        cpp_name: Some("cinttypes"),
        decls: &[
            "typedef struct { intmax_t quot; intmax_t rem; } imaxdiv_t;",
            "intmax_t imaxabs(intmax_t j);",
            "imaxdiv_t imaxdiv(intmax_t numer, intmax_t denom);",
            "intmax_t strtoimax(const char *restrict nptr, char **restrict endptr, int base);",
            "uintmax_t strtoumax(const char *restrict nptr, char **restrict endptr, int base);",
        ],
        macros: &[
            ("PRId8", "\"d\""),
            ("PRId16", "\"d\""),
            ("PRId32", "\"d\""),
            ("PRId64", "\"ld\""),
            ("PRIu32", "\"u\""),
            ("PRIu64", "\"lu\""),
            ("PRIx32", "\"x\""),
            ("PRIx64", "\"lx\""),
            ("PRIdMAX", "\"ld\""),
            ("PRIuPTR", "\"lu\""),
            ("SCNd32", "\"d\""),
            ("SCNd64", "\"ld\""),
        ],
    },
    CHeader {
        name: "iso646.h",
        cpp_name: None,
        decls: &[],
        macros: &[
            ("and", "&&"),
            ("and_eq", "&="),
            ("bitand", "&"),
            ("bitor", "|"),
            ("compl", "~"),
            ("not", "!"),
            ("not_eq", "!="),
            ("or", "||"),
            ("or_eq", "|="),
            ("xor", "^"),
            ("xor_eq", "^="),
        ],
    },
    CHeader {
        name: "limits.h",
        cpp_name: Some("climits"),
        decls: &[],
        macros: &[
            ("CHAR_BIT", "8"),
            ("SCHAR_MIN", "(-128)"),
            ("SCHAR_MAX", "127"),
            ("UCHAR_MAX", "255"),
            ("CHAR_MIN", "SCHAR_MIN"),
            ("CHAR_MAX", "SCHAR_MAX"),
            ("MB_LEN_MAX", "16"),
            ("SHRT_MIN", "(-32768)"),
            ("SHRT_MAX", "32767"),
            ("USHRT_MAX", "65535"),
            ("INT_MIN", "(-INT_MAX - 1)"),
            ("INT_MAX", "2147483647"),
            ("UINT_MAX", "4294967295U"),
            ("LONG_MIN", "(-LONG_MAX - 1L)"),
            ("LONG_MAX", "9223372036854775807L"),
            ("ULONG_MAX", "18446744073709551615UL"),
            ("LLONG_MIN", "(-LLONG_MAX - 1LL)"),
            ("LLONG_MAX", "9223372036854775807LL"),
            ("ULLONG_MAX", "18446744073709551615ULL"),
        ],
    },
    CHeader {
        name: "locale.h",
        cpp_name: Some("clocale"),
        decls: &[
            "struct lconv { char *decimal_point; char *thousands_sep; char *grouping; char *int_curr_symbol; char *currency_symbol; char *mon_decimal_point; char *mon_thousands_sep; char *mon_grouping; char *positive_sign; char *negative_sign; char int_frac_digits; char frac_digits; char p_cs_precedes; char p_sep_by_space; char n_cs_precedes; char n_sep_by_space; char p_sign_posn; char n_sign_posn; };",
            "char *setlocale(int category, const char *locale);",
            "struct lconv *localeconv(void);",
        ],
        macros: &[
            ("LC_ALL", "6"),
            ("LC_COLLATE", "3"),
            ("LC_CTYPE", "0"),
            ("LC_MONETARY", "4"),
            ("LC_NUMERIC", "1"),
            ("LC_TIME", "2"),
            ("NULL", "((void*)0)"),
        ],
    },
    CHeader {
        name: "math.h",
        cpp_name: Some("cmath"),
        decls: &[
            "typedef float float_t;",
            "typedef double double_t;",
            "double acos(double x);",
            "double asin(double x);",
            "double atan(double x);",
            "double atan2(double y, double x);",
            "double cos(double x);",
            "double sin(double x);",
            "double tan(double x);",
            "double acosh(double x);",
            "double asinh(double x);",
            "double atanh(double x);",
            "double cosh(double x);",
            "double sinh(double x);",
            "double tanh(double x);",
            "double exp(double x);",
            "double exp2(double x);",
            "double expm1(double x);",
            "double frexp(double value, int *exp);",
            "double ldexp(double x, int exp);",
            "double log(double x);",
            "double log10(double x);",
            "double log1p(double x);",
            "double log2(double x);",
            "double modf(double value, double *iptr);",
            "double pow(double x, double y);",
            "double sqrt(double x);",
            "double cbrt(double x);",
            "double hypot(double x, double y);",
            "double fabs(double x);",
            "double ceil(double x);",
            "double floor(double x);",
            "double round(double x);",
            "long lround(double x);",
            "double trunc(double x);",
            "double fmod(double x, double y);",
            "double remainder(double x, double y);",
            "double fmax(double x, double y);",
            "double fmin(double x, double y);",
            "double fdim(double x, double y);",
            "double fma(double x, double y, double z);",
            "double nan(const char *tagp);",
            "float sqrtf(float x);",
            "float sinf(float x);",
            "float cosf(float x);",
            "float tanf(float x);",
            "float powf(float x, float y);",
            "float fabsf(float x);",
            "float floorf(float x);",
            "float ceilf(float x);",
            "float roundf(float x);",
            "float expf(float x);",
            "float logf(float x);",
            "long double sqrtl(long double x);",
            "long double fabsl(long double x);",
            "long double powl(long double x, long double y);",
        ],
        macros: &[
            ("HUGE_VAL", "(__builtin_huge_val())"),
            ("HUGE_VALF", "(__builtin_huge_valf())"),
            ("INFINITY", "(__builtin_inff())"),
            ("NAN", "(__builtin_nanf(\"\"))"),
            ("FP_INFINITE", "1"),
            ("FP_NAN", "0"),
            ("FP_NORMAL", "4"),
            ("FP_SUBNORMAL", "3"),
            ("FP_ZERO", "2"),
            ("fpclassify(x)", "__builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, x)"),
            ("isfinite(x)", "__builtin_isfinite(x)"),
            ("isinf(x)", "__builtin_isinf_sign(x)"),
            ("isnan(x)", "__builtin_isnan(x)"),
            ("isnormal(x)", "__builtin_isnormal(x)"),
            ("signbit(x)", "__builtin_signbit(x)"),
        ],
    },
    CHeader {
        name: "setjmp.h",
        cpp_name: Some("csetjmp"),
        decls: &[
            "typedef struct __jmp_buf_tag jmp_buf[1];",
            "int setjmp(jmp_buf env);",
            "_Noreturn void longjmp(jmp_buf env, int val);",
        ],
        macros: &[],
    },
    CHeader {
        name: "signal.h",
        cpp_name: Some("csignal"),
        decls: &[
            "typedef int sig_atomic_t;",
            "void (*signal(int sig, void (*func)(int)))(int);",
            "int raise(int sig);",
        ],
        macros: &[
            ("SIGABRT", "6"),
            ("SIGFPE", "8"),
            ("SIGILL", "4"),
            ("SIGINT", "2"),
            ("SIGSEGV", "11"),
            ("SIGTERM", "15"),
            ("SIG_DFL", "((void (*)(int))0)"),
            ("SIG_ERR", "((void (*)(int))-1)"),
            ("SIG_IGN", "((void (*)(int))1)"),
        ],
    },
    CHeader {
        name: "stdarg.h",
        cpp_name: Some("cstdarg"),
        decls: &["typedef __builtin_va_list va_list;"],
        macros: &[
            ("va_start(ap, parmN)", "__builtin_va_start(ap, parmN)"),
            ("va_arg(ap, type)", "__builtin_va_arg(ap, type)"),
            ("va_copy(dest, src)", "__builtin_va_copy(dest, src)"),
            ("va_end(ap)", "__builtin_va_end(ap)"),
        ],
    },
    CHeader {
        name: "stdatomic.h",
        cpp_name: None,
        decls: &[
            "typedef enum memory_order { memory_order_relaxed, memory_order_consume, memory_order_acquire, memory_order_release, memory_order_acq_rel, memory_order_seq_cst } memory_order;",
            "typedef struct atomic_flag { _Bool _Value; } atomic_flag;",
            "typedef _Atomic _Bool atomic_bool;",
            "typedef _Atomic char atomic_char;",
            "typedef _Atomic int atomic_int;",
            "typedef _Atomic unsigned int atomic_uint;",
            "typedef _Atomic long atomic_long;",
            "typedef _Atomic unsigned long atomic_ulong;",
            "typedef _Atomic long long atomic_llong;",
            "typedef _Atomic unsigned long atomic_size_t;",
            "void atomic_thread_fence(memory_order order);",
            "void atomic_signal_fence(memory_order order);",
            "_Bool atomic_flag_test_and_set(volatile atomic_flag *object);",
            "void atomic_flag_clear(volatile atomic_flag *object);",
        ],
        macros: &[
            ("ATOMIC_FLAG_INIT", "{ 0 }"),
            ("atomic_init(obj, value)", "__c11_atomic_init(obj, value)"),
            ("atomic_load(object)", "__c11_atomic_load(object, memory_order_seq_cst)"),
            ("atomic_store(object, desired)", "__c11_atomic_store(object, desired, memory_order_seq_cst)"),
            ("atomic_exchange(object, desired)", "__c11_atomic_exchange(object, desired, memory_order_seq_cst)"),
            ("atomic_fetch_add(object, operand)", "__c11_atomic_fetch_add(object, operand, memory_order_seq_cst)"),
            ("atomic_fetch_sub(object, operand)", "__c11_atomic_fetch_sub(object, operand, memory_order_seq_cst)"),
            ("atomic_compare_exchange_strong(object, expected, desired)", "__c11_atomic_compare_exchange_strong(object, expected, desired, memory_order_seq_cst, memory_order_seq_cst)"),
        ],
    },
    CHeader {
        name: "stdbool.h",
        cpp_name: None,
        decls: &[],
        macros: &[
            ("bool", "_Bool"),
            ("true", "1"),
            ("false", "0"),
            ("__bool_true_false_are_defined", "1"),
        ],
    },
    CHeader {
        name: "stddef.h",
        cpp_name: Some("cstddef"),
        decls: &[
            "typedef long ptrdiff_t;",
            "typedef unsigned long size_t;",
            "typedef int wchar_t;",
            "typedef struct { long long __max_align_ll; long double __max_align_ld; } max_align_t;",
        ],
        macros: &[
            ("NULL", "((void*)0)"),
            ("offsetof(type, member)", "__builtin_offsetof(type, member)"),
        ],
    },
    CHeader {
        name: "stdint.h",
        cpp_name: Some("cstdint"),
        decls: &[
            "typedef signed char int8_t;",
            "typedef short int16_t;",
            "typedef int int32_t;",
            "typedef long int64_t;",
            "typedef unsigned char uint8_t;",
            "typedef unsigned short uint16_t;",
            "typedef unsigned int uint32_t;",
            "typedef unsigned long uint64_t;",
            "typedef signed char int_least8_t;",
            "typedef short int_least16_t;",
            "typedef int int_least32_t;",
            "typedef long int_least64_t;",
            "typedef unsigned char uint_least8_t;",
            "typedef unsigned short uint_least16_t;",
            "typedef unsigned int uint_least32_t;",
            "typedef unsigned long uint_least64_t;",
            "typedef signed char int_fast8_t;",
            "typedef long int_fast16_t;",
            "typedef long int_fast32_t;",
            "typedef long int_fast64_t;",
            "typedef unsigned char uint_fast8_t;",
            "typedef unsigned long uint_fast16_t;",
            "typedef unsigned long uint_fast32_t;",
            "typedef unsigned long uint_fast64_t;",
            "typedef long intptr_t;",
            "typedef unsigned long uintptr_t;",
            "typedef long intmax_t;",
            "typedef unsigned long uintmax_t;",
        ],
        macros: &[
            ("INT8_MIN", "(-128)"),
            ("INT8_MAX", "127"),
            ("UINT8_MAX", "255"),
            ("INT16_MIN", "(-32768)"),
            ("INT16_MAX", "32767"),
            ("UINT16_MAX", "65535"),
            ("INT32_MIN", "(-2147483647-1)"),
            ("INT32_MAX", "2147483647"),
            ("UINT32_MAX", "4294967295U"),
            ("INT64_MIN", "(-9223372036854775807L-1)"),
            ("INT64_MAX", "9223372036854775807L"),
            ("UINT64_MAX", "18446744073709551615UL"),
            ("INTPTR_MAX", "9223372036854775807L"),
            ("UINTPTR_MAX", "18446744073709551615UL"),
            ("INTMAX_MAX", "9223372036854775807L"),
            ("UINTMAX_MAX", "18446744073709551615UL"),
            ("SIZE_MAX", "18446744073709551615UL"),
            ("PTRDIFF_MAX", "9223372036854775807L"),
            ("INT8_C(c)", "c"),
            ("INT16_C(c)", "c"),
            ("INT32_C(c)", "c"),
            ("INT64_C(c)", "c ## L"),
            ("UINT8_C(c)", "c"),
            ("UINT16_C(c)", "c"),
            ("UINT32_C(c)", "c ## U"),
            ("UINT64_C(c)", "c ## UL"),
            ("INTMAX_C(c)", "c ## L"),
            ("UINTMAX_C(c)", "c ## UL"),
        ],
    },
    CHeader {
        name: "stdio.h",
        cpp_name: Some("cstdio"),
        decls: &[
            "typedef struct _IO_FILE FILE;",
            "typedef struct __fpos_t fpos_t;",
            "extern FILE *stdin;",
            "extern FILE *stdout;",
            "extern FILE *stderr;",
            "int remove(const char *filename);",
            "int rename(const char *old, const char *new_name);",
            "FILE *tmpfile(void);",
            "char *tmpnam(char *s);",
            "int fclose(FILE *stream);",
            "int fflush(FILE *stream);",
            "FILE *fopen(const char *restrict filename, const char *restrict mode);",
            "FILE *freopen(const char *restrict filename, const char *restrict mode, FILE *restrict stream);",
            "void setbuf(FILE *restrict stream, char *restrict buf);",
            "int setvbuf(FILE *restrict stream, char *restrict buf, int mode, size_t size);",
            "int fprintf(FILE *restrict stream, const char *restrict format, ...);",
            "int fscanf(FILE *restrict stream, const char *restrict format, ...);",
            "int printf(const char *restrict format, ...);",
            "int scanf(const char *restrict format, ...);",
            "int snprintf(char *restrict s, size_t n, const char *restrict format, ...);",
            "int sprintf(char *restrict s, const char *restrict format, ...);",
            "int sscanf(const char *restrict s, const char *restrict format, ...);",
            "int vfprintf(FILE *restrict stream, const char *restrict format, va_list arg);",
            "int vprintf(const char *restrict format, va_list arg);",
            "int vsnprintf(char *restrict s, size_t n, const char *restrict format, va_list arg);",
            "int vsprintf(char *restrict s, const char *restrict format, va_list arg);",
            "int fgetc(FILE *stream);",
            "char *fgets(char *restrict s, int n, FILE *restrict stream);",
            "int fputc(int c, FILE *stream);",
            "int fputs(const char *restrict s, FILE *restrict stream);",
            "int getc(FILE *stream);",
            "int getchar(void);",
            "int putc(int c, FILE *stream);",
            "int putchar(int c);",
            "int puts(const char *s);",
            "int ungetc(int c, FILE *stream);",
            "size_t fread(void *restrict ptr, size_t size, size_t nmemb, FILE *restrict stream);",
            "size_t fwrite(const void *restrict ptr, size_t size, size_t nmemb, FILE *restrict stream);",
            "int fgetpos(FILE *restrict stream, fpos_t *restrict pos);",
            "int fseek(FILE *stream, long offset, int whence);",
            "int fsetpos(FILE *stream, const fpos_t *pos);",
            "long ftell(FILE *stream);",
            "void rewind(FILE *stream);",
            "void clearerr(FILE *stream);",
            "int feof(FILE *stream);",
            "int ferror(FILE *stream);",
            "void perror(const char *s);",
        ],
        macros: &[
            ("EOF", "(-1)"),
            ("BUFSIZ", "8192"),
            ("FILENAME_MAX", "4096"),
            ("FOPEN_MAX", "16"),
            ("L_tmpnam", "20"),
            ("SEEK_SET", "0"),
            ("SEEK_CUR", "1"),
            ("SEEK_END", "2"),
            ("TMP_MAX", "238328"),
            ("_IOFBF", "0"),
            ("_IOLBF", "1"),
            ("_IONBF", "2"),
            ("NULL", "((void*)0)"),
        ],
    },
    CHeader {
        name: "stdlib.h",
        cpp_name: Some("cstdlib"),
        decls: &[
            "typedef struct { int quot; int rem; } div_t;",
            "typedef struct { long quot; long rem; } ldiv_t;",
            "typedef struct { long long quot; long long rem; } lldiv_t;",
            "double atof(const char *nptr);",
            "int atoi(const char *nptr);",
            "long atol(const char *nptr);",
            "long long atoll(const char *nptr);",
            "double strtod(const char *restrict nptr, char **restrict endptr);",
            "float strtof(const char *restrict nptr, char **restrict endptr);",
            "long double strtold(const char *restrict nptr, char **restrict endptr);",
            "long strtol(const char *restrict nptr, char **restrict endptr, int base);",
            "long long strtoll(const char *restrict nptr, char **restrict endptr, int base);",
            "unsigned long strtoul(const char *restrict nptr, char **restrict endptr, int base);",
            "unsigned long long strtoull(const char *restrict nptr, char **restrict endptr, int base);",
            "int rand(void);",
            "void srand(unsigned int seed);",
            "void *aligned_alloc(size_t alignment, size_t size);",
            "void *calloc(size_t nmemb, size_t size);",
            "void free(void *ptr);",
            "void *malloc(size_t size);",
            "void *realloc(void *ptr, size_t size);",
            "_Noreturn void abort(void);",
            "int atexit(void (*func)(void));",
            "int at_quick_exit(void (*func)(void));",
            "_Noreturn void exit(int status);",
            "_Noreturn void _Exit(int status);",
            "char *getenv(const char *name);",
            "_Noreturn void quick_exit(int status);",
            "int system(const char *string);",
            "void *bsearch(const void *key, const void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *));",
            "void qsort(void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *));",
            "int abs(int j);",
            "long labs(long j);",
            "long long llabs(long long j);",
            "div_t div(int numer, int denom);",
            "ldiv_t ldiv(long numer, long denom);",
            "lldiv_t lldiv(long long numer, long long denom);",
            "int mblen(const char *s, size_t n);",
            "int mbtowc(wchar_t *restrict pwc, const char *restrict s, size_t n);",
            "int wctomb(char *s, wchar_t wc);",
            "size_t mbstowcs(wchar_t *restrict pwcs, const char *restrict s, size_t n);",
            "size_t wcstombs(char *restrict s, const wchar_t *restrict pwcs, size_t n);",
        ],
        macros: &[
            ("EXIT_FAILURE", "1"),
            ("EXIT_SUCCESS", "0"),
            ("RAND_MAX", "2147483647"),
            ("MB_CUR_MAX", "(__ctype_get_mb_cur_max())"),
            ("NULL", "((void*)0)"),
        ],
    },
    CHeader {
        name: "string.h",
        cpp_name: Some("cstring"),
        decls: &[
            "void *memcpy(void *restrict s1, const void *restrict s2, size_t n);",
            "void *memmove(void *s1, const void *s2, size_t n);",
            "char *strcpy(char *restrict s1, const char *restrict s2);",
            "char *strncpy(char *restrict s1, const char *restrict s2, size_t n);",
            "char *strcat(char *restrict s1, const char *restrict s2);",
            "char *strncat(char *restrict s1, const char *restrict s2, size_t n);",
            "int memcmp(const void *s1, const void *s2, size_t n);",
            "int strcmp(const char *s1, const char *s2);",
            "int strcoll(const char *s1, const char *s2);",
            "int strncmp(const char *s1, const char *s2, size_t n);",
            "size_t strxfrm(char *restrict s1, const char *restrict s2, size_t n);",
            "void *memchr(const void *s, int c, size_t n);",
            "char *strchr(const char *s, int c);",
            "size_t strcspn(const char *s1, const char *s2);",
            "char *strpbrk(const char *s1, const char *s2);",
            "char *strrchr(const char *s, int c);",
            "size_t strspn(const char *s1, const char *s2);",
            "char *strstr(const char *s1, const char *s2);",
            "char *strtok(char *restrict s1, const char *restrict s2);",
            "void *memset(void *s, int c, size_t n);",
            "char *strerror(int errnum);",
            "size_t strlen(const char *s);",
        ],
        macros: &[("NULL", "((void*)0)")],
    },
    CHeader {
        name: "threads.h",
        cpp_name: None,
        decls: &[
            "typedef unsigned long thrd_t;",
            "typedef int (*thrd_start_t)(void *arg);",
            "typedef struct __mtx_t mtx_t;",
            "typedef struct __cnd_t cnd_t;",
            "typedef unsigned int tss_t;",
            "typedef void (*tss_dtor_t)(void *val);",
            "typedef struct __once_flag once_flag;",
            "enum { thrd_success, thrd_busy, thrd_error, thrd_nomem, thrd_timedout };",
            "enum { mtx_plain, mtx_recursive, mtx_timed };",
            "int thrd_create(thrd_t *thr, thrd_start_t func, void *arg);",
            "thrd_t thrd_current(void);",
            "int thrd_detach(thrd_t thr);",
            "int thrd_equal(thrd_t thr0, thrd_t thr1);",
            "_Noreturn void thrd_exit(int res);",
            "int thrd_join(thrd_t thr, int *res);",
            "int thrd_sleep(const struct timespec *duration, struct timespec *remaining);",
            "void thrd_yield(void);",
            "int mtx_init(mtx_t *mtx, int type);",
            "int mtx_lock(mtx_t *mtx);",
            "int mtx_timedlock(mtx_t *restrict mtx, const struct timespec *restrict ts);",
            "int mtx_trylock(mtx_t *mtx);",
            "int mtx_unlock(mtx_t *mtx);",
            "void mtx_destroy(mtx_t *mtx);",
            "int cnd_init(cnd_t *cond);",
            "int cnd_signal(cnd_t *cond);",
            "int cnd_broadcast(cnd_t *cond);",
            "int cnd_wait(cnd_t *cond, mtx_t *mtx);",
            "void cnd_destroy(cnd_t *cond);",
            "void call_once(once_flag *flag, void (*func)(void));",
            "int tss_create(tss_t *key, tss_dtor_t dtor);",
            "void *tss_get(tss_t key);",
            "int tss_set(tss_t key, void *val);",
        ],
        macros: &[("thread_local", "_Thread_local"), ("ONCE_FLAG_INIT", "{ 0 }")],
    },
    CHeader {
        name: "time.h",
        cpp_name: Some("ctime"),
        decls: &[
            "typedef long clock_t;",
            "typedef long time_t;",
            "struct tm { int tm_sec; int tm_min; int tm_hour; int tm_mday; int tm_mon; int tm_year; int tm_wday; int tm_yday; int tm_isdst; };",
            "struct timespec { time_t tv_sec; long tv_nsec; };",
            "clock_t clock(void);",
            "double difftime(time_t time1, time_t time0);",
            "time_t mktime(struct tm *timeptr);",
            "time_t time(time_t *timer);",
            "int timespec_get(struct timespec *ts, int base);",
            "char *asctime(const struct tm *timeptr);",
            "char *ctime(const time_t *timer);",
            "struct tm *gmtime(const time_t *timer);",
            "struct tm *localtime(const time_t *timer);",
            "size_t strftime(char *restrict s, size_t maxsize, const char *restrict format, const struct tm *restrict timeptr);",
        ],
        macros: &[("CLOCKS_PER_SEC", "((clock_t)1000000)"), ("TIME_UTC", "1"), ("NULL", "((void*)0)")],
    },
    CHeader {
        name: "uchar.h",
        cpp_name: Some("cuchar"),
        decls: &[
            "typedef unsigned short char16_t;",
            "typedef unsigned int char32_t;",
            "size_t mbrtoc16(char16_t *restrict pc16, const char *restrict s, size_t n, mbstate_t *restrict ps);",
            "size_t c16rtomb(char *restrict s, char16_t c16, mbstate_t *restrict ps);",
            "size_t mbrtoc32(char32_t *restrict pc32, const char *restrict s, size_t n, mbstate_t *restrict ps);",
            "size_t c32rtomb(char *restrict s, char32_t c32, mbstate_t *restrict ps);",
        ],
        macros: &[],
    },
    CHeader {
        name: "wchar.h",
        cpp_name: Some("cwchar"),
        decls: &[
            "typedef struct __mbstate_t mbstate_t;",
            "typedef unsigned int wint_t;",
            "size_t wcslen(const wchar_t *s);",
            "wchar_t *wcscpy(wchar_t *restrict s1, const wchar_t *restrict s2);",
            "wchar_t *wcsncpy(wchar_t *restrict s1, const wchar_t *restrict s2, size_t n);",
            "wchar_t *wcscat(wchar_t *restrict s1, const wchar_t *restrict s2);",
            "int wcscmp(const wchar_t *s1, const wchar_t *s2);",
            "int wcsncmp(const wchar_t *s1, const wchar_t *s2, size_t n);",
            "wchar_t *wcschr(const wchar_t *s, wchar_t c);",
            "wchar_t *wcsstr(const wchar_t *s1, const wchar_t *s2);",
            "long wcstol(const wchar_t *restrict nptr, wchar_t **restrict endptr, int base);",
            "double wcstod(const wchar_t *restrict nptr, wchar_t **restrict endptr);",
            "int wprintf(const wchar_t *restrict format, ...);",
            "int fwprintf(FILE *restrict stream, const wchar_t *restrict format, ...);",
            "int swprintf(wchar_t *restrict s, size_t n, const wchar_t *restrict format, ...);",
            "int wscanf(const wchar_t *restrict format, ...);",
            "wint_t fgetwc(FILE *stream);",
            "wint_t fputwc(wchar_t c, FILE *stream);",
            "wint_t getwc(FILE *stream);",
            "wint_t putwc(wchar_t c, FILE *stream);",
            "wint_t btowc(int c);",
            "int wctob(wint_t c);",
            "size_t mbrtowc(wchar_t *restrict pwc, const char *restrict s, size_t n, mbstate_t *restrict ps);",
            "size_t wcrtomb(char *restrict s, wchar_t wc, mbstate_t *restrict ps);",
            "size_t mbsrtowcs(wchar_t *restrict dst, const char **restrict src, size_t len, mbstate_t *restrict ps);",
            "size_t wcsrtombs(char *restrict dst, const wchar_t **restrict src, size_t len, mbstate_t *restrict ps);",
        ],
        macros: &[("WEOF", "((wint_t)-1)"), ("WCHAR_MIN", "(-2147483647-1)"), ("WCHAR_MAX", "2147483647")],
    },
    CHeader {
        name: "wctype.h",
        cpp_name: Some("cwctype"),
        decls: &[
            "typedef unsigned long wctype_t;",
            "typedef const int *wctrans_t;",
            "int iswalnum(wint_t wc);",
            "int iswalpha(wint_t wc);",
            "int iswdigit(wint_t wc);",
            "int iswlower(wint_t wc);",
            "int iswspace(wint_t wc);",
            "int iswupper(wint_t wc);",
            "int iswpunct(wint_t wc);",
            "wint_t towlower(wint_t wc);",
            "wint_t towupper(wint_t wc);",
            "wctype_t wctype(const char *property);",
            "int iswctype(wint_t wc, wctype_t desc);",
        ],
        macros: &[],
    },
    CHeader {
        name: "complex.h",
        cpp_name: None,
        decls: &[
            "double cabs(double _Complex z);",
            "double carg(double _Complex z);",
            "double cimag(double _Complex z);",
            "double creal(double _Complex z);",
            "double _Complex conj(double _Complex z);",
            "double _Complex cexp(double _Complex z);",
            "double _Complex clog(double _Complex z);",
            "double _Complex csqrt(double _Complex z);",
            "double _Complex cpow(double _Complex x, double _Complex y);",
        ],
        macros: &[("complex", "_Complex"), ("I", "_Complex_I"), ("_Complex_I", "(__extension__ 1.0iF)")],
    },
    CHeader {
        name: "stdalign.h",
        cpp_name: None,
        decls: &[],
        macros: &[("alignas", "_Alignas"), ("alignof", "_Alignof")],
    },
    CHeader {
        name: "stdnoreturn.h",
        cpp_name: None,
        decls: &[],
        macros: &[("noreturn", "_Noreturn")],
    },
];

/// What a catalog entry declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CLibKind {
    /// A typedef name (`size_t`, `FILE`).
    Type,
    /// A structure, union or enumeration tag (`struct tm`).
    Tag,
    /// An enumeration constant (`memory_order_relaxed`).
    Enumerator,
    Function,
    /// An object with external linkage (`stdin`).
    Object,
    Macro,
    FunctionMacro,
}

/// One named entry of the C library model with its stub symbol id.
#[derive(Debug, Clone)]
pub struct CLibItem {
    pub symbol_id: u32,
    pub header: &'static str,
    pub cpp_header: Option<&'static str>,
    pub name: String,
    pub kind: CLibKind,
    /// The declaration the entry comes from (absent for macros).
    pub decl: Option<ParsedDeclaration>,
    /// Source text of the declaration or of the macro.
    pub text: &'static str,
    pub macro_def: Option<MacroDefinition>,
}

/// The parsed C library: entries in catalog order, plus catalog declarations the grammar
/// rejected (reported, never dropped silently).
#[derive(Debug, Clone)]
pub struct CLibCatalog {
    pub items: Vec<CLibItem>,
    pub errors: Vec<(&'static str, &'static str, DeclError)>,
}

fn macro_from_catalog(signature: &str, body: &str) -> MacroDefinition {
    let (name, params, variadic) = match signature.find('(') {
        Some(open) => {
            let inner = signature[open + 1..].trim_end_matches(')');
            let mut variadic = false;
            let params = inner
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .filter(|p| {
                    if *p == "..." {
                        variadic = true;
                        false
                    } else {
                        true
                    }
                })
                .map(str::to_string)
                .collect::<Vec<_>>();
            (signature[..open].trim().to_string(), Some(params), variadic)
        }
        None => (signature.trim().to_string(), None, false),
    };
    MacroDefinition {
        name,
        token: u32::MAX,
        line: 0,
        params,
        variadic,
        body: body.to_string(),
    }
}

/// Parses the catalog and assigns stub symbol ids in catalog order (deterministic).
pub fn c_library_catalog() -> CLibCatalog {
    let mut oracle = NameSetOracle::default();
    for h in C_HEADERS {
        for d in h.decls {
            let toks = lex_c_text(d);
            collect_type_names(&toks, Dialect::C, &mut oracle.types);
        }
    }
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut next_id = C_STUB_BASE_ID;
    let mut push = |items: &mut Vec<CLibItem>,
                    h: &CHeader,
                    name: String,
                    kind: CLibKind,
                    decl: Option<ParsedDeclaration>,
                    text: &'static str,
                    macro_def: Option<MacroDefinition>| {
        items.push(CLibItem {
            symbol_id: next_id,
            header: h.name,
            cpp_header: h.cpp_name,
            name,
            kind,
            decl,
            text,
            macro_def,
        });
        next_id += 1;
    };
    for h in C_HEADERS {
        for &text in h.decls {
            let toks = lex_c_text(text);
            let mut p = DeclParser::new(&toks, DeclContext::plain(Dialect::C, &oracle));
            let decl = match p.parse_declaration() {
                Ok(d) => d,
                Err(e) => {
                    errors.push((h.name, text, e));
                    continue;
                }
            };
            if let Some(tag) = &decl.specifiers.tag {
                if let Some(tag_name) = &tag.name {
                    if tag.body.is_some() || decl.declarators.is_empty() {
                        push(
                            &mut items,
                            h,
                            tag_name.last_name().to_string(),
                            CLibKind::Tag,
                            Some(decl.clone()),
                            text,
                            None,
                        );
                    }
                }
                if tag.key == crate::lang::c::semantics::TagKey::Enum && tag.body.is_some() {
                    let mut ep = DeclParser::new(&toks, DeclContext::plain(Dialect::C, &oracle));
                    while !ep.at_end() && !ep.at("enum") {
                        ep.set_pos(ep.pos() + 1);
                    }
                    if let Ok(head) = ep.parse_enum_head() {
                        for e in head.enumerators {
                            push(
                                &mut items,
                                h,
                                e.name,
                                CLibKind::Enumerator,
                                Some(decl.clone()),
                                text,
                                None,
                            );
                        }
                    }
                }
            }
            for d in &decl.declarators {
                let name = match &d.name {
                    Some(n) => n.last_name().to_string(),
                    None => continue,
                };
                let kind = if decl.specifiers.storage & SC_TYPEDEF != 0 {
                    CLibKind::Type
                } else if d.ty.function().is_some() {
                    CLibKind::Function
                } else {
                    CLibKind::Object
                };
                push(&mut items, h, name, kind, Some(decl.clone()), text, None);
            }
        }
        for &(signature, body) in h.macros {
            let def = macro_from_catalog(signature, body);
            let kind = if def.params.is_some() {
                CLibKind::FunctionMacro
            } else {
                CLibKind::Macro
            };
            push(&mut items, h, def.name.clone(), kind, None, signature, Some(def));
        }
    }
    CLibCatalog { items, errors }
}

/// Stub-symbol manager of the C library, in the shape of the Java `StdLibManager`.
#[derive(Debug, Clone)]
pub struct CStdLibManager {
    pub stubs: HashMap<String, u32>,
    pub stub_records: HashMap<u32, SymbolRecord>,
    pub catalog: CLibCatalog,
    by_name: HashMap<String, Vec<usize>>,
}

impl CStdLibManager {
    pub fn new(qual_table: &mut QualifiedNameTable) -> Self {
        let catalog = c_library_catalog();
        let mut stubs = HashMap::new();
        let mut stub_records = HashMap::new();
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, item) in catalog.items.iter().enumerate() {
            by_name.entry(item.name.clone()).or_default().push(idx);
            let kind = match item.kind {
                CLibKind::Type | CLibKind::Tag => SymbolKind::SK_CLASS,
                CLibKind::Function => SymbolKind::SK_METHOD,
                CLibKind::Object => SymbolKind::SK_FIELD,
                CLibKind::Enumerator => SymbolKind::SK_ENUM_CONSTANT,
                // The symbol table has no macro kind; macros stay in the catalog only.
                CLibKind::Macro | CLibKind::FunctionMacro => continue,
            };
            let qual_name = if item.kind == CLibKind::Tag {
                format!("struct {}", item.name)
            } else {
                item.name.clone()
            };
            let qual_name_id = qual_table.get_or_intern(&qual_name);
            let param_count = item
                .decl
                .as_ref()
                .and_then(|d| {
                    d.declarators
                        .iter()
                        .find(|x| x.name.as_ref().map(|n| n.last_name()) == Some(item.name.as_str()))
                })
                .and_then(|x| x.ty.function())
                .map(|f| f.params.len() as u16)
                .unwrap_or(0);

            let record = SymbolRecord {
                symbol_id: item.symbol_id,
                name_id: u32::MAX,
                qual_name_id,
                type_id: u32::MAX,
                decl_node: u32::MAX,
                def_node: u32::MAX,
                parent_sym: u32::MAX,
                first_child: u32::MAX,
                next_sibling: u32::MAX,
                scope_id: u32::MAX,
                uml_meta_offset: 0,
                param_count,
                modifiers: 0,
                kind: kind as u8,
                visibility: SymbolVisibility::Public as u8,
                type_param_count: 0,
                flags: 0,
                first_token_id: u32::MAX,
                last_token_id: u32::MAX,
                _reserved: 0,
            };

            stubs.entry(qual_name).or_insert(item.symbol_id);
            stub_records.insert(item.symbol_id, record);
        }

        Self {
            stubs,
            stub_records,
            catalog,
            by_name,
        }
    }

    /// Stub id of a type, tag (`struct tm`), function or object.
    pub fn lookup(&self, name: &str) -> Option<u32> {
        self.stubs.get(name).copied()
    }

    /// Every catalog entry named `name` (a typedef and a tag may share a name).
    pub fn items_named(&self, name: &str) -> impl Iterator<Item = &CLibItem> + '_ {
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .map(move |&i| &self.catalog.items[i])
    }

    /// Header that declares `name`.
    pub fn header_of(&self, name: &str) -> Option<&'static str> {
        self.items_named(name).next().map(|i| i.header)
    }

    pub fn header(&self, name: &str) -> Option<&'static CHeader> {
        C_HEADERS.iter().find(|h| h.name == name || h.cpp_name == Some(name))
    }
}
