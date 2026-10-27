// SPDX-License-Identifier: GPL-3.0-or-later
// Independent client using the original libwayland-client and generated C ABI.
#define _GNU_SOURCE
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

struct window {
    struct wl_surface *surface;
    struct xdg_surface *xdg;
    struct xdg_toplevel *top;
    struct wl_buffer *buffer;
    int width,height,frames;
    uint32_t color;
};
static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct xdg_wm_base *shell;
static int target_frames=1,closed=0,hold=0;
static void draw(struct window *window);
static void frame_done(void *data,struct wl_callback *callback,uint32_t time) {
    (void)time;struct window *w=data;wl_callback_destroy(callback);w->frames++;
    if(w->frames<target_frames)draw(w);
}
static const struct wl_callback_listener frame_listener={frame_done};
static int shared_fd(void) {
#ifdef __linux__
    return memfd_create("wayland-c-buffer",MFD_CLOEXEC|MFD_ALLOW_SEALING);
#else
    char path[]="/tmp/trames-wayland-XXXXXX";
    int fd=mkstemp(path);if(fd>=0){unlink(path);fcntl(fd,F_SETFD,FD_CLOEXEC);}return fd;
#endif
}
static void draw(struct window *w) {
    size_t size=(size_t)w->width*w->height*4;
    int fd=shared_fd();assert(fd>=0);
    assert(ftruncate(fd,(off_t)size)==0);
    uint32_t *pixels=mmap(NULL,size,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);assert(pixels!=MAP_FAILED);
    for(size_t i=0;i<size/4;i++)pixels[i]=w->color;
    struct wl_shm_pool *pool=wl_shm_create_pool(shm,fd,(int)size);
    struct wl_buffer *buffer=wl_shm_pool_create_buffer(pool,0,w->width,w->height,w->width*4,WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    if(w->buffer)wl_buffer_destroy(w->buffer);
    w->buffer=buffer;
    wl_surface_attach(w->surface,buffer,0,0);
    wl_surface_damage(w->surface,0,0,w->width,w->height);
    if(target_frames>1){struct wl_callback *callback=wl_surface_frame(w->surface);wl_callback_add_listener(callback,&frame_listener,w);}
    wl_surface_commit(w->surface);
    // The server must retain the inode even after all client file references close.
    munmap(pixels,size);close(fd);
}
static void configured(void *data,struct xdg_surface *surface,uint32_t serial) {
    xdg_surface_ack_configure(surface,serial);draw(data);
}
static const struct xdg_surface_listener surface_listener={configured};
static void top_configure(void *data,struct xdg_toplevel *top,int32_t width,int32_t height,struct wl_array *states) {
    (void)top;(void)states;struct window *w=data;if(width>0&&height>0){w->width=width;w->height=height;}
}
static void top_close(void *data,struct xdg_toplevel *top){(void)data;(void)top;closed=1;}
static const struct xdg_toplevel_listener top_listener={.configure=top_configure,.close=top_close};
static void ping(void *data,struct xdg_wm_base *base,uint32_t serial){(void)data;xdg_wm_base_pong(base,serial);}
static const struct xdg_wm_base_listener shell_listener={ping};
static void global(void *data,struct wl_registry *registry,uint32_t id,const char *interface,uint32_t version) {
    (void)data;
    if(!strcmp(interface,"wl_compositor"))compositor=wl_registry_bind(registry,id,&wl_compositor_interface,version<4?version:4);
    if(!strcmp(interface,"wl_shm"))shm=wl_registry_bind(registry,id,&wl_shm_interface,1);
    if(!strcmp(interface,"xdg_wm_base")){shell=wl_registry_bind(registry,id,&xdg_wm_base_interface,version<3?version:3);xdg_wm_base_add_listener(shell,&shell_listener,NULL);}
}
static void global_remove(void *data,struct wl_registry *registry,uint32_t id){(void)data;(void)registry;(void)id;}
static const struct wl_registry_listener registry_listener={global,global_remove};
int main(int argc,char **argv) {
    if(argc>1)target_frames=atoi(argv[1]);
    hold=argc>2&&!strcmp(argv[2],"hold");
    assert(target_frames>0&&target_frames<10000);
    display=wl_display_connect(NULL);assert(display);
    struct wl_registry *registry=wl_display_get_registry(display);wl_registry_add_listener(registry,&registry_listener,NULL);
    assert(wl_display_roundtrip(display)>=0);assert(compositor&&shm&&shell);
    struct window windows[2]={{.width=240,.height=160,.color=0xff224466},{.width=160,.height=120,.color=0xffaa6633}};
    for(int i=0;i<2;i++) {
        struct window *w=&windows[i];w->surface=wl_compositor_create_surface(compositor);
        w->xdg=xdg_wm_base_get_xdg_surface(shell,w->surface);xdg_surface_add_listener(w->xdg,&surface_listener,w);
        w->top=xdg_surface_get_toplevel(w->xdg);xdg_toplevel_add_listener(w->top,&top_listener,w);
        xdg_toplevel_set_title(w->top,i?"Wayland C window 2":"Wayland C window 1");xdg_toplevel_set_app_id(w->top,"trames-wayland-client-test");
        wl_surface_commit(w->surface);
    }
    assert(wl_display_roundtrip(display)>=0);assert(wl_display_roundtrip(display)>=0);
    while(!closed&&(hold||(target_frames>1&&(windows[0].frames<target_frames||windows[1].frames<target_frames))))assert(wl_display_dispatch(display)>=0);
    printf("wayland-client-ok windows=2 frames=%d sizes=%dx%d,%dx%d\n",target_frames,windows[0].width,windows[0].height,windows[1].width,windows[1].height);
    for(int i=0;i<2;i++){struct window *w=&windows[i];if(w->buffer)wl_buffer_destroy(w->buffer);xdg_toplevel_destroy(w->top);xdg_surface_destroy(w->xdg);wl_surface_destroy(w->surface);}
    xdg_wm_base_destroy(shell);wl_shm_destroy(shm);wl_compositor_destroy(compositor);wl_registry_destroy(registry);wl_display_disconnect(display);return 0;
}
