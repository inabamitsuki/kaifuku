#include "kaifuku_ffi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <stdarg.h>
#include <time.h>
#include <pthread.h>
#include <limits.h>

#include "common.h"
#include "filegen.h"
#include "photorec.h"
#include "hdaccess.h"
#include "hdcache.h"
#include "log.h"
#include "psearchn.h"
#include "sessionp.h"
#include "phcfg.h"
#include "file_tar.h"
#include "psearch.h"
#include "file_found.h"
#include "list.h"
#include "fnctdsk.h"
#include "phbf.h"
#include "fat_unformat.h"

extern file_enable_t array_file_enable[];
int need_to_stop = 0;

struct kaifuku_ctx {
    volatile int running;
    volatile int stop_requested;
    pthread_t scan_thread;

    kaifuku_callbacks_t callbacks;

    struct ph_param params;
    struct ph_options options;
    alloc_data_t *list_search_space;
    disk_t *disk;
    char *output_dir;
    char *device;
    char *dir_filter;
    uint64_t part_offset;
    uint64_t part_size;
    int fs_pass;
};

kaifuku_ctx_t *kaifuku_init(void)
{
    kaifuku_ctx_t *ctx = (kaifuku_ctx_t *)calloc(1, sizeof(kaifuku_ctx_t));
    if (!ctx) return NULL;

    srand((unsigned int)(time(NULL) & 0xffffffff));

    memset(&ctx->params, 0, sizeof(ctx->params));
    ctx->options.paranoid = 1;
    ctx->options.keep_corrupted_file = 0;
    ctx->options.mode_ext2 = 0;
    ctx->options.expert = 0;
    ctx->options.lowmem = 0;
    ctx->options.verbose = 0;
    ctx->options.list_file_format = array_file_enable;

    reset_array_file_enable(ctx->options.list_file_format);

    ctx->running = 0;
    ctx->stop_requested = 0;

    return ctx;
}

/* Cheap boot-sector test: does the partition at part_offset look like FAT? */
static int kf_partition_is_fat(disk_t *disk, uint64_t part_offset, unsigned int sector_size)
{
    unsigned char buf[512];
    if (disk->pread(disk, buf, sizeof(buf), part_offset) != (int)sizeof(buf))
        return 0;
    if ((buf[0x0B] | (buf[0x0C] << 8)) != sector_size)
        return 0;
    if (buf[0x0D] == 0)
        return 0;
    if (memcmp(buf + 0x36, "FAT", 3) == 0)
        return 1;
    if (memcmp(buf + 0x52, "FAT", 3) == 0)
        return 1;
    if (memcmp(buf + 0x03, "MSDOS5.0", 8) == 0)
        return 1;
    return 0;
}

/* Cheap boot-sector test: does the partition at part_offset look like NTFS? */
static int kf_partition_is_ntfs(disk_t *disk, uint64_t part_offset)
{
    unsigned char buf[512];
    if (disk->pread(disk, buf, sizeof(buf), part_offset) != (int)sizeof(buf))
        return 0;
    if (memcmp(buf + 0x03, "NTFS    ", 8) != 0)
        return 0;
    return 1;
}

static void kf_log(kaifuku_ctx_t *ctx, const char *msg)
{
    if (ctx->callbacks.log_msg)
        ctx->callbacks.log_msg(msg, ctx->callbacks.user_data);
}

static void kf_logf(kaifuku_ctx_t *ctx, const char *fmt, ...)
{
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    kf_log(ctx, buf);
}

