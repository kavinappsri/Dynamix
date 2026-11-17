/*
 * ACPICA platform header for DynamixOS (x86_64, UEFI-built kernel).
 *
 * ACPICA includes this from `platform/acenv.h` when `_DYNAMIX` is defined
 * (see third_party/acpica-dynamix.patch). It is deliberately kept out of the
 * vendored ACPICA tree, in this crate, so it stays ours.
 *
 * Contract with the Rust OSL (hal/src/services/acpica_osl.rs):
 *  - ACPI_USE_LOCAL_CACHE: ACPICA keeps its own object caches, so the OSL
 *    does not implement AcpiOsCreateCache and friends.
 *  - AcpiOsAllocateZeroed is the OSL's (USE_NATIVE_ALLOCATE_ZEROED).
 *  - ACPI_MUTEX_TYPE is left at its default (ACPI_BINARY_SEMAPHORE), so the
 *    OSL does not implement AcpiOsCreateMutex and friends.
 *  - No ACPI_DEBUGGER / ACPI_DISASSEMBLER / ACPI_DEBUG_OUTPUT.
 *  - No C library. ACPICA brings its own (utclib.c, utprint.c).
 */

#ifndef __ACDYNAMIX_H__
#define __ACDYNAMIX_H__

#define ACPI_MACHINE_WIDTH          64
#define ACPI_USE_LOCAL_CACHE

/* The OSL provides AcpiOsAllocateZeroed itself; skip ACPICA's default one. */
#define USE_NATIVE_ALLOCATE_ZEROED

/*
 * ACPICA's built-in C library defines global symbols with libc names
 * (memcpy, memset, strlen, vsnprintf, ...). Linked into a Rust kernel those
 * would collide with, or silently replace, the memcpy/memset that Rust's
 * compiler_builtins provides for the whole kernel. So every one of them is
 * renamed here; this header reaches every ACPICA translation unit before
 * acclib.h declares them. Calls the compiler itself generates (struct
 * copies, ...) keep going to the real memcpy and friends.
 */
#define memcmp      acpica_memcmp
#define memcpy      acpica_memcpy
#define memmove     acpica_memmove
#define memset      acpica_memset
#define snprintf    acpica_snprintf
#define sprintf     acpica_sprintf
#define strcat      acpica_strcat
#define strcmp      acpica_strcmp
#define strcpy      acpica_strcpy
#define strlen      acpica_strlen
#define strncat     acpica_strncat
#define strncmp     acpica_strncmp
#define strncpy     acpica_strncpy
#define strpbrk     acpica_strpbrk
#define strstr      acpica_strstr
#define strtok      acpica_strtok
#define strtoul     acpica_strtoul
#define tolower     acpica_tolower
#define toupper     acpica_toupper
#define vsnprintf   acpica_vsnprintf

/* Cache flush used around sleep-state entry. */
#define ACPI_FLUSH_CPU_CACHE()      __asm__ volatile ("wbinvd" ::: "memory")

/*
 * Global Lock (FACS): the handshake with firmware (embedded controller etc.)
 * that real hardware needs. Same protocol as Linux's x86 implementation:
 * bit 0 = pending, bit 1 = owned.
 *   acquire: returns nonzero if the lock was taken, 0 if the owner is firmware
 *            (pending is then set and a release event will follow)
 *   release: returns nonzero if firmware has a pending request
 */
static __inline__ int dynamix_acquire_global_lock(volatile unsigned int *lock)
{
    unsigned int old, new_val, seen;

    do {
        old = *lock;
        new_val = ((old & ~0x3u) + 2u) + ((old >> 1) & 0x1u);
        seen = __sync_val_compare_and_swap(lock, old, new_val);
    } while (seen != old);

    return ((new_val & 0x3u) < 3u) ? -1 : 0;
}

static __inline__ int dynamix_release_global_lock(volatile unsigned int *lock)
{
    unsigned int old, new_val, seen;

    do {
        old = *lock;
        new_val = old & ~0x3u;
        seen = __sync_val_compare_and_swap(lock, old, new_val);
    } while (seen != old);

    return (int) (old & 0x1u);
}

#define ACPI_ACQUIRE_GLOBAL_LOCK(GLptr, Acq) \
    ((Acq) = dynamix_acquire_global_lock(&(GLptr)->GlobalLock))
#define ACPI_RELEASE_GLOBAL_LOCK(GLptr, Pnd) \
    ((Pnd) = dynamix_release_global_lock(&(GLptr)->GlobalLock))

#endif /* __ACDYNAMIX_H__ */
