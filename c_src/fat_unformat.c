/*

    File: fat_unformat.c

    Copyright (C) 2009-2012 Christophe GRENIER <grenier@cgsecurity.org>

    This software is free software; you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation; either version 2 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License along
    with this program; if not, write the Free Software Foundation, Inc., 51
    Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA.

 */

#ifdef HAVE_CONFIG_H
#include <config.h>
#endif
#ifdef HAVE_STDLIB_H
#include <stdlib.h>
#endif
#ifdef HAVE_STRING_H
#include <string.h>
#endif
#ifdef HAVE_TIME_H
#include <time.h>
#endif
#ifdef HAVE_SYS_TIME_H
#include <sys/time.h>
#endif
#include <stdio.h>
#include "types.h"
#include "common.h"
#include "intrf.h"
#include "intrfn.h"
#include "dir.h"
#include "fat.h"
#include "fat_dir.h"
#include "list.h"
#include "filegen.h"
#include "photorec.h"
#include "log.h"
#include "pblocksize.h"
#include "fat_cluster.h"
#include "fat_unformat.h"
#include "pnext.h"
#include "setdate.h"
#include "fat_common.h"
#include <assert.h>

#ifndef DISABLED_FOR_FRAMAC
extern int need_to_stop;

#define READ_SIZE 4*1024*1024
static int pfind_sectors_per_cluster(disk_t *disk, const partition_t *partition, const int verbose, unsigned int *sectors_per_cluster, uint64_t *offset_org, alloc_data_t *list_search_space)
{
  uint64_t offset=0;
  uint64_t next_offset=0;
  uint64_t diff_offset=0;
  time_t previous_time=0;
  unsigned int nbr_subdir=0;
  sector_cluster_t sector_cluster[10];
  alloc_data_t *current_search_space;
  unsigned char *buffer_start=(unsigned char *)MALLOC(READ_SIZE);
  unsigned char *buffer=buffer_start;
  assert(disk->sector_size!=0);
  current_search_space=td_list_first_entry(&list_search_space->list, alloc_data_t, list);
  if(current_search_space!=list_search_space)
    offset=current_search_space->start;
  if(verbose>0)
    info_list_search_space(list_search_space, current_search_space, disk->sector_size, 0, verbose);
#ifdef HAVE_NCURSES
  wmove(stdscr,22,0);
  wattrset(stdscr, A_REVERSE);
  waddstr(stdscr,"  Stop  ");
  wattroff(stdscr, A_REVERSE);
#endif
  disk->pread(disk, buffer_start, READ_SIZE, offset);
  while(current_search_space!=list_search_space && nbr_subdir<10)
  {
    const uint64_t old_offset=offset;
    if(buffer[0]=='.' && is_fat_directory(buffer))
    {
      const unsigned long int cluster=fat_get_cluster_from_entry((const struct msdos_dir_entry *)buffer);
      log_info("sector %lu, cluster %lu\n",
	  (unsigned long)(offset/disk->sector_size), cluster);
      sector_cluster[nbr_subdir].cluster=cluster;
      sector_cluster[nbr_subdir].sector=offset/disk->sector_size;
      log_flush();
      nbr_subdir++;
    }
    get_next_sector(list_search_space, &current_search_space, &offset, 512);
    buffer+=512;
    if( old_offset+512!=offset ||
        buffer+512>buffer_start+READ_SIZE)
    {
      buffer=buffer_start;
#ifdef HAVE_NCURSES
      if(offset > next_offset)
      {
	const time_t current_time=time(NULL);
	if(current_time==previous_time)
	  diff_offset<<=1;
	else
	  diff_offset>>=1;
	if(diff_offset < disk->sector_size)
	  diff_offset=disk->sector_size;
	next_offset=offset+diff_offset;
	previous_time=current_time;
	wmove(stdscr,9,0);
	wclrtoeol(stdscr);
	wprintw(stdscr,"Search subdirectory %10lu/%lu %u",(unsigned long)(offset/disk->sector_size),(unsigned long)(partition->part_size/disk->sector_size),nbr_subdir);
	wrefresh(stdscr);
      }
#endif
      if(verbose>1)
      {
        log_verbose("Reading sector %10llu/%llu\n",
	    (unsigned long long)((offset-partition->part_offset)/disk->sector_size),
	    (unsigned long long)((partition->part_size-1)/disk->sector_size));
      }
      if(disk->pread(disk, buffer_start, READ_SIZE, offset) != READ_SIZE)
      {
#ifdef HAVE_NCURSES
	wmove(stdscr,11,0);
	wclrtoeol(stdscr);
	wprintw(stdscr,"Error reading sector %10lu\n",
	    (unsigned long)((offset - partition->part_offset) / disk->sector_size));
#endif
      }
    }
  } /* end while(current_search_space!=list_search_space) */
  free(buffer_start);
  return find_sectors_per_cluster_aux(sector_cluster,nbr_subdir,sectors_per_cluster,offset_org,verbose,partition->part_size/disk->sector_size, UP_UNK);
}