static void *scan_thread_func(void *arg)
{
    kaifuku_ctx_t *ctx = (kaifuku_ctx_t *)arg;
    pstatus_t result = PSTATUS_OK;
    ctx->params.recup_dir = strdup(ctx->output_dir);

    ctx->disk = file_test_availability(ctx->device, 0,
        TESTDISK_O_RDONLY | TESTDISK_O_READAHEAD_32K);
    if (!ctx->disk) {
        kf_logf(ctx, "Failed to open device: %s", ctx->device);
        ctx->running = 0;
        return NULL;
    }

    ctx->disk = new_diskcache(ctx->disk, TESTDISK_O_RDONLY | TESTDISK_O_READAHEAD_32K);

    ctx->params.disk = ctx->disk;

    partition_t *partition;
    if (ctx->part_offset == 0 && ctx->part_size == 0) {
        partition = new_whole_disk(ctx->disk);
    } else {
        partition = partition_new(ctx->disk->arch);
        if (partition) {
            partition->part_offset = ctx->part_offset;
            partition->part_size = ctx->part_size;
            partition->blocksize = ctx->disk->sector_size;
            partition->status = STATUS_PRIM;
            snprintf(partition->fsname, sizeof(partition->fsname), "Selected");
        }
    }
    if (!partition) {
        kf_log(ctx, "Failed to create partition");
        ctx->running = 0;
        return NULL;
    }
    ctx->params.partition = partition;

    ctx->list_search_space = (alloc_data_t *)MALLOC(sizeof(alloc_data_t));
    TD_INIT_LIST_HEAD(&ctx->list_search_space->list);

    init_search_space(ctx->list_search_space, ctx->disk, partition);

    ctx->params.carve_free_space_only = 0;
    params_reset(&ctx->params, &ctx->options);

    if (ctx->fs_pass) {
        ctx->params.status = STATUS_UNFORMAT;
    }

    ctx->params.progress_cb = ctx->callbacks.progress;
    ctx->params.file_found_cb = ctx->callbacks.file_found;
    ctx->params.user_data = ctx->callbacks.user_data;

    ctx->params.dir_num = photorec_mkdir(ctx->params.recup_dir, ctx->params.dir_num);

    kf_logf(ctx, "Recovery started — destination: %s", ctx->params.recup_dir);

    for (ctx->params.pass = 0;
         ctx->params.status != STATUS_QUIT && !ctx->stop_requested;
         ctx->params.pass++)
    {
        need_to_stop = 0;
        result = PSTATUS_OK;

        switch (ctx->params.status) {
        case STATUS_FIND_OFFSET:
        {
            uint64_t start_offset = 0;
            unsigned int bs = find_blocksize(ctx->list_search_space, 512, &start_offset);
            ctx->params.blocksize = bs;
            update_blocksize(ctx->params.blocksize, ctx->list_search_space, start_offset);
            kf_logf(ctx, "Sector size: %u bytes, %llu sectors in scan area",
                bs, (unsigned long long)(ctx->params.partition->part_size / (bs ? bs : 1)));
            break;
        }
        case STATUS_UNFORMAT:
            if (kf_partition_is_fat(ctx->disk, ctx->params.partition->part_offset,
                ctx->disk->sector_size)) {
                kf_log(ctx, "Filesystem structure pass: FAT");
                if (ctx->dir_filter != NULL && ctx->dir_filter[0] != '\0')
                    result = fat_unformat_dir(&ctx->params, &ctx->options,
                        ctx->list_search_space, ctx->dir_filter);
                else
                    result = fat_unformat(&ctx->params, &ctx->options, ctx->list_search_space);
            }
            else if (kf_partition_is_ntfs(ctx->disk, ctx->params.partition->part_offset) &&
                ctx->callbacks.ntfs_unformat)
            {
                kf_log(ctx, "Filesystem structure pass: NTFS (MFT)");
                uint64_t nbr = ctx->callbacks.ntfs_unformat(ctx, ctx->params.recup_dir,
                    ctx->params.dir_num, ctx->dir_filter,
                    ctx->params.partition->part_offset,
                    ctx->params.partition->part_size,
                    ctx->disk->sector_size, ctx->callbacks.user_data);
                ctx->params.file_nbr += (unsigned int)nbr;
                result = PSTATUS_OK;
            }
            break;
        case STATUS_EXT2_ON_BF:
        case STATUS_EXT2_OFF_BF:
            kf_log(ctx, "Brute-force fragmented-file reassembly");
            result = photorec_bf(&ctx->params, &ctx->options, ctx->list_search_space);
            break;
        default:
            kf_log(ctx, "Carving for file types");
            result = photorec_aux(&ctx->params, &ctx->options, ctx->list_search_space);
            break;
        }

        if (ctx->stop_requested) {
            need_to_stop = 1;
            break;
        }

        if (result == PSTATUS_EACCES) {
            kf_log(ctx, "Cannot create files - permission error");
            break;
        }

        if (result == PSTATUS_ENOSPC) {
            kf_log(ctx, "No space left on destination");
            break;
        }

        if (result == PSTATUS_OK) {
            status_inc(&ctx->params, &ctx->options);
        }

        if (ctx->callbacks.progress) {
            ctx->callbacks.progress(100, "", ctx->params.file_nbr, ctx->callbacks.user_data);
        }
    }

    need_to_stop = 0;

    free_header_check();
    free(ctx->params.file_stats);
    ctx->params.file_stats = NULL;

    kf_logf(ctx, "Scan complete. Files recovered: %u", ctx->params.file_nbr);

    if (ctx->list_search_space) {
        free_list_search_space(ctx->list_search_space);
        free_search_space(ctx->list_search_space);
        free(ctx->list_search_space);
        ctx->list_search_space = NULL;
    }

    if (ctx->params.partition) {
        free(ctx->params.partition);
        ctx->params.partition = NULL;
    }

    if (ctx->disk) {
        generic_clean(ctx->disk);
        ctx->disk = NULL;
    }

    ctx->running = 0;
    return NULL;
}

