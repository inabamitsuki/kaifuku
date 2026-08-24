#ifndef JPEGREPAIR_MEM_H
#define JPEGREPAIR_MEM_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

int jp_repair_mem (const unsigned char *inbuf, unsigned long inlen,
                   unsigned char **outbuf, unsigned long *outlen,
                   int op_count, char *const *ops);

#ifdef __cplusplus
}
#endif

#endif