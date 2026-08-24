/*
Repair jpeg images, by the following operations.
- Change color components: Y,Cb,Cr
- Insert blocks
- Delete blocks
- Copy relative blocks

Adapted from jpegrepair (Copyright (c) 2017, Don Mahurin, BSD-3-Clause):
https://github.com/ImageProcessing-ElectronicPublications/jpegrepair

This build replaces the stdio file I/O with libjpeg memory source/dest so it
can be called in-process from Rust (src/backend/repair.rs) without temp files.
*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
#include <ctype.h>
#include <setjmp.h>
#include <jpeglib.h>
#include "transupp.h"

#define JPEGREPAIR_VERSION "0.20220110"
#define OP_CDELTA 1
#define OP_COPY 2
#define OP_INSERT 3
#define OP_DELETE 4

static void transform (struct jpeg_decompress_struct *srcinfo, jvirt_barray_ptr *coef_arrays, int dest_row, int dest_col, int dest_h, int dest_w, int op, int arg_count, char **args)
{
    int ci, block_y, block_x, by, bx, i;
    int nx, ny;
    JBLOCKARRAY coef_buffer;
    int dv, dh, comp, dc, d, n;
    if(op == OP_CDELTA)
    {
        comp = atoi(args[0]);
        dc = atoi(args[1]);
        d = 0;
    }
    else if(op == OP_COPY)
    {
        dv = atoi(args[0]);
        dh = atoi(args[1]);
    }
    else if(op == OP_INSERT || op == OP_DELETE)
    {
        n = atoi(args[0]);
    }

    bool reverse_order = (op == OP_INSERT || (op == OP_COPY && ((dv < 0 && dh <=0) || (dh < 0 && dv <= 0)))) ? true : false;

    for (ci=0; ci<srcinfo->num_components; ci++)
    {
        coef_buffer = (srcinfo->mem->access_virt_barray)
                      ((j_common_ptr)&srcinfo, coef_arrays[ci], 0,
                       srcinfo->comp_info[ci].v_samp_factor, TRUE);
        int h_samp_factor = srcinfo->comp_info[ci].h_samp_factor;
        int v_samp_factor = srcinfo->comp_info[ci].v_samp_factor;
        int height_in_blocks = srcinfo->comp_info[ci].height_in_blocks;
        int width_in_blocks = srcinfo->comp_info[ci].width_in_blocks;

        if(op == OP_CDELTA)
        {
            if (ci != comp) continue;
        }
        for (block_y=0; block_y<height_in_blocks; block_y++)
        {
            by = reverse_order ? (height_in_blocks - block_y - 1) : block_y;
            for (block_x=0; block_x<width_in_blocks; block_x++)
            {
                bx = reverse_order ? (width_in_blocks - block_x - 1) : block_x;
                if(dest_h == 0)   // dest_w assumed to be block count
                {
                    if((((by / v_samp_factor * width_in_blocks + bx) / h_samp_factor) < (dest_row * width_in_blocks / h_samp_factor + dest_col)) ||
                            ((dest_w > 0) &&
                            (((by / v_samp_factor * width_in_blocks + bx ) / h_samp_factor) > (dest_w + ( dest_row * width_in_blocks / h_samp_factor + dest_col))))) continue;
                }
                else if(by/v_samp_factor < dest_row || (dest_h > 0 && by/v_samp_factor >= (dest_row + dest_h)) || bx/h_samp_factor < dest_col || (dest_w > 0 && bx/h_samp_factor >= (dest_col + dest_w))) continue;

                for (i=0; i<DCTSIZE2; i++)
                {
                    if(op == OP_CDELTA)
                    {
                        if (i != d) continue;
                    }
                    if(op == OP_DELETE)
                    {
                        nx = bx + n * h_samp_factor;
                        ny = by + ( nx / width_in_blocks ) * v_samp_factor;
                        nx = nx % width_in_blocks;
                        if(ny < height_in_blocks)
                        {
                            coef_buffer[by][bx][i] = coef_buffer[ny][nx][i];
                        }
                    }
                    else if(op == OP_INSERT)
                    {
                        nx = n * h_samp_factor + width_in_blocks - 1 - bx;
                        ny = by - ( nx / width_in_blocks ) * v_samp_factor;
                        nx = width_in_blocks - ( nx % width_in_blocks ) - 1;
                        if(ny >= 0)
                        {
                            coef_buffer[by][bx][i] = coef_buffer[ny][nx][i];
                        }
                    }
                    else if(op == OP_COPY)
                    {
                        ny = by + v_samp_factor * dv;
                        nx = bx + h_samp_factor * dh;
                        if (ny >= 0 && nx >= 0 && ny < height_in_blocks && nx < width_in_blocks)
                            coef_buffer[by][bx][i] = coef_buffer[ny][nx][i];
                    }
                    if(op == OP_CDELTA)
                    {
                        coef_buffer[by][bx][i] += dc;
                    }
                }
            }
        }
    }
}

struct jp_error_mgr {
    struct jpeg_error_mgr pub;
    const char *msg;
    jmp_buf jb;
};

static void jp_error_exit (j_common_ptr cinfo)
{
    struct jp_error_mgr *myerr = (struct jp_error_mgr *)cinfo->err;
    long line = cinfo->err->msg_parm.i[0];
    (void)line;
    myerr->msg = "jpeg decode/encode failed";
    /* longjmp out of libjpeg back to the caller's setjmp */
    longjmp(myerr->jb, 1);
}