static void strip_fn(char *fn)
{
  unsigned int i;
  for(i=0; fn[i]!='\0'; i++);
  while(i>0 && (fn[i-1]==' ' || fn[i-1]=='.'))
    i--;
  if(i==0 && (fn[0]==' ' || fn[0]=='.'))
    fn[i++]='_';
  fn[i]='\0';
}

static copy_file_t fat_copy_file(disk_t *disk, const partition_t *partition, const unsigned int cluster_size, const uint64_t start_data, const char *recup_dir, const unsigned int dir_num, const unsigned int inode_num, const file_info_t *file)
{
  char *new_file;	
  FILE *f_out;
  unsigned int cluster;
  unsigned int file_size=file->st_size;
  const unsigned long int no_of_cluster=(partition->part_size - start_data) / cluster_size;
  unsigned char *buffer_file=(unsigned char *)MALLOC(cluster_size);
  cluster = file->st_ino;
  new_file=(char *)MALLOC(1024);
#ifdef HAVE_MKDIR
  snprintf(new_file, 1024, "%s.%u/inode_%u", recup_dir, dir_num, inode_num);
#ifdef __MINGW32__
  mkdir(new_file);
#else
  (void)mkdir(new_file, 0775);
#endif
#endif
  snprintf(new_file, 1024, "%s.%u/inode_%u/%s", recup_dir, dir_num, inode_num,
      file->name);
  strip_fn(new_file);
  if((f_out=fopen(new_file, "rb"))!=NULL)
  {
    fclose(f_out);
    snprintf(new_file, 1024, "%s.%u/inode_%u/f%07u-%s", recup_dir, dir_num, inode_num,
	(unsigned int)((start_data - partition->part_offset + (uint64_t)(cluster-2)*cluster_size)/disk->sector_size),
	file->name);
    strip_fn(new_file);
  }
  log_info("fat_copy_file %s\n", new_file);
  f_out=fopen(new_file, "wb");
  if(!f_out)
  {
    log_critical("Can't create file %s: \n",new_file);
    free(new_file);
    free(buffer_file);
    return CP_CREATE_FAILED;
  }
  while(cluster>=2 && cluster<=no_of_cluster+2 && file_size>0)
  {
    const uint64_t start=start_data + (uint64_t)(cluster-2)*cluster_size;
    unsigned int toread = cluster_size;
    if (toread > file_size)
      toread = file_size;
    if((unsigned)disk->pread(disk, buffer_file, toread, start) != toread)
    {
      log_error("fat_copy_file: Can't read cluster %u.\n", cluster);
    }
    if(fwrite(buffer_file, 1, toread, f_out) != toread)
    {
      log_error("fat_copy_file: no space left on destination.\n");
      fclose(f_out);
      set_date(new_file, file->td_atime, file->td_mtime);
      free(new_file);
      free(buffer_file);
      return CP_NOSPACE;
    }
    file_size -= toread;
    cluster++;
  }
  fclose(f_out);
  set_date(new_file, file->td_atime, file->td_mtime);
  free(new_file);
  free(buffer_file);
  return CP_OK;
}