int kaifuku_start_scan(kaifuku_ctx_t *ctx, const char *device_path,
    const char *output_dir, kaifuku_callbacks_t callbacks,
    uint64_t part_offset, uint64_t part_size)
{
    if (!ctx || ctx->running) return -1;

    ctx->device = strdup(device_path);
    ctx->output_dir = strdup(output_dir);
    ctx->callbacks = callbacks;
    ctx->part_offset = part_offset;
    ctx->part_size = part_size;
    ctx->running = 1;
    ctx->stop_requested = 0;

    if (pthread_create(&ctx->scan_thread, NULL, scan_thread_func, ctx) != 0) {
        ctx->running = 0;
        free(ctx->device);
        free(ctx->output_dir);
        ctx->device = NULL;
        ctx->output_dir = NULL;
        return -1;
    }
    pthread_detach(ctx->scan_thread);

    return 0;
}

int kaifuku_is_running(kaifuku_ctx_t *ctx)
{
    return ctx ? ctx->running : 0;
}

int kaifuku_stop_requested(kaifuku_ctx_t *ctx)
{
    return ctx ? ctx->stop_requested : 1;
}

void kaifuku_stop(kaifuku_ctx_t *ctx)
{
    if (ctx) {
        ctx->stop_requested = 1;
        need_to_stop = 1;
    }
}

void kaifuku_destroy(kaifuku_ctx_t *ctx)
{
    if (!ctx) return;

    kaifuku_stop(ctx);

    while (ctx->running) {
        struct timespec ts = {0, 10000000};
        nanosleep(&ts, NULL);
    }

    free(ctx->device);
    free(ctx->output_dir);
    free(ctx->dir_filter);
    free(ctx->params.recup_dir);

    free(ctx);
}

void kaifuku_log_set_callback(kaifuku_ctx_t *ctx, kaifuku_log_cb cb, void *user_data)
{
  if (ctx) {
    ctx->callbacks.log_msg = cb;
    ctx->callbacks.user_data = user_data;
  }
}

void kaifuku_enumerate_extensions(kaifuku_extension_cb cb, void *user_data)
{
  file_enable_t *file_enable;
  if (!cb)
    return;
  for (file_enable = array_file_enable; file_enable->file_hint != NULL; file_enable++)
    if (file_enable->file_hint->extension != NULL)
      cb(file_enable->file_hint->extension, user_data);
}

int kaifuku_set_frag_reassembly(kaifuku_ctx_t *ctx, int enabled)
{
  if (!ctx || ctx->running)
    return -1;
  ctx->options.paranoid = enabled ? 2 : 1;
  return 0;
}

int kaifuku_set_filesystem_pass(kaifuku_ctx_t *ctx, int enabled)
{
  if (!ctx || ctx->running)
    return -1;
  ctx->fs_pass = enabled ? 1 : 0;
  return 0;
}

int kaifuku_set_directory_filter(kaifuku_ctx_t *ctx, const char *dir_path)
{
  if (!ctx || ctx->running)
    return -1;
  free(ctx->dir_filter);
  ctx->dir_filter = NULL;
  if (dir_path != NULL && dir_path[0] != '\0')
    ctx->dir_filter = strdup(dir_path);
  return 0;
}

ssize_t kaifuku_pread(kaifuku_ctx_t *ctx, uint64_t offset,
    unsigned char *buf, size_t count)
{
  size_t done = 0;
  if (!ctx || !ctx->disk)
    return -1;
  while (done < count)
  {
    unsigned int chunk;
    int r;
    if ((size_t)(count - done) > (size_t)INT_MAX)
      chunk = (unsigned int)INT_MAX;
    else
      chunk = (unsigned int)(count - done);
    r = ctx->disk->pread(ctx->disk, buf + done, chunk, offset + done);
    if (r <= 0)
      return done ? (ssize_t)done : -1;
    done += (size_t)r;
    if ((size_t)r < chunk)
      break;
  }
  return (ssize_t)done;
}

void kaifuku_mark_used_ranges(kaifuku_ctx_t *ctx, const kaifuku_range_t *ranges,
    size_t count)
{
  size_t i;
  if (!ctx || !ctx->list_search_space || ranges == NULL)
    return;
  for (i = 0; i < count; i++)
  {
    const kaifuku_range_t *r = &ranges[i];
    if (r->size > 0)
      del_search_space(ctx->list_search_space, r->offset, r->offset + r->size - 1);
  }
}

void kaifuku_set_file_filter(const char * const *extensions, size_t count)
{
  file_enable_t *file_enable;
  if (extensions == NULL || count == 0)
  {
    reset_array_file_enable(array_file_enable);
    return;
  }
  for (file_enable = array_file_enable; file_enable->file_hint != NULL; file_enable++)
    file_enable->enable = 0;
  for (size_t i = 0; i < count; i++)
  {
    if (extensions[i] == NULL)
      continue;
    for (file_enable = array_file_enable; file_enable->file_hint != NULL; file_enable++)
    {
      if (file_enable->file_hint->extension != NULL &&
          strcasecmp(file_enable->file_hint->extension, extensions[i]) == 0)
      {
        file_enable->enable = 1;
      }
    }
  }
}
