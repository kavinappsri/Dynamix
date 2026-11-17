#include "acpi.h"
#include "accommon.h"

extern void dynamix_acpica_log_write(const char *buffer, ACPI_SIZE length);

void ACPI_INTERNAL_VAR_XFACE
AcpiOsPrintf(const char *Format, ...)
{
    va_list args;

    va_start(args, Format);
    AcpiOsVprintf(Format, args);
    va_end(args);
}

void
AcpiOsVprintf(const char *Format, va_list Args)
{
    char buffer[256];
    int length = vsnprintf(buffer, sizeof(buffer), Format, Args);

    if (length <= 0) {
        return;
    }
    if ((unsigned) length >= sizeof(buffer)) {
        length = sizeof(buffer) - 1; /* truncated */
    }
    dynamix_acpica_log_write(buffer, (ACPI_SIZE) length);
}