static pstatus_t fat_unformat_aux(struct ph_param *params, const struct ph_options *options, const uint64_t start_data, alloc_data_t *list_search_space)
{
  pstatus_t ind_stop=PSTATUS_OK;
  uint64_t offset;
  uint64_t offset_end;
  unsigned char *buffer_start;
  unsigned char *buffer;
  time_t start_time;
  time_t previous_time;
  const unsigned int cluster_size=params->blocksize;
  const unsigned int read_size=(cluster_size>65536?cluster_size:65536);
  alloc_data_t *current_search_space;
  disk_t *disk=params->disk;
  const partition_t *partition=params->partition;
  const unsigned long int no_of_cluster=(partition->part_size - start_data) / cluster_size;
  log_info("fat_unformat_aux: no_of_cluster=%lu\n", no_of_cluster);

#ifdef HAVE_NCURSES
  aff_copy(stdscr);
#endif
  start_time=time(NULL);
  previous_time=start_time;
  current_search_space=td_list_last_entry(&list_search_space->list, alloc_data_t, list);
  if(current_search_space==list_search_space)
  {
    return PSTATUS_OK;
  }
  offset_end=current_search_space->end;
  current_search_space=td_list_first_entry(&list_search_space->list, alloc_data_t, list);
  offset=set_search_start(params, &current_search_space, list_search_space);
  if(options->verbose>0)
    info_list_search_space(list_search_space, current_search_space, disk->sector_size, 0, options->verbose);
  buffer_start=(unsigned char *)MALLOC(READ_SIZE);
  buffer=buffer_start;
  disk->pread(disk, buffer_start, READ_SIZE, offset);
  for(;offset < offset_end; offset+=cluster_size)
  {
    if(buffer[0]=='.' && is_fat_directory(buffer))
    {
      file_info_t dir_list;
      TD_INIT_LIST_HEAD(&dir_list.list);
      dir_fat_aux(buffer, read_size, 0, &dir_list);
      if(!td_list_empty(&dir_list.list))
      {
	struct td_list_head *file_walker = NULL;
	unsigned int dir_inode=0;
	unsigned int nbr;
	int stop=0;
	log_info("Sector %llu\n", (long long unsigned)offset/disk->sector_size);
	dir_aff_log(NULL, &dir_list);
	del_search_space(list_search_space, offset, offset + cluster_size -1);
	for(file_walker=dir_list.list.next, nbr=0;
	    stop==0 && file_walker!=&dir_list.list;
	    file_walker=file_walker->next, nbr++)
	{
	  const file_info_t *current_file=td_list_entry_const(file_walker, const file_info_t, list);
	  if(current_file->st_ino==1 ||
	      current_file->st_ino >= no_of_cluster+2)
	    stop=1;
	  else if(LINUX_S_ISDIR(current_file->st_mode)!=0)
	  {
	    if(strcmp(current_file->name,"..")==0)
	    {
	      if(nbr!=1)
		stop=1;
	    }
	    else if(current_file->st_ino==0)
	      stop=1;
	    else if(strcmp(current_file->name,".")==0)
	    {
	      if(nbr==0)
		dir_inode=current_file->st_ino;
	      else
		stop=1;
	    }
	    else
	    {
#ifdef HAVE_MKDIR
	      char *new_file=(char *)MALLOC(1024);
	      snprintf(new_file, 1024, "%s.%u/inode_%u/inode_%u_%s",
		  params->recup_dir, params->dir_num, dir_inode,
		  (unsigned int)current_file->st_ino, current_file->name);
#ifdef __MINGW32__
	      mkdir(new_file);
#else
	      mkdir(new_file, 0775);
#endif
	      free(new_file);
#endif
	    }
	  }
	  else if(LINUX_S_ISREG(current_file->st_mode)!=0)
	  {
	    const uint64_t file_start=start_data + (uint64_t)(current_file->st_ino - 2) * cluster_size;
	    const uint64_t file_end=file_start+(current_file->st_size+cluster_size-1)/cluster_size*cluster_size - 1;
	    if(file_end < partition->part_offset + partition->part_size && current_file->st_ino>0)
	    {
	      if(fat_copy_file(disk, partition, cluster_size, start_data, params->recup_dir, params->dir_num, dir_inode, current_file)==0)
	      {
		params->file_nbr++;
		del_search_space(list_search_space, file_start, file_end);
	      }
	    }
	    else
	      stop=1;
	  }
	}
	delete_list_file(&dir_list);
      }
    }
    buffer+=cluster_size;
    if(buffer+read_size>buffer_start+READ_SIZE)
    {
      buffer=buffer_start;
      if(options->verbose>1)
      {
        log_verbose("Reading sector %10llu/%llu\n",
	    (unsigned long long)((offset-partition->part_offset)/disk->sector_size),
	    (unsigned long long)((partition->part_size-1)/disk->sector_size));
      }
      if(disk->pread(disk, buffer_start, READ_SIZE, offset) != READ_SIZE)
      {
#ifdef HAVE_NCURSES
	wmove(stdscr,11,0);
	wclrtoeol(stdscr);
	wprintw(stdscr,"Error reading sector %10lu\n",
	    (unsigned long)((offset-partition->part_offset)/disk->sector_size));
#endif
      }
#ifdef HAVE_NCURSES
      {
        time_t current_time;
        current_time=time(NULL);
        if(current_time>previous_time)
        {
          previous_time=current_time;
	  wmove(stdscr,9,0);
	  wclrtoeol(stdscr);
	  wprintw(stdscr,"Reading sector %10llu/%llu, %u files found\n",
	      (unsigned long long)((offset-partition->part_offset)/disk->sector_size),
	      (unsigned long long)(partition->part_size/disk->sector_size), params->file_nbr);
	  wmove(stdscr,10,0);
	  wclrtoeol(stdscr);
	  if(current_time > params->real_start_time)
	  {
	    const time_t elapsed_time=current_time - params->real_start_time;
	    wprintw(stdscr,"Elapsed time %uh%02um%02us",
		(unsigned)(elapsed_time/60/60),
		(unsigned)((elapsed_time/60)%60),
		(unsigned)(elapsed_time%60));
	    if(offset > partition->part_offset)
	    {
	      const time_t eta=(partition->part_offset+partition->part_size-1-offset)*elapsed_time/(offset-partition->part_offset);
	      wprintw(stdscr," - Estimated time to completion %uh%02um%02u\n",
		  (unsigned)(eta/3600),
		  (unsigned)((eta/60)%60),
		  (unsigned)(eta%60));
	    }
	  }
	  wrefresh(stdscr);
	  if(check_enter_key_or_s(stdscr))
	  {
	    log_info("PhotoRec has been stopped\n");
	    params->offset=offset;
	    offset = offset_end;
	    ind_stop=PSTATUS_STOP;
	  }
	}
      }
#endif
      if(need_to_stop!=0)
      {
	log_info("PhotoRec has been stopped\n");
	params->offset=offset;
	offset = offset_end;
	ind_stop=PSTATUS_STOP;
      }
    }
  }
  free(buffer_start);
  return ind_stop;
}

