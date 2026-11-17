/*
* Second platform header, included at the very end of platform/acenvex.h,
 * i.e. after the compiler headers. acgccex.h does `#undef strchr` (a GCC
 * quirk), which would undo the rename started in acdynamix.h, so it is
 * redone here. See the symbol rename comment in acdynamix.h.
 */

#ifndef __ACDYNAMIXEX_H__
#define __ACDYNAMIXEX_H__

#define strchr      acpica_strchr

#endif /* __ACDYNAMIXEX_H__ */
