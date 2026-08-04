#include "paper_pro_epaper.h"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <string>
#include <unistd.h>

namespace {

void fill_square(unsigned char *pixels, int stride, int x, int y, int size,
                 unsigned char value) {
    for (int row = y; row < y + size; ++row) {
        unsigned char *target =
            pixels + size_t(row) * stride + size_t(x) * 4;
        for (int column = 0; column < size; ++column) {
            target[column * 4 + 0] = value;
            target[column * 4 + 1] = value;
            target[column * 4 + 2] = value;
            target[column * 4 + 3] = 0xff;
        }
    }
}

bool process_loaded(const char *name) {
    std::ifstream maps("/proc/self/maps");
    std::string line;
    while (std::getline(maps, line))
        if (line.find(name) != std::string::npos) return true;
    return false;
}

}  // namespace

int main() {
    PaperProEpaperFramebuffer framebuffer{};
    if (paper_pro_epaper_open(&framebuffer) != 0 || !framebuffer.pixels ||
        framebuffer.width <= 0 || framebuffer.height <= 0 ||
        framebuffer.stride < framebuffer.width * 4)
        return 2;

    const size_t byte_count = size_t(framebuffer.stride) * framebuffer.height;
    auto *saved = static_cast<unsigned char *>(std::malloc(byte_count));
    if (!saved) return 3;
    std::memcpy(saved, framebuffer.pixels, byte_count);

    constexpr int square_size = 64;
    constexpr int square_count = 20;
    constexpr useconds_t interval_us = 120'000;
    const int x = framebuffer.width / 4 - square_size / 2;
    const int first_y = framebuffer.height / 5 - square_size / 2;
    const int last_y = framebuffer.height * 4 / 5 - square_size / 2;

    std::printf("framebuffer=%dx%d\n", framebuffer.width, framebuffer.height);
    std::printf("partial_area_fraction=%.9f\n",
                double(square_size * square_size) /
                    double(framebuffer.width * framebuffer.height));
    std::printf("libquill_loaded=%d\n", process_loaded("libquill.so"));
    std::fflush(stdout);

    sleep(2);
    bool passed = true;
    for (int index = 0; index < square_count; ++index) {
        const int y = first_y +
                      (last_y - first_y) * index / (square_count - 1);
        fill_square(framebuffer.pixels, framebuffer.stride, x, y, square_size,
                    0);
        passed &= paper_pro_epaper_submit_update(
                      x, y, square_size, square_size, 0, 0, 0) == 1;
        paper_pro_epaper_run_pending_events();
        usleep(interval_us);
    }
    passed &= paper_pro_epaper_wait_until_update_queue_empty() == 1;
    sleep(1);

    std::memcpy(framebuffer.pixels, saved, byte_count);
    std::free(saved);
    passed &= paper_pro_epaper_submit_update(
                  0, 0, framebuffer.width, framebuffer.height, 1, 4, 1) == 1;
    passed &= paper_pro_epaper_wait_until_update_queue_empty() == 1;
    sleep(2);
    return passed ? 0 : 4;
}