/* fat_unformat()
 * @param struct ph_param *params
 * @param const struct ph_options *options
 * @param alloc_data_t *list_search_space
 *
 * @returns:
 * 0: Completed or not possible
 * 1: Stop by user request
 *    params->offset is set
 */
pstatus_t fat_unformat(struct ph_param *params, const struct ph_options *options, alloc_data_t *list_search_space)
{
  unsigned int sectors_per_cluster=0;
  uint64_t start_data=0;
  params->blocksize=0;
  if(pfind_sectors_per_cluster(params->disk, params->partition, options->verbose, &sectors_per_cluster, &start_data, list_search_space)==0)
  {
    display_message("Can't find FAT cluster size\n");
    return PSTATUS_OK;
  }
  if(start_data <= params->partition->part_offset)
  {
    display_message("FAT filesystem was beginning before the actual partition.");
    return PSTATUS_OK;
  }
  start_data *= params->disk->sector_size;
  del_search_space(list_search_space, params->partition->part_offset, start_data - 1);
  {
    uint64_t offset=start_data;
    params->blocksize=sectors_per_cluster * params->disk->sector_size;
#ifdef HAVE_NCURSES
    if(options->expert>0)
      menu_choose_blocksize(&params->blocksize, &offset, params->disk->sector_size);
#endif
    update_blocksize(params->blocksize, list_search_space, offset);
  }
  /* start_data is relative to the disk */
  return fat_unformat_aux(params, options, start_data, list_search_space);
}

/* Directory-tree recovery for FAT with an optional path filter.
 *
 * When dir_filter is NULL or empty, falls back to the whole-volume
 * fat_unformat(). When a path (Windows style, e.g. "\\Documents") is given,
 * only the files stored inside that directory are recovered by following the
 * FAT cluster chain. Every recovered cluster is removed from
 * list_search_space so the subsequent carve pass skips it.
 */

#define KF_FAT_MAX_DIR_CLUSTERS (1024u * 1024u)
#define KF_FAT_MAX_DIR_DEPTH    64u

