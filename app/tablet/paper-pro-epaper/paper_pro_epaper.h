#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    uint8_t *pixels;
    int32_t width;
    int32_t height;
    int32_t stride;
    int32_t format;
} PaperProEpaperFramebuffer;

int paper_pro_epaper_open(PaperProEpaperFramebuffer *framebuffer);

int paper_pro_epaper_submit_update(int32_t x, int32_t y, int32_t width,
                                   int32_t height, int32_t content_type,
                                   int32_t screen_mode, int32_t update_flags);

int paper_pro_epaper_wait_until_update_queue_empty(void);

void paper_pro_epaper_run_pending_events(void);

#ifdef __cplusplus
}
#endif
