#ifndef KAIFUKU_FFI_H
#define KAIFUKU_FFI_H

#include <stdint.h>
#include <stddef.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*kaifuku_progress_cb)(int percent, const char *current_file,
    uint64_t files_found, void *user_data);

typedef void (*kaifuku_file_found_cb)(const char *filename, const char *extension,
    uint64_t size, void *user_data);

typedef int (*kaifuku_log_cb)(const char *message, void *user_data);

typedef void (*kaifuku_extension_cb)(const char *extension, void *user_data);

typedef struct kaifuku_ctx kaifuku_ctx_t;

/* Run the NTFS structure pass. Called from the C scan thread when the
 * partition is NTFS and fs_pass is enabled. The implementation reads sectors
 * through kaifuku_pread() and marks used space with kaifuku_mark_used_ranges().
 * Returns the number of files recovered. */
typedef uint64_t (*kaifuku_ntfs_unformat_cb)(kaifuku_ctx_t *ctx,
    const char *recup_dir, uint32_t dir_num, const char *dir_filter,
    uint64_t part_offset, uint64_t part_size, uint32_t sector_size,
    void *user_data);

typedef struct {
    kaifuku_progress_cb progress;
    kaifuku_file_found_cb file_found;
    kaifuku_log_cb log_msg;
    kaifuku_ntfs_unformat_cb ntfs_unformat;
    void *user_data;
} kaifuku_callbacks_t;

typedef struct {
    uint64_t offset;
    uint64_t size;
    uint32_t blocksize;
    char fsname[128];
} kaifuku_partition_info_t;

typedef struct {
    uint64_t offset;
    uint64_t size;
} kaifuku_range_t;

kaifuku_ctx_t *kaifuku_init(void);

int kaifuku_start_scan(kaifuku_ctx_t *ctx, const char *device_path,
    const char *output_dir, kaifuku_callbacks_t callbacks,
    uint64_t part_offset, uint64_t part_size);
/* If part_offset == 0 and part_size == 0, scan the whole disk */

int kaifuku_is_running(kaifuku_ctx_t *ctx);

/* Non-zero when a stop has been requested (for long-running passes that check
 * from inside a callback). */
int kaifuku_stop_requested(kaifuku_ctx_t *ctx);

void kaifuku_stop(kaifuku_ctx_t *ctx);

void kaifuku_destroy(kaifuku_ctx_t *ctx);

void kaifuku_log_set_callback(kaifuku_ctx_t *ctx, kaifuku_log_cb cb, void *user_data);

/* Enumerate every file extension the scan engine knows about (file_hint list). */
void kaifuku_enumerate_extensions(kaifuku_extension_cb cb, void *user_data);

/* Restrict recovery to the listed extensions. Passing NULL or count==0 resets
 * to the engine defaults (every format enabled by default). */
void kaifuku_set_file_filter(const char * const *extensions, size_t count);

/* Enable/disable the brute-force fragmented-file reassembly pass.
 * enabled != 0 raises paranoia to 2 so the engine's photorec_bf runs after
 * the normal carve; enabled == 0 restores paranoia 1 (contiguous carve only).
 * Returns 0 on success, -1 if the scan is already running. */
int kaifuku_set_frag_reassembly(kaifuku_ctx_t *ctx, int enabled);

/* Enable/disable the filesystem-structure pass. When enabled, files are first
 * recovered by following the filesystem structure (FAT cluster table / NTFS
 * MFT; exact names, fragmentation handled, clusters removed from the carve
 * space) before the normal carve runs. No-op on other partitions.
 * Returns 0 on success, -1 if the scan is already running. */
int kaifuku_set_filesystem_pass(kaifuku_ctx_t *ctx, int enabled);

/* Restrict the filesystem-structure pass to a single directory (Windows
 * style path, e.g. "\\Documents"). Only files under that path are recovered
 * by the structure pass. Pass NULL or "" to recover the whole volume.
 * Returns 0 on success, -1 if the scan is already running. */
int kaifuku_set_directory_filter(kaifuku_ctx_t *ctx, const char *dir_path);

/* Read 'count' bytes at absolute byte 'offset' through the scan disk.
 * Returns the number of bytes read, or -1 on error. Short reads are possible. */
ssize_t kaifuku_pread(kaifuku_ctx_t *ctx, uint64_t offset,
    unsigned char *buf, size_t count);

/* Remove the given absolute byte ranges from the carve search space, so the
 * carve pass skips them. 'ranges' is an array of (offset,size) pairs.
 * Safe to call only from the scan thread (or the ntfs_unformat callback). */
void kaifuku_mark_used_ranges(kaifuku_ctx_t *ctx, const kaifuku_range_t *ranges,
    size_t count);

#ifdef __cplusplus
}
#endif

#endif