static int kf_fat_is_eoc(const upart_type_t upart_type, unsigned int cluster)
{
  switch(upart_type)
  {
    case UP_FAT12:
      return cluster >= 0x0FF8;
    case UP_FAT16:
      return cluster >= 0xFFF8;
    case UP_FAT32:
      return cluster >= 0x0FFFFFF8;
    default:
      return 1;
  }
}

/* Read the whole directory content following the FAT cluster chain.
 * Recovered directory clusters are removed from list_search_space.
 * Returns a malloc'd buffer (or NULL) and sets *dir_size. */
static unsigned char *kf_fat_read_dir_chain(disk_t *disk, const partition_t *partition,
    const upart_type_t upart_type, const int fat_offset, const uint64_t start_data,
    const unsigned int cluster_size, unsigned int cluster,
    alloc_data_t *list_search_space, uint64_t *dir_size)
{
  unsigned char *buffer;
  const uint64_t no_of_cluster=(partition->part_size - start_data) / cluster_size;
  uint64_t nbr_clusters=0;
  unsigned int next_cluster=cluster;
  while(nbr_clusters < KF_FAT_MAX_DIR_CLUSTERS &&
      next_cluster>=2 && next_cluster <= no_of_cluster+2 &&
      !kf_fat_is_eoc(upart_type, next_cluster))
  {
    nbr_clusters++;
    next_cluster=get_next_cluster(disk, partition, upart_type, fat_offset, next_cluster);
    if(next_cluster==0)
      break;
  }
  if(nbr_clusters==0)
    return NULL;
  buffer=(unsigned char *)MALLOC(nbr_clusters * cluster_size);
  next_cluster=cluster;
  nbr_clusters=0;
  while(next_cluster>=2 && next_cluster <= no_of_cluster+2 &&
      !kf_fat_is_eoc(upart_type, next_cluster))
  {
    const uint64_t start=start_data + (uint64_t)(next_cluster-2) * cluster_size;
    if((unsigned)disk->pread(disk, buffer + nbr_clusters*cluster_size, cluster_size, start)!=cluster_size)
      break;
    if(list_search_space!=NULL)
      del_search_space(list_search_space, start, start + cluster_size - 1);
    nbr_clusters++;
    next_cluster=get_next_cluster(disk, partition, upart_type, fat_offset, next_cluster);
    if(next_cluster==0)
      break;
  }
  *dir_size=nbr_clusters * cluster_size;
  if(nbr_clusters==0)
  {
    free(buffer);
    return NULL;
  }
  return buffer;
}

/* FAT12/16 root directory: a fixed contiguous region before the data area. */
static unsigned char *kf_fat_read_root_fixed(disk_t *disk, const partition_t *partition,
    const struct fat_boot_sector *bs, const unsigned int sector_size, uint64_t *dir_size)
{
  const uint32_t fat_length=le16(bs->fat_length)>0?le16(bs->fat_length):le32(bs->fat32_length);
  const unsigned int root_dir_sectors=((get_dir_entries(bs)*32) + (sector_size-1))/sector_size;
  const uint64_t start=partition->part_offset +
      ((uint64_t)le16(bs->reserved) + bs->fats * fat_length) * sector_size;
  const uint64_t root_size=(uint64_t)root_dir_sectors * sector_size;
  unsigned char *buffer=(unsigned char *)MALLOC(root_size);
  *dir_size=root_size;
  if((unsigned)disk->pread(disk, buffer, root_dir_sectors * sector_size, start)!=
      root_dir_sectors * sector_size)
  {
    free(buffer);
    return NULL;
  }
  return buffer;
}

/* Normalize a Windows style path into a '/' separated one (no leading '/'). */
static void kf_normalize_path(const char *src, char *dst, size_t dst_len)
{
  size_t i=0, j=0;
  while(src[i]!='\0' && (src[i]=='\\' || src[i]=='/'))
    i++;
  for(; src[i]!='\0' && j+1<dst_len; i++)
  {
    if(src[i]=='\\')
      dst[j]='/';
    else
      dst[j]=src[i];
    j++;
  }
  while(j>0 && dst[j-1]=='/')
    j--;
  dst[j]='\0';
}