/*
Apply a sequence of jpegrepair operations to an in-memory JPEG.

  inbuf/inlen : input JPEG bytes
  outbuf/outlen : returned freshly-malloc'd output JPEG bytes (caller frees),
                  or NULL on failure
  op_count / ops : op tokens, e.g. {"delete","1"}, {"cdelta","0","100"}
                   Op stream is a flat array of C strings.

Returns 0 on success, non-zero on failure.
*/
int jp_repair_mem (const unsigned char *inbuf, unsigned long inlen,
                   unsigned char **outbuf, unsigned long *outlen,
                   int op_count, char *const *ops)
{
    struct jpeg_decompress_struct srcinfo;
    struct jpeg_compress_struct dstinfo;
    struct jp_error_mgr jerr;
    jvirt_barray_ptr *coef_arrays;

    unsigned char *dst_buf = NULL;
    unsigned long dst_size = 0;
    int ret = 1;

    *outbuf = NULL;
    *outlen = 0;

    /* Set up the shared error manager once. jpeg_std_error() initializes every
       handler, including error_exit; calling it again for dstinfo would clobber
       our longjmp-based handler with libjpeg's default, which calls exit(). */
    jerr.pub = *jpeg_std_error(&jerr.pub);
    jerr.pub.error_exit = jp_error_exit;
    jerr.msg = NULL;

    srcinfo.err = &jerr.pub;
    if (setjmp(jerr.jb)) {
        goto cleanup;
    }
    jpeg_create_decompress(&srcinfo);

    dstinfo.err = &jerr.pub;
    if (setjmp(jerr.jb)) {
        goto cleanup;
    }
    jpeg_create_compress(&dstinfo);

    jpeg_mem_src(&srcinfo, (unsigned char *)inbuf, inlen);

    jcopy_markers_setup(&srcinfo, JCOPYOPT_ALL);

    if (jpeg_read_header(&srcinfo, TRUE) != JPEG_HEADER_OK)
        goto cleanup;

    coef_arrays = jpeg_read_coefficients(&srcinfo);
    if (coef_arrays == NULL)
        goto cleanup;

    jpeg_copy_critical_parameters(&srcinfo, &dstinfo);

    int dest_row = 0, dest_col = 0, dest_h = -1, dest_w = -1;
    for (int i = 0; i < op_count; )
    {
        const char *tk = ops[i];
        int op = 0;
        if(!strcmp(tk, "dest"))
        {
            if (i + 2 >= op_count) break;
            dest_row = atoi(ops[i+1]);
            dest_col = atoi(ops[i+2]);
            i += 3;
            dest_h = -1; dest_w = -1;
            if (i < op_count && (isdigit(ops[i][0]) || (ops[i][0] == '-' && isdigit(ops[i][1])))) {
                dest_h = atoi(ops[i]); i += 1;
                if (i < op_count && (isdigit(ops[i][0]) || (ops[i][0] == '-' && isdigit(ops[i][1])))) {
                    dest_w = atoi(ops[i]); i += 1;
                } else { dest_w = dest_h; dest_h = 0; }
            } else {
                dest_h = 0;
            }
            continue;
        }
        else if(!strcmp(tk, "copy"))
        {
            if (i + 2 >= op_count) break;
            op = OP_COPY;
            transform(&srcinfo, coef_arrays, dest_row, dest_col, dest_h, dest_w, op, 2, (char **)(ops + i + 1));
            i += 3;
        }
        else if(!strcmp(tk, "cdelta"))
        {
            if (i + 2 >= op_count) break;
            op = OP_CDELTA;
            transform(&srcinfo, coef_arrays, dest_row, dest_col, dest_h, dest_w, op, 2, (char **)(ops + i + 1));
            i += 3;
        }
        else if(!strcmp(tk, "insert"))
        {
            if (i + 1 >= op_count) break;
            transform(&srcinfo, coef_arrays, dest_row, dest_col, dest_h, dest_w, OP_INSERT, 1, (char **)(ops + i + 1));
            i += 2;
        }
        else if(!strcmp(tk, "delete"))
        {
            if (i + 1 >= op_count) break;
            transform(&srcinfo, coef_arrays, dest_row, dest_col, dest_h, dest_w, OP_DELETE, 1, (char **)(ops + i + 1));
            i += 2;
        }
        else
        {
            break;
        }
    }

    jpeg_mem_dest(&dstinfo, &dst_buf, &dst_size);

    jpeg_write_coefficients(&dstinfo, coef_arrays);

    jcopy_markers_execute(&srcinfo, &dstinfo, JCOPYOPT_ALL);

    jpeg_finish_compress(&dstinfo);
    jpeg_destroy_compress(&dstinfo);
    jpeg_finish_decompress(&srcinfo);
    jpeg_destroy_decompress(&srcinfo);

    *outbuf = dst_buf;
    *outlen = dst_size;
    ret = 0;
    return ret;

cleanup:
    if (dst_buf) free(dst_buf);
    return ret;
}