/* Is parent an ancestor of child (or equal), comparing whole components? */
static int kf_path_is_within(const char *parent, const char *child)
{
  size_t plen;
  if(parent[0]=='\0')
    return 1;
  plen=strlen(parent);
  if(strncasecmp(child, parent, plen)!=0)
    return 0;
  if(child[plen]=='\0' || child[plen]=='/')
    return 1;
  return 0;
}

/* Are two paths related (one is an ancestor of the other)? */
static int kf_paths_related(const char *a, const char *b)
{
  return kf_path_is_within(a, b) || kf_path_is_within(b, a);
}

/* Sanitize a single path component for the output filesystem. */
static void kf_sanitize_name(char *name)
{
  char *p;
  for(p=name; *p!='\0'; p++)
    if(*p=='/' || *p=='\\' || *p==':' || (unsigned char)*p < 0x20)
      *p='_';
}

static void kf_mkdir_p(const char *path)
{
  char tmp[4096];
  size_t i, len;
  snprintf(tmp, sizeof(tmp), "%s", path);
  len=strlen(tmp);
  if(len==0 || len>=sizeof(tmp))
    return;
  for(i=1; i<len; i++)
  {
    if(tmp[i]=='/')
    {
      tmp[i]='\0';
#ifdef HAVE_MKDIR
#ifdef __MINGW32__
      mkdir(tmp);
#else
      (void)mkdir(tmp, 0775);
#endif
#endif
      tmp[i]='/';
    }
  }
#ifdef HAVE_MKDIR
#ifdef __MINGW32__
  mkdir(tmp);
#else
  (void)mkdir(tmp, 0775);
#endif
#endif
}

static pstatus_t kf_fat_recover_file(struct ph_param *params, const struct ph_options *options,
    alloc_data_t *list_search_space, disk_t *disk, const partition_t *partition,
    const upart_type_t upart_type, const int fat_offset, const uint64_t start_data,
    const unsigned int cluster_size, const char *dir_path, const file_info_t *file)
{
  const uint64_t no_of_cluster=(partition->part_size - start_data) / cluster_size;
  char out_base[4096];
  char out_file[4608];
  char name[256];
  unsigned int cluster=file->st_ino;
  unsigned int file_size=file->st_size;
  unsigned char *buffer_file=(unsigned char *)MALLOC(cluster_size);
  FILE *f_out;
  snprintf(name, sizeof(name), "%s", file->name);
  kf_sanitize_name(name);
  strip_fn(name);
  if(dir_path[0]=='\0')
    snprintf(out_base, sizeof(out_base), "%s.%u", params->recup_dir, params->dir_num);
  else
    snprintf(out_base, sizeof(out_base), "%s.%u/%s", params->recup_dir, params->dir_num, dir_path);
  kf_mkdir_p(out_base);
  snprintf(out_file, sizeof(out_file), "%s/%s", out_base, name);
  if((f_out=fopen(out_file, "rb"))!=NULL)
  {
    fclose(f_out);
    snprintf(out_file, sizeof(out_file), "%s/f%07u-%s", out_base,
        (unsigned int)((start_data - partition->part_offset + (uint64_t)(cluster-2)*cluster_size)/disk->sector_size),
        name);
  }
  log_info("fat_unformat: recovering %s (%u bytes)\n", out_file, file_size);
  f_out=fopen(out_file, "wb");
  if(!f_out)
  {
    log_critical("fat_unformat: Can't create file %s\n", out_file);
    free(buffer_file);
    return PSTATUS_EACCES;
  }
  while(cluster>=2 && cluster<=no_of_cluster+2 && file_size>0)
  {
    const uint64_t start=start_data + (uint64_t)(cluster-2) * cluster_size;
    const unsigned int toread=(file_size<cluster_size?file_size:cluster_size);
    if((unsigned)disk->pread(disk, buffer_file, toread, start)!=toread)
    {
      log_error("fat_unformat: Can't read cluster %u.\n", cluster);
    }
    if(fwrite(buffer_file, 1, toread, f_out)!=toread)
    {
      log_error("fat_unformat: no space left on destination.\n");
      fclose(f_out);
      set_date(out_file, file->td_atime, file->td_mtime);
      free(buffer_file);
      return PSTATUS_ENOSPC;
    }
    del_search_space(list_search_space, start, start + cluster_size - 1);
    file_size-=toread;
    cluster=get_next_cluster(disk, partition, upart_type, fat_offset, cluster);
    if(cluster==0)
      break;
  }
  fclose(f_out);
  set_date(out_file, file->td_atime, file->td_mtime);
  params->file_nbr++;
  free(buffer_file);
  return PSTATUS_OK;
}

static pstatus_t kf_fat_walk_dir(struct ph_param *params, const struct ph_options *options,
    alloc_data_t *list_search_space, disk_t *disk, const partition_t *partition,
    const upart_type_t upart_type, const int fat_offset, const uint64_t start_data,
    const unsigned int cluster_size, const char *dir_path, const char *filter,
    const unsigned char *dir_buffer, const uint64_t dir_size, const unsigned int depth)
{
  pstatus_t result=PSTATUS_OK;
  file_info_t dir_list;
  struct td_list_head *file_walker;
  TD_INIT_LIST_HEAD(&dir_list.list);
  if(depth > KF_FAT_MAX_DIR_DEPTH)
    return PSTATUS_OK;
  if(dir_size>0 && dir_fat_aux(dir_buffer, (unsigned int)dir_size, 0, &dir_list)==0)
  {
    for(file_walker=dir_list.list.next; file_walker!=&dir_list.list;
        file_walker=file_walker->next)
    {
      const file_info_t *current_file=td_list_entry_const(file_walker, const file_info_t, list);
      if(need_to_stop!=0)
      {
        result=PSTATUS_STOP;
        break;
      }
      if(current_file->name==NULL || current_file->name[0]=='\0')
        continue;
      if(LINUX_S_ISDIR(current_file->st_mode)!=0)
      {
        char child_path[4096];
        uint64_t child_size=0;
        unsigned char *child;
        if(strcmp(current_file->name, ".")==0 || strcmp(current_file->name, "..")==0)
          continue;
        if(current_file->st_ino==0)
          continue;
        if(dir_path[0]=='\0')
          snprintf(child_path, sizeof(child_path), "%s", current_file->name);
        else
          snprintf(child_path, sizeof(child_path), "%s/%s", dir_path, current_file->name);
        if(kf_paths_related(child_path, filter))
        {
          child=kf_fat_read_dir_chain(disk, partition, upart_type, fat_offset,
              start_data, cluster_size, current_file->st_ino, list_search_space, &child_size);
          if(child!=NULL)
          {
            result=kf_fat_walk_dir(params, options, list_search_space, disk, partition,
                upart_type, fat_offset, start_data, cluster_size, child_path, filter,
                child, child_size, depth+1);
            free(child);
            if(result!=PSTATUS_OK)
              break;
          }
        }
      }
      else if(LINUX_S_ISREG(current_file->st_mode)!=0)
      {
        if(kf_path_is_within(filter, dir_path))
        {
          result=kf_fat_recover_file(params, options, list_search_space, disk, partition,
              upart_type, fat_offset, start_data, cluster_size, dir_path, current_file);
          if(result==PSTATUS_EACCES || result==PSTATUS_ENOSPC)
            break;
        }
      }
    }
  }
  delete_list_file(&dir_list);
  return result;
}

/* kf_fat_boot_geometry()
 * Parse the FAT boot sector at partition->part_offset and compute the geometry.
 * Returns 1 on success, 0 if the boot sector does not look like a supported FAT. */
static int kf_fat_boot_geometry(disk_t *disk, const partition_t *partition,
    unsigned char bs_buf[512], unsigned int *sector_size, uint32_t *fat_length,
    uint64_t *fat_sectors_cnt, uint64_t *root_dir_sectors, uint64_t *start_data,
    uint64_t *data_sectors, unsigned long long *no_of_cluster,
    upart_type_t *upart_type, int *fat_offset)
{
  const struct fat_boot_sector *bs=(const struct fat_boot_sector *)bs_buf;
  unsigned int spc;
  if((unsigned)disk->pread(disk, bs_buf, 512, partition->part_offset)!=512)
    return 0;
  *sector_size=fat_sector_size(bs);
  if(*sector_size==0 || *sector_size>4096 || *sector_size!=disk->sector_size)
    return 0;
  spc=bs->sectors_per_cluster;
  if(spc==0 || bs->fats==0)
    return 0;
  *fat_length=le16(bs->fat_length)>0?le16(bs->fat_length):le32(bs->fat32_length);
  if(*fat_length==0)
    return 0;
  *fat_sectors_cnt=fat_sectors(bs)>0?fat_sectors(bs):le32(bs->total_sect);
  if(*fat_sectors_cnt==0)
    return 0;
  *root_dir_sectors=((uint64_t)get_dir_entries(bs)*32 + *sector_size - 1) / *sector_size;
  *start_data=partition->part_offset +
      ((uint64_t)le16(bs->reserved) + bs->fats*(*fat_length) + *root_dir_sectors) * *sector_size;
  *data_sectors=*fat_sectors_cnt - le16(bs->reserved) - (uint64_t)bs->fats*(*fat_length) - *root_dir_sectors;
  *no_of_cluster=*data_sectors / spc;
  if(*no_of_cluster<2 || *start_data <= partition->part_offset ||
      *start_data >= partition->part_offset + partition->part_size)
    return 0;
  *fat_offset=le16(bs->reserved);
  if(memcmp(bs_buf+0x36, "FAT32", 4)==0 || memcmp(bs_buf+0x52, "FAT32", 4)==0)
    *upart_type=UP_FAT32;
  else if(memcmp(bs_buf+0x36, "FAT16", 4)==0 || memcmp(bs_buf+0x52, "FAT16", 4)==0)
    *upart_type=UP_FAT16;
  else if(memcmp(bs_buf+0x36, "FAT12", 4)==0 || memcmp(bs_buf+0x52, "FAT12", 4)==0)
    *upart_type=UP_FAT12;
  else if(*no_of_cluster<4085)
    *upart_type=UP_FAT12;
  else if(*no_of_cluster<65525)
    *upart_type=UP_FAT16;
  else
    *upart_type=UP_FAT32;
  return 1;
}

/* fat_unformat_dir()
 * @param struct ph_param *params
 * @param const struct ph_options *options
 * @param alloc_data_t *list_search_space
 * @param const char *dir_filter  Windows style path, e.g. "\\Documents"
 *
 * @returns:
 * 0: Completed or not possible
 * 1: Stop by user request
 */
pstatus_t fat_unformat_dir(struct ph_param *params, const struct ph_options *options,
    alloc_data_t *list_search_space, const char *dir_filter)
{
  uint64_t start_data=0;
  unsigned char bs_buf[512];
  unsigned int sector_size;
  uint32_t fat_length;
  uint64_t fat_sectors_cnt;
  uint64_t root_dir_sectors;
  uint64_t data_sectors;
  unsigned long long no_of_cluster;
  upart_type_t upart_type;
  int fat_offset;
  uint64_t root_size=0;
  unsigned char *root;
  char filter[4096];
  pstatus_t result;

  if(dir_filter==NULL || dir_filter[0]=='\0')
    dir_filter="";
  if(!kf_fat_boot_geometry(params->disk, params->partition, bs_buf, &sector_size,
      &fat_length, &fat_sectors_cnt, &root_dir_sectors, &start_data,
      &data_sectors, &no_of_cluster, &upart_type, &fat_offset))
  {
    if(dir_filter[0]=='\0')
      return fat_unformat(params, options, list_search_space);
    return PSTATUS_OK;
  }
  params->blocksize=bs_buf[13] * sector_size;
  del_search_space(list_search_space, params->partition->part_offset, start_data - 1);
  {
    uint64_t offset=start_data;
    update_blocksize(params->blocksize, list_search_space, offset);
  }
  kf_normalize_path(dir_filter, filter, sizeof(filter));

  if(upart_type==UP_FAT32)
  {
    const unsigned int root_cluster=le32(((const struct fat_boot_sector *)bs_buf)->root_cluster);
    if(root_cluster<2)
      return PSTATUS_OK;
    root=kf_fat_read_dir_chain(params->disk, params->partition, upart_type, fat_offset,
        start_data, params->blocksize, root_cluster, list_search_space, &root_size);
  }
  else
  {
    root=kf_fat_read_root_fixed(params->disk, params->partition,
        (const struct fat_boot_sector *)bs_buf, sector_size, &root_size);
  }
  if(root==NULL)
  {
    return PSTATUS_OK;
  }
  result=kf_fat_walk_dir(params, options, list_search_space, params->disk, params->partition,
      upart_type, fat_offset, start_data, params->blocksize, "", filter, root, root_size, 0);
  free(root);
  return result;
}
#endif